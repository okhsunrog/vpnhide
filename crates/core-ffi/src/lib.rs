//! App-side FFI surface for the persistent root core (core-2.0 §9 / step 7).
//!
//! UniFFI + JSON boundary for domain payloads. Kotlin owns editor drafts and local
//! preferences; this crate is the typed client onto core for config, operations, and
//! root observations.

uniffi::setup_scaffolding!();

pub mod config_relay;
pub mod root_relay;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::Mutex;
use tokio::sync::watch;
use vpnhide_core_daemon::CoreClient;
use vpnhide_core_rpc::{
    BaseRef, ConfigEdit, OperationId, PRODUCTION_SOCKET_PATH, PROTOCOL_VERSION, StateSnapshot,
};

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
    /// Protocol skew from [`hello`](CoreClientHandle::hello).
    #[error("protocol: {0}")]
    Protocol(String),
}

/// Production UDS path (the app's device-encrypted data dir, user 0).
#[uniffi::export]
pub fn production_socket_path() -> String {
    PRODUCTION_SOCKET_PATH.to_owned()
}

/// Wire protocol the FFI client speaks (`core-rpc` PROTOCOL_VERSION).
#[uniffi::export]
pub fn protocol_version() -> u32 {
    PROTOCOL_VERSION
}

/// Size cap on the canonical config JSON the save path accepts.
#[uniffi::export]
pub fn max_config_bytes() -> u64 {
    config_relay::MAX_CONFIG
}

/// The core's published state for the save path: one JSON reply (`status` `ok`, or a
/// delivery status such as `unreachable` / `protocol_skew`). Never throws.
#[uniffi::export(async_runtime = "tokio")]
pub async fn config_state(socket_path: String) -> String {
    config_relay::relay(&socket_path, config_relay::Request::State).await
}

/// An operation receipt plus the current state (`ok` / `unknown_op` / delivery status).
#[uniffi::export(async_runtime = "tokio")]
pub async fn config_operation(socket_path: String, op: u64) -> String {
    config_relay::relay(&socket_path, config_relay::Request::Operation { op }).await
}

/// Submit `ConfigEdit::Replace` with the on-disk canonical JSON against `base_revision`,
/// wait for it to settle (bounded), and reply with the receipt and the resulting state.
#[uniffi::export(async_runtime = "tokio")]
pub async fn config_replace(
    socket_path: String,
    op: u64,
    base_revision: u64,
    config_json: String,
) -> String {
    let request = config_relay::Request::Replace {
        op,
        base: base_revision,
        config_json,
    };
    config_relay::relay(&socket_path, request).await
}

/// Submit `ConfigEdit::RepairActivation` (re-apply native + Ports for the stored config).
#[uniffi::export(async_runtime = "tokio")]
pub async fn config_repair(socket_path: String, op: u64, base_revision: u64) -> String {
    let request = config_relay::Request::Repair {
        op,
        base: base_revision,
    };
    config_relay::relay(&socket_path, request).await
}

/// Run fixed root probes in core. `request_json` is a `core-rpc` `DiagnosticsRequest`;
/// the reply is `{"status":"ok","report":…}` or a delivery status. Never throws.
#[uniffi::export(async_runtime = "tokio")]
pub async fn collect_diagnostics(socket_path: String, request_json: String) -> String {
    root_relay::diagnostics(&socket_path, &request_json).await
}

/// Apply a fixed root effect (`core-rpc` `RootEffect` JSON) in core. Never throws.
#[uniffi::export(async_runtime = "tokio")]
pub async fn root_effect(socket_path: String, op: u64, effect_json: String) -> String {
    root_relay::root_effect(&socket_path, op, &effect_json).await
}

/// Connect to the production socket path (convenience for the APK).
#[uniffi::export]
pub async fn connect_production() -> Arc<CoreClientHandle> {
    CoreClientHandle::connect(PRODUCTION_SOCKET_PATH.to_owned()).await
}

/// The app's handle onto the core: one cached, multiplexed tarpc connection plus a mirror
/// loop, wrapped so Kotlin can hold it as a normal object.
#[derive(uniffi::Object)]
pub struct CoreClientHandle {
    client: Arc<CoreClient>,
    /// The mirrored read-replica. `await_next_state_json` parks on this until it changes.
    replica: Mutex<watch::Receiver<StateSnapshot>>,
    /// Last epoch returned by a successful `hello`. On change, Kotlin must treat the
    /// replica as a fresh world (revisions restart) and re-query operation receipts.
    last_epoch: AtomicU64,
}

#[uniffi::export(async_runtime = "tokio")]
impl CoreClientHandle {
    /// Connect to the core's UDS socket and start mirroring its state.
    #[uniffi::constructor]
    pub async fn connect(socket_path: String) -> Arc<Self> {
        let client = CoreClient::connect_and_mirror(socket_path);
        let replica = Mutex::new(client.state());
        Arc::new(Self {
            client,
            replica,
            last_epoch: AtomicU64::new(0),
        })
    }

    /// Version handshake — call once before relying on the connection, and again after
    /// reconnect when the mirror may have crossed a core restart. Returns the core epoch.
    /// On epoch change vs [`last_hello_epoch`], callers must take a fresh snapshot and
    /// query operation receipts independently (they are not implied by state updates).
    pub async fn hello(&self) -> Result<u64, CoreFfiError> {
        let epoch = self.client.hello().await.map_err(|e| {
            if e.contains("protocol skew") {
                CoreFfiError::Protocol(e)
            } else {
                CoreFfiError::Transport(e)
            }
        })?;
        self.last_epoch.store(epoch, Ordering::SeqCst);
        Ok(epoch)
    }

    /// Epoch from the last successful [`hello`]. `0` if hello has not succeeded yet.
    pub fn last_hello_epoch(&self) -> u64 {
        self.last_epoch.load(Ordering::SeqCst)
    }

    /// Submit a typed config edit. `edit_json` is a `ConfigEdit` encoded as JSON.
    /// Returns `Result<Accepted, SubmitError>` encoded as JSON. `Err` is delivery only.
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
            .submit_edit(
                OperationId(op),
                BaseRef {
                    revision: base_revision,
                },
                edit,
            )
            .await
            .map_err(CoreFfiError::Transport)?;
        serde_json::to_string(&outcome).map_err(|e| CoreFfiError::Json(e.to_string()))
    }

    /// Query an operation receipt independently of the state mirror (§5 / step 7).
    /// Returns JSON `null` when the core has no record for `op`.
    pub async fn get_operation_json(&self, op: u64) -> Result<String, CoreFfiError> {
        let record = self
            .client
            .get_operation(OperationId(op))
            .await
            .map_err(CoreFfiError::Transport)?;
        serde_json::to_string(&record).map_err(|e| CoreFfiError::Json(e.to_string()))
    }

    /// The current mirrored snapshot, as JSON, without waiting. For the initial paint.
    pub async fn current_state_json(&self) -> Result<String, CoreFfiError> {
        let snapshot = self.replica.lock().await.borrow().clone();
        serde_json::to_string(&snapshot).map_err(|e| CoreFfiError::Json(e.to_string()))
    }

    /// Park until the mirrored snapshot changes, then return it as JSON.
    pub async fn await_next_state_json(&self) -> Result<String, CoreFfiError> {
        let mut rx = self.replica.lock().await;
        rx.changed()
            .await
            .map_err(|e| CoreFfiError::Transport(e.to_string()))?;
        let snapshot = rx.borrow_and_update().clone();
        serde_json::to_string(&snapshot).map_err(|e| CoreFfiError::Json(e.to_string()))
    }
}
