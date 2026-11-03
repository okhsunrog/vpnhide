//! One running snapshot and one bounded pending read. Waiting never consumes
//! a blocking thread or the config writer; cancellation cannot release an
//! already dispatched probe's permit.
use std::{sync::Arc, time::Instant};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use vpnhide_core_rpc::ROOT_SNAPSHOT_MAX_RUNTIME;

pub(super) struct RootProbeAdmission {
    active: Arc<Semaphore>,
    pending: Arc<Semaphore>,
}
impl Default for RootProbeAdmission {
    fn default() -> Self {
        Self {
            active: Arc::new(Semaphore::new(1)),
            pending: Arc::new(Semaphore::new(1)),
        }
    }
}
impl RootProbeAdmission {
    pub(super) async fn acquire(&self, deadline: Instant) -> Option<OwnedSemaphorePermit> {
        let left = deadline.checked_duration_since(Instant::now())?;
        if left.is_zero() {
            return None;
        }
        if let Ok(permit) = self.active.clone().try_acquire_owned() {
            return Some(permit);
        }
        let _pending = self.pending.clone().try_acquire_owned().ok()?;
        tokio::time::timeout(
            left.min(ROOT_SNAPSHOT_MAX_RUNTIME),
            self.active.clone().acquire_owned(),
        )
        .await
        .ok()?
        .ok()
    }
    #[cfg(test)]
    pub(super) fn available_permits(&self) -> usize {
        self.active.available_permits()
    }
    #[cfg(test)]
    pub(super) fn pending_permits(&self) -> usize {
        self.pending.available_permits()
    }
}
