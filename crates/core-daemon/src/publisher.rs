//! The core's published state, and the `state_since` long-poll over it.
//!
//! tarpc-free and host-tested: the epoch/revision adoption rule is exactly
//! the kind of thing that must not be checked only on a phone. A restart is a new `epoch`
//! whose `revision` starts over, so a client following a dead epoch is answered at once
//! rather than stalled on sequence numbers that are no longer comparable.

use tokio::sync::watch;
use vpnhide_core_rpc::{Published, STATE_HOLD, StateSnapshot};

/// Owns the current [`Published`] and wakes long polls when it changes.
pub struct Publisher {
    epoch: u64,
    tx: watch::Sender<Published>,
}

impl Publisher {
    /// Start an epoch at revision 1 with an initial snapshot.
    pub fn new(epoch: u64, initial: StateSnapshot) -> Self {
        let (tx, _rx) = watch::channel(Published {
            epoch,
            revision: 1,
            snapshot: initial,
        });
        Publisher { epoch, tx }
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn current(&self) -> Published {
        self.tx.borrow().clone()
    }

    /// Publish a new snapshot, bumping the revision. Republishing an identical snapshot
    /// still bumps — callers that want to avoid a spurious wake should compare first.
    pub fn publish(&self, snapshot: StateSnapshot) {
        self.tx.send_modify(|p| {
            p.revision += 1;
            p.snapshot = snapshot;
        });
    }

    /// Mutate the published snapshot in place, bumping the revision. Only touch the fields you
    /// own: this preserves everything else, so a config edit and an observation update never
    /// clobber each other's fields even if they interleave.
    pub fn update(&self, edit: impl FnOnce(&mut StateSnapshot)) {
        self.tx.send_modify(|p| {
            p.revision += 1;
            edit(&mut p.snapshot);
        });
    }

    /// Publish only when the owning source changed its fields. The closure must
    /// leave state unchanged when returning false (including stale-source rejection).
    pub fn update_if_changed(&self, edit: impl FnOnce(&mut StateSnapshot) -> bool) {
        self.tx.send_if_modified(|published| {
            if edit(&mut published.snapshot) {
                published.revision += 1;
                true
            } else {
                false
            }
        });
    }

    /// The first published state newer than `(epoch, revision)` — the [`VpnhideCore`]
    /// long-poll body. Returns immediately when the caller's epoch is not this run, or
    /// when a strictly newer revision already exists; otherwise holds until one arrives
    /// or [`STATE_HOLD`] elapses, then returns the current state unchanged.
    ///
    /// [`VpnhideCore`]: vpnhide_core_rpc::VpnhideCore
    pub async fn state_since(&self, epoch: u64, revision: u64) -> Published {
        let mut rx = self.tx.subscribe();
        let current = rx.borrow_and_update().clone();
        if current.epoch != epoch || current.revision > revision {
            return current;
        }
        let deadline = tokio::time::sleep(STATE_HOLD);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                _ = &mut deadline => return self.tx.borrow().clone(),
                changed = rx.changed() => {
                    // Sender dropped (shutting down): answer with the last state we hold.
                    if changed.is_err() {
                        return self.tx.borrow().clone();
                    }
                    let next = rx.borrow_and_update().clone();
                    if next.epoch != epoch || next.revision > revision {
                        return next;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(available: bool) -> StateSnapshot {
        StateSnapshot {
            config: available.then(vpnhide_core_rpc::CanonicalConfig::default),
            ..StateSnapshot::default()
        }
    }

    #[tokio::test]
    async fn newer_revision_returns_immediately() {
        let pub_ = Publisher::new(7, snap(false));
        pub_.publish(snap(true)); // revision 2
        let got = pub_.state_since(7, 1).await;
        assert_eq!(got.revision, 2);
        assert!(got.snapshot.config.is_some());
    }

    #[tokio::test]
    async fn a_dead_epoch_is_adopted_at_once_whatever_the_revision() {
        let pub_ = Publisher::new(9, snap(true));
        // Caller followed epoch 8 at a high revision; this run is 9 at revision 1.
        let got = pub_.state_since(8, u64::MAX).await;
        assert_eq!((got.epoch, got.revision), (9, 1));
    }

    #[tokio::test(start_paused = true)]
    async fn holds_then_wakes_on_a_newer_publish() {
        let pub_ = std::sync::Arc::new(Publisher::new(1, snap(false)));
        let caller = pub_.clone();
        let poll = tokio::spawn(async move { caller.state_since(1, 1).await });
        // Nothing newer yet: the call is parked. Advance past a beat and publish.
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        pub_.publish(snap(true));
        let got = poll.await.unwrap();
        assert_eq!(got.revision, 2);
    }

    #[tokio::test(start_paused = true)]
    async fn returns_current_unchanged_after_the_hold() {
        let pub_ = std::sync::Arc::new(Publisher::new(1, snap(false)));
        let caller = pub_.clone();
        let poll = tokio::spawn(async move { caller.state_since(1, 1).await });
        tokio::time::advance(STATE_HOLD + std::time::Duration::from_secs(1)).await;
        let got = poll.await.unwrap();
        assert_eq!(got.revision, 1); // unchanged; the caller asks again
    }
}
