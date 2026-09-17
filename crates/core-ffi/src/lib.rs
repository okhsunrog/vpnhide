//! THROWAWAY SPIKE (core-2.0): the app-side FFI surface.
//!
//! Goal: de-risk the Kotlin <-> core path *end to end* — async methods crossing JNI, a
//! long-poll subscription that maps onto a Kotlin `suspend` (the "suspend-long-poll"), and
//! a JSON-string boundary for the domain payloads so we don't mirror every wire type
//! through UniFFI just to try the plumbing.
//!
//! # Why a JSON boundary (not per-type UniFFI records)
//!
//! The domain vocabulary (`CanonicalConfig`, `ConfigEdit`, `StateSnapshot`) is large and
//! still moving. Re-declaring all of it as `#[derive(uniffi::Record)]` would double the
//! type surface and couple the app build to every core wire change. For the spike we hand
//! JSON strings across the boundary: Rust already round-trips these through `serde_json`
//! on the tarpc wire (see `core-rpc`), and Kotlin can `kotlinx.serialization`-decode the
//! *same* shapes. The FFI surface stays tiny (connect / hello / submit / state) and stable
//! while the domain churns. If the spike graduates, individual hot types can move to
//! typed records later.
//!
//! # Async runtime
//!
//! `#[uniffi::export(async_runtime = "tokio")]` drives every exported future on a Tokio
//! reactor (via `async_compat`'s shared runtime), so `tokio::net`/`tarpc` have a reactor.
//! The object is *constructed* in an async fn too, so the mirror task `CoreClient` spawns
//! at construction lands on that same runtime — no cross-runtime split.

uniffi::setup_scaffolding!();

use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::sync::watch;
use vpnhide_core_daemon::CoreClient;
use vpnhide_core_rpc::{BaseRef, ConfigEdit, OperationId, StateSnapshot};

/// Everything that can go wrong at the boundary, flattened to something UniFFI can carry to
/// Kotlin as a checked exception.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum CoreFfiError {
    /// The call could not be delivered (socket missing, connection reset, deadline). Says
    /// nothing about whether the core ran the edit — the mirror tells the truth next tick.
    #[error("transport: {0}")]
    Transport(String),
    /// A JSON payload from Kotlin did not decode into the expected wire type, or a wire
    /// value would not encode. A programming error in the caller, not a core failure.
    #[error("json: {0}")]
    Json(String),
}

/// The app's handle onto the core: one cached, multiplexed tarpc connection plus a mirror
/// loop, wrapped so Kotlin can hold it as a normal object.
#[derive(uniffi::Object)]
pub struct CoreClientHandle {
    client: Arc<CoreClient>,
    /// The mirrored read-replica. `await_next_state_json` parks on this until it changes,
    /// which is the whole point — a `suspend fun` that resumes when the core state moves.
    replica: Mutex<watch::Receiver<StateSnapshot>>,
}

#[uniffi::export(async_runtime = "tokio")]
impl CoreClientHandle {
    /// Connect to the core's UDS socket and start mirroring its state. Async so the mirror
    /// task spawns on the same Tokio runtime the exported methods run on.
    #[uniffi::constructor]
    pub async fn connect(socket_path: String) -> Arc<Self> {
        let client = CoreClient::connect_and_mirror(socket_path);
        let replica = Mutex::new(client.state());
        Arc::new(Self { client, replica })
    }

    /// Version handshake — call once before relying on the connection. Returns the core's
    /// epoch, or an error on protocol skew / transport failure.
    pub async fn hello(&self) -> Result<u64, CoreFfiError> {
        self.client.hello().await.map_err(CoreFfiError::Transport)
    }

    /// Submit a typed config edit. `op` is the client-chosen operation id; `base_revision`
    /// is the `config_revision` the edit was formed against (optimistic concurrency);
    /// `edit_json` is a `ConfigEdit` encoded as JSON.
    ///
    /// Returns the `Result<Accepted, SubmitError>` the core produced, itself encoded as
    /// JSON — so Kotlin decodes one shape and sees both the accept and the typed refusal.
    /// An `Err` here is only a *delivery* failure.
    pub async fn submit_edit_json(
        &self,
        op: u64,
        base_revision: u64,
        edit_json: String,
    ) -> Result<String, CoreFfiError> {
        let edit: ConfigEdit =
            serde_json::from_str(&edit_json).map_err(|e| CoreFfiError::Json(e.to_string()))?;
        let outcome = self
            .client
            .submit_edit(OperationId(op), BaseRef { revision: base_revision }, edit)
            .await
            .map_err(CoreFfiError::Transport)?;
        serde_json::to_string(&outcome).map_err(|e| CoreFfiError::Json(e.to_string()))
    }

    /// The current mirrored snapshot, as JSON, without waiting. For the initial paint.
    pub async fn current_state_json(&self) -> Result<String, CoreFfiError> {
        let snapshot = self.replica.lock().await.borrow().clone();
        serde_json::to_string(&snapshot).map_err(|e| CoreFfiError::Json(e.to_string()))
    }

    /// The suspend-long-poll: park until the mirrored snapshot changes, then return the new
    /// one as JSON. Kotlin calls this in a loop from a coroutine and feeds a `StateFlow` —
    /// no callbacks, no polling interval, the coroutine simply suspends until there is news.
    pub async fn await_next_state_json(&self) -> Result<String, CoreFfiError> {
        let mut rx = self.replica.lock().await;
        // `changed()` resolves on the next value after the last one this receiver observed;
        // an `Err` means the sender is gone (the mirror never ends, so treat as transport).
        rx.changed()
            .await
            .map_err(|e| CoreFfiError::Transport(e.to_string()))?;
        let snapshot = rx.borrow_and_update().clone();
        serde_json::to_string(&snapshot).map_err(|e| CoreFfiError::Json(e.to_string()))
    }
}
