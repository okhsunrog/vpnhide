//! The app's only route to root work: typed diagnostics probes and fixed root effects,
//! both run by core (core-daemon is the only root process; the APK runs no `su`).
//!
//! Requests cross the boundary as `core-rpc` JSON (`DiagnosticsRequest`, `RootEffect`);
//! every reply is one JSON object whose `status` carries the result, so the calls never
//! throw. Large probe output and artifacts land in the app's `vpnhide-core-export/` dir,
//! owned by the app; the report carries their paths.

use serde::Serialize;
use vpnhide_core_daemon::CoreClient;
use vpnhide_core_rpc::{
    DiagnosticsReport, DiagnosticsRequest, EffectError, OperationId, RootEffect,
};

/// Bound on request JSON from the app. Requests are a handful of enum tags.
const MAX_REQUEST: usize = 64 * 1024;

#[derive(Debug, Default, Serialize)]
struct Reply {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    report: Option<DiagnosticsReport>,
}

impl Reply {
    fn status(status: &'static str, detail: impl Into<String>) -> Self {
        Reply {
            status,
            detail: Some(detail.into()),
            report: None,
        }
    }
}

fn encode(reply: Reply) -> String {
    serde_json::to_string(&reply)
        .unwrap_or_else(|e| format!(r#"{{"status":"unavailable","detail":"{e}"}}"#))
}

fn decode<T: serde::de::DeserializeOwned>(json: &str) -> Result<T, Reply> {
    if json.len() > MAX_REQUEST {
        return Err(Reply::status("invalid_input", "request too large"));
    }
    serde_json::from_str(json).map_err(|e| Reply::status("invalid_input", e.to_string()))
}

async fn connect(socket: &str) -> Result<std::sync::Arc<CoreClient>, Reply> {
    let client = CoreClient::connect(socket);
    match client.hello().await {
        Ok(_) => Ok(client),
        Err(e) if e.contains("protocol skew") => Err(Reply::status("protocol_skew", e)),
        Err(e) => Err(Reply::status("unreachable", e)),
    }
}

/// Run the requested probes. `ok` carries `report`; anything else is a delivery status.
pub async fn diagnostics(socket: &str, request_json: &str) -> String {
    let run = async {
        let request: DiagnosticsRequest = decode(request_json)?;
        let client = connect(socket).await?;
        let report = client
            .collect_diagnostics(request)
            .await
            .map_err(|e| Reply::status("transport", e))?;
        Ok::<_, Reply>(Reply {
            status: "ok",
            detail: None,
            report: Some(report),
        })
    };
    encode(run.await.unwrap_or_else(|reply| reply))
}

/// Apply one root effect: `ok`, `busy`, `refused` (invalid), `failed`, or a delivery status
/// (`transport` means it may or may not have run).
pub async fn root_effect(socket: &str, op: u64, effect_json: &str) -> String {
    let run = async {
        if op == 0 || op >= i64::MAX as u64 {
            return Err(Reply::status("invalid_input", "bad op"));
        }
        let effect: RootEffect = decode(effect_json)?;
        let client = connect(socket).await?;
        let outcome = client
            .root_effect(OperationId(op), effect)
            .await
            .map_err(|e| Reply::status("transport", e))?;
        Ok(match outcome {
            Ok(()) => Reply {
                status: "ok",
                ..Reply::default()
            },
            Err(EffectError::Busy) => Reply::status("busy", "core is applying an operation"),
            Err(EffectError::Invalid(e)) => Reply::status("refused", e),
            Err(EffectError::Failed(e)) => Reply::status("failed", e),
        })
    };
    encode(run.await.unwrap_or_else(|reply| reply))
}
