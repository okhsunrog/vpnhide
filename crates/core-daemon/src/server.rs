//! The tarpc plumbing: the [`VpnhideCore`] service impl and the per-connection transport
//! (length-delimited framing + JSON, per core-rpc's codec decision).
//!
//! Kept apart from [`crate::listener`] so the accept-loop lifetime rules stay host-tested
//! while this side owns the tarpc/tokio-serde wiring.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use futures::prelude::*;
use tarpc::context::Context;
use tarpc::server::{BaseChannel, Channel};
use tokio::net::UnixStream;
use tokio_util::codec::length_delimited::LengthDelimitedCodec;
use tokio_util::sync::CancellationToken;
use vpnhide_core_rpc::{
    Accepted, BaseRef, CanonicalConfig, ClientHello, ConfigEdit, Handshake, OperationId,
    OperationRecord, Published, SubmitError, VpnhideCore, PROTOCOL_VERSION,
};

use crate::config_edit::apply_canonical_edit;
use crate::config_file;
use crate::operation::{self, RealPhaseRunner};
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
    /// Where the durable canonical config lives. `None` = no persistence (host tests); a
    /// path means every accepted change is written there before it is acknowledged (§4).
    config_path: Option<PathBuf>,
}

impl CoreState {
    /// A core with no durable file — the config lives only in memory. Used by host tests.
    pub fn new(publisher: Publisher) -> Arc<Self> {
        Arc::new(CoreState {
            publisher,
            config: Mutex::new(ConfigStore {
                config: None,
                config_revision: 0,
                operations: HashMap::new(),
            }),
            config_path: None,
        })
    }

    /// Load the durable config from `path` and own it going forward (§4 tier-1, restart
    /// recovery). A present config starts at revision 1 and is published into the replica at
    /// once; an absent/blank file leaves the core with no config (revision 0, absent ≠ empty).
    /// A corrupt file is an error the caller decides on — core does not silently start blank.
    pub fn load(publisher: Publisher, path: PathBuf) -> std::io::Result<Arc<Self>> {
        let (config, config_revision) = match config_file::read(&path)? {
            Some(config) => (Some(config), 1),
            None => (None, 0),
        };
        let state = Arc::new(CoreState {
            publisher,
            config: Mutex::new(ConfigStore {
                config: config.clone(),
                config_revision,
                operations: HashMap::new(),
            }),
            config_path: Some(path),
        });
        if let Some(config) = config {
            state.publisher.update(|snapshot| {
                snapshot.config = Some(config);
                snapshot.config_revision = config_revision;
            });
        }
        Ok(state)
    }

    /// True once a durable config is loaded/created (for a startup log).
    pub fn has_config(&self) -> bool {
        self.config.lock().expect("config mutex").config.is_some()
    }

    /// Admit one typed edit (§5) and run its phase plan. Coarse optimistic concurrency: the
    /// caller's `base` must match the current config revision, or the edit is a conflict.
    ///
    /// Admission failures (conflict, no config to edit, a reused id) are `Err(SubmitError)`.
    /// An **admitted** edit returns `Ok(Accepted)` and its execution outcome — including a
    /// phase failure — lives in the queryable [`OperationRecord`] (settled), the way the app
    /// accepts an operation and then reports how it settled. `Persist` is the durable commit,
    /// so only a confirmed `Persist` publishes the new config into the replica.
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
        // concurrent edits is a later refinement (§5).
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
        let changed = store.config.as_ref() != Some(&next);

        // Run the operation's phase plan. The runner performs the durable write (Persist),
        // so an accepted edit is on disk before the config is committed in memory. Blocking
        // IO under the lock is deliberate: it serializes writers and the file is tiny.
        let plan = operation::plan_for(false);
        let runner = RealPhaseRunner { config_path: self.config_path.clone() };
        let outcome = operation::run_plan(&plan, &next, changed, &runner);

        if outcome.persisted {
            store.config = Some(next.clone());
            store.config_revision += 1;
        }
        store.operations.insert(
            op,
            OperationRecord { op, phases: outcome.phases, settled: Some(outcome.settled) },
        );

        if outcome.persisted {
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
    use vpnhide_core_rpc::{ConfigToggle, PhaseOutcome, StateSnapshot};

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

    #[tokio::test]
    async fn accepted_edit_persists_and_is_recovered_on_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vpnhide_config.json");
        let ctx = tarpc::context::current;

        // First run: no file yet -> no config, revision 0. A Replace persists it.
        let state = CoreState::load(Publisher::new(1, StateSnapshot::default()), path.clone()).unwrap();
        assert!(!state.has_config());
        let server = CoreServer::new(state);
        let cfg = CanonicalConfig { debug: true, ..Default::default() };
        let replace = ConfigEdit::Replace { config: cfg.clone() };
        server.clone().submit_edit(ctx(), OperationId(1), BaseRef { revision: 0 }, replace).await.expect("accepted");
        assert!(path.exists(), "an accepted edit must be on disk before it is acknowledged");

        // Second run (a restart): a fresh core loads the same file and recovers the config
        // at revision 1, so a follow-up edit conflict-checks against the recovered revision.
        let restarted = CoreState::load(Publisher::new(2, StateSnapshot::default()), path.clone()).unwrap();
        assert!(restarted.has_config());
        let server2 = CoreServer::new(restarted);
        let published = server2.clone().state_since(ctx(), 0, 0).await;
        assert_eq!(published.snapshot.config, Some(cfg));
        assert_eq!(published.snapshot.config_revision, 1);
        // An edit against the recovered revision applies; against a stale one it conflicts.
        assert!(server2.clone().submit_edit(ctx(), OperationId(2), BaseRef { revision: 1 }, ConfigEdit::Toggle { field: ConfigToggle::AutoHideName, enabled: true }).await.is_ok());
    }
}
