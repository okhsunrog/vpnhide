//! App-side FFI surface for the persistent root core (core-2.0 §9 / step 7).
//!
//! UniFFI + JSON boundary for domain payloads. Kotlin owns editor drafts and local
//! preferences; this crate is the typed client onto core for config, operations, and
//! root observations. Every export is one request → one JSON reply that never throws;
//! the app passes its own socket path (`CorePaths.socket`), so nothing here hard-codes
//! the user-0 path, and there is no long-lived handle or mirror object to manage.

uniffi::setup_scaffolding!();

pub mod config_relay;
pub mod root_relay;

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

/// Core's full-system logcat recording. `request_json` is
/// `{"action":"start"|"stop"|"status"|"ack"|"discard","session":<id>}` (`session` unused
/// for status); the reply is `{"status":"ok","session":…|null}` (`null` for status with no
/// session and for discard), `busy` (with `running`),
/// `unknown_session`, `disabled`, `refused`, `failed`, or a delivery status. Never throws.
#[uniffi::export(async_runtime = "tokio")]
pub async fn logcat_recording(socket_path: String, request_json: String) -> String {
    root_relay::logcat_recording(&socket_path, &request_json).await
}
