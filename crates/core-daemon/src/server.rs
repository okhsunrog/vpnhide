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
    Accepted, CanonicalConfig, ClientHello, ConfigEdit, ConfigPhase, Handshake, OperationId,
    OperationRecord, PhaseOutcome, PhaseRecord, Published, SubmitError, VpnhideCore,
    PROTOCOL_VERSION,
};

use crate::publisher::Publisher;

/// This core build, reported in the handshake for diagnostics only.
pub const CORE_BUILD: &str = env!("CARGO_PKG_VERSION");

/// The core's owned state: the published-state store plus the durable operation records.
///
/// The operation side is a placeholder (a real submit would prepare against fresh config,
/// dispatch phases, and recover) — enough to exercise the wire and the ownership rule that
/// an accepted operation outlives the submitting connection.
pub struct CoreState {
    pub publisher: Publisher,
    operations: Mutex<HashMap<OperationId, OperationRecord>>,
}

impl CoreState {
    pub fn new(publisher: Publisher) -> Arc<Self> {
        Arc::new(CoreState {
            publisher,
            operations: Mutex::new(HashMap::new()),
        })
    }

    /// Placeholder mutation: record the operation as a settled single-phase write and add a
    /// revision so a watching client observes the change. Real edits (§5) prepare against
    /// fresh state, run typed phases, and can conflict — TODO when the domain lands.
    fn submit_edit(&self, op: OperationId, edit: &ConfigEdit) -> Result<Accepted, SubmitError> {
        let mut ops = self.operations.lock().expect("operations mutex");
        if ops.contains_key(&op) {
            // Re-submitting the same id is a query/dedup; a different payload would be
            // IdReused. The placeholder cannot compare payloads yet, so it dedups.
            return Ok(Accepted { op });
        }
        ops.insert(
            op,
            OperationRecord {
                op,
                phases: vec![PhaseRecord {
                    phase: ConfigPhase::Persist,
                    outcome: PhaseOutcome::Confirmed,
                }],
                settled: Some(PhaseOutcome::Confirmed),
            },
        );
        drop(ops);
        // Placeholder application: a Replace sets the config; any other edit just marks it
        // present so a long poll wakes. Real edit application (applyCanonicalEdit) lands when
        // the config coordinator moves into core (§5).
        let mut snapshot = self.publisher.current().snapshot;
        match edit {
            ConfigEdit::Replace { config } => snapshot.config = Some(config.clone()),
            _ => {
                snapshot.config.get_or_insert_with(CanonicalConfig::default);
            }
        }
        self.publisher.publish(snapshot);
        Ok(Accepted { op })
    }

    fn get_operation(&self, op: OperationId) -> Option<OperationRecord> {
        self.operations.lock().expect("operations mutex").get(&op).cloned()
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
        _base: vpnhide_core_rpc::BaseRef,
        edit: ConfigEdit,
    ) -> Result<Accepted, SubmitError> {
        // TODO(§5): check `_base` against the fresh revision for a conflict.
        self.state.submit_edit(op, &edit)
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
    async fn a_submitted_edit_becomes_a_queryable_settled_operation() {
        let server = server();
        let edit = ConfigEdit::Toggle { field: ConfigToggle::Debug, enabled: true };
        let accepted = server
            .clone()
            .submit_edit(tarpc::context::current(), OperationId(3), vpnhide_core_rpc::BaseRef { revision: 1 }, edit)
            .await
            .expect("accepted");
        assert_eq!(accepted.op, OperationId(3));

        let record = server
            .get_operation(tarpc::context::current(), OperationId(3))
            .await
            .expect("record exists");
        assert_eq!(record.settled, Some(PhaseOutcome::Confirmed));
    }
}
