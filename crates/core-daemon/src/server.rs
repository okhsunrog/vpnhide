//! The tarpc plumbing: the [`VpnhideCore`] service impl and the per-connection transport
//! (length-delimited framing + JSON, per core-rpc's codec decision).
//!
//! Kept apart from [`crate::listener`] so the accept-loop lifetime rules stay host-tested
//! while this side owns the tarpc/tokio-serde wiring.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures::prelude::*;
use tarpc::context::Context;
use tarpc::server::{BaseChannel, Channel};
use tokio::net::UnixStream;
use tokio_util::codec::length_delimited::LengthDelimitedCodec;
use tokio_util::sync::CancellationToken;
use vpnhide_core_rpc::{
    Accepted, BaseRef, CanonicalConfig, ClientHello, ConfigEdit, ConfigPhase, Handshake,
    OperationId, OperationRecord, PhaseOutcome, PhaseRecord, Published, SubmitError, VpnhideCore,
    PROTOCOL_VERSION,
};

use crate::config_edit::apply_canonical_edit;
use crate::publisher::Publisher;

/// This core build, reported in the handshake for diagnostics only.
pub const CORE_BUILD: &str = env!("CARGO_PKG_VERSION");

/// The authoritative config core owns, its revision, and the durable operation records.
///
/// This is core's in-memory canonical (§4 tier 1 — the durable file read/write lands in the
/// next step). `config_revision` bumps only on a real config change and is what an edit
/// conflict-checks against.
struct ConfigStore {
    config: Option<CanonicalConfig>,
    config_revision: u64,
    operations: HashMap<OperationId, OperationRecord>,
}

/// The core's owned state: the published-state store plus the authoritative config.
pub struct CoreState {
    pub publisher: Publisher,
    config: Mutex<ConfigStore>,
}

impl CoreState {
    pub fn new(publisher: Publisher) -> Arc<Self> {
        Arc::new(CoreState {
            publisher,
            config: Mutex::new(ConfigStore {
                config: None,
                config_revision: 0,
                operations: HashMap::new(),
            }),
        })
    }

    /// Apply one typed edit against fresh config (§5). Coarse optimistic concurrency: the
    /// caller's `base` must match the current config revision, or the edit is a conflict.
    /// Operation phases and durable persistence are the next steps; this records a settled
    /// single-phase op and publishes the new config into the replica.
    fn submit_edit(
        &self,
        op: OperationId,
        base: BaseRef,
        edit: &ConfigEdit,
    ) -> Result<Accepted, SubmitError> {
        let mut store = self.config.lock().expect("config mutex");
        // Re-submitting the same id queries/dedups (a different payload would be IdReused —
        // deferred until operations carry their payload).
        if store.operations.contains_key(&op) {
            return Ok(Accepted { op });
        }
        // Coarse conflict: whole-config optimistic lock. Field-level merge of unrelated
        // concurrent edits is a later refinement (the operation reducer, §5).
        if base.revision != store.config_revision {
            return Err(SubmitError::Conflict { field: "config".to_owned() });
        }
        // Apply against fresh config. With none yet, only a Replace can bootstrap one.
        let current = match &store.config {
            Some(config) => config.clone(),
            None => match edit {
                ConfigEdit::Replace { .. } => CanonicalConfig::default(),
                _ => return Err(SubmitError::Unavailable),
            },
        };
        let next = apply_canonical_edit(&current, edit);
        store.operations.insert(
            op,
            OperationRecord {
                op,
                phases: vec![PhaseRecord { phase: ConfigPhase::Persist, outcome: PhaseOutcome::Confirmed }],
                settled: Some(PhaseOutcome::Confirmed),
            },
        );
        // A no-op edit is still accepted, but must not bump the revision or wake watchers.
        if store.config.as_ref() != Some(&next) {
            store.config = Some(next.clone());
            store.config_revision += 1;
            let revision = store.config_revision;
            drop(store);
            self.publisher.update(|snapshot| {
                snapshot.config = Some(next);
                snapshot.config_revision = revision;
            });
        }
        Ok(Accepted { op })
    }

    fn get_operation(&self, op: OperationId) -> Option<OperationRecord> {
        self.config.lock().expect("config mutex").operations.get(&op).cloned()
    }
}

/// One connection's view of the core. Cloned per request by tarpc (cheap: an `Arc`).
#[derive(Clone)]
pub struct CoreServer {
    state: Arc<CoreState>,
}

impl CoreServer {
    pub fn new(state: Arc<CoreState>) -> Self {
        CoreServer { state }
    }
}

impl VpnhideCore for CoreServer {
    async fn hello(self, _ctx: Context, client: ClientHello) -> Handshake {
        if client.protocol == PROTOCOL_VERSION {
            Handshake::Compatible {
                core_protocol: PROTOCOL_VERSION,
                core_build: CORE_BUILD.to_owned(),
                epoch: self.state.publisher.epoch(),
            }
        } else {
            Handshake::Incompatible { core_protocol: PROTOCOL_VERSION }
        }
    }

    async fn state_since(self, _ctx: Context, epoch: u64, revision: u64) -> Published {
        self.state.publisher.state_since(epoch, revision).await
    }

    async fn submit_edit(
        self,
        _ctx: Context,
        op: OperationId,
        base: BaseRef,
        edit: ConfigEdit,
    ) -> Result<Accepted, SubmitError> {
        self.state.submit_edit(op, base, &edit)
    }

    async fn get_operation(self, _ctx: Context, op: OperationId) -> Option<OperationRecord> {
        self.state.get_operation(op)
    }
}

/// Serve one accepted connection until it closes or its generation is cancelled.
///
/// Framing + codec match core-rpc's contract (length-delimited + JSON). Spawned onto the
/// current runtime; ends when the peer disconnects or `cancel` fires, so a connection never
/// outlives the generation that accepted it.
pub fn serve_connection(stream: UnixStream, state: Arc<CoreState>, cancel: CancellationToken) {
    let framed = LengthDelimitedCodec::builder().new_framed(stream);
    let transport = tarpc::serde_transport::new(framed, tokio_serde::formats::Json::default());
    let server = CoreServer::new(state);
    tokio::spawn(async move {
        let serve = BaseChannel::with_defaults(transport)
            .execute(server.serve())
            .for_each(|response| async move {
                tokio::spawn(response);
            });
        tokio::select! {
            _ = cancel.cancelled() => {}
            _ = serve => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpnhide_core_rpc::{ConfigToggle, StateSnapshot};

    fn server() -> CoreServer {
        CoreServer::new(CoreState::new(Publisher::new(1, StateSnapshot::default())))
    }

    #[tokio::test]
    async fn hello_accepts_the_current_protocol_and_refuses_skew() {
        let hello = |protocol| ClientHello { protocol, build: "test".to_owned() };
        let ok = server()
            .hello(tarpc::context::current(), hello(PROTOCOL_VERSION))
            .await;
        assert!(matches!(ok, Handshake::Compatible { epoch: 1, .. }));

        let skewed = server()
            .hello(tarpc::context::current(), hello(PROTOCOL_VERSION + 1))
            .await;
        assert!(matches!(skewed, Handshake::Incompatible { .. }));
    }

    #[tokio::test]
    async fn submit_bootstraps_applies_conflicts_and_reports_unavailable() {
        let server = server();
        let ctx = tarpc::context::current;
        let toggle = |f| ConfigEdit::Toggle { field: f, enabled: true };

        // No config yet: a non-Replace edit is Unavailable.
        let unavailable = server
            .clone()
            .submit_edit(ctx(), OperationId(1), BaseRef { revision: 0 }, toggle(ConfigToggle::Debug))
            .await;
        assert!(matches!(unavailable, Err(SubmitError::Unavailable)));

        // Replace bootstraps the config against revision 0, and is a queryable settled op.
        let bootstrap = ConfigEdit::Replace { config: CanonicalConfig::default() };
        assert_eq!(
            server.clone().submit_edit(ctx(), OperationId(2), BaseRef { revision: 0 }, bootstrap).await.expect("accepted").op,
            OperationId(2),
        );
        let record = server.clone().get_operation(ctx(), OperationId(2)).await.expect("record");
        assert_eq!(record.settled, Some(PhaseOutcome::Confirmed));

        // A toggle now applies against fresh config; the revision advanced to 1.
        assert!(server.clone().submit_edit(ctx(), OperationId(3), BaseRef { revision: 1 }, toggle(ConfigToggle::Debug)).await.is_ok());

        // A stale base conflicts.
        let stale = server
            .submit_edit(ctx(), OperationId(4), BaseRef { revision: 0 }, toggle(ConfigToggle::AutoHideName))
            .await;
        assert!(matches!(stale, Err(SubmitError::Conflict { .. })));
    }
}
