//! The app's config save path onto core (core-2.0 hard cutover).
//!
//! The APK never writes the canonical config, `/proc/vpnhide_ctl`, Zygisk `targets.txt` or
//! the Ports rules, and runs no root process: this in-process client speaks the typed
//! `core-rpc` contract to the root core over the app-owned UDS (see
//! `vpnhide_core_daemon::app_socket`). Core persists, applies the native backend and Ports,
//! and journals the operation.
//!
//! The config crosses the boundary as on-disk canonical JSON text — the format the app,
//! LSPosed and the core codec ([`vpnhide_core_daemon::config_file`]) already share — so
//! there is exactly one Rust codec and Kotlin keeps its tested parser. Every reply is one
//! JSON object; `status` carries the result, so the calls never throw.

use std::time::{Duration, Instant};

use serde::Serialize;
use vpnhide_core_daemon::CoreClient;
use vpnhide_core_daemon::config_file::{self, ConfigLoad};
use vpnhide_core_rpc::{
    BaseRef, ConfigEdit, NativeBackendStatus, OperationId, OperationRecord, SubmitError,
};

/// Bound on the canonical JSON accepted from the app (mirrors the coordinator's cap), well
/// inside the transport's `MAX_FRAME_BYTES`.
pub const MAX_CONFIG: u64 = 512 * 1024;
/// How long to wait for an accepted operation to settle before reporting it running.
const SETTLE_WAIT: Duration = Duration::from_secs(90);
const SETTLE_POLL: Duration = Duration::from_millis(200);

/// One request from the app. Mirrors the four exported calls in `lib.rs`.
pub enum Request {
    State,
    Operation {
        op: u64,
    },
    Replace {
        op: u64,
        base: u64,
        config_json: String,
    },
    Repair {
        op: u64,
        base: u64,
    },
}

/// Run one request against the core at `socket` and return the JSON reply. Never fails:
/// delivery problems are `unreachable` / `transport` / `unavailable` replies.
pub async fn relay(socket: &str, request: Request) -> String {
    let reply = match validate(&request) {
        Err(detail) => Reply::status("invalid_input", detail),
        Ok(()) => dispatch(socket, request)
            .await
            .unwrap_or_else(|e| Reply::status("unavailable", e)),
    };
    serde_json::to_string(&reply)
        .unwrap_or_else(|e| format!(r#"{{"status":"unavailable","detail":"{e}"}}"#))
}

fn validate(request: &Request) -> Result<(), &'static str> {
    let (op, base) = match request {
        Request::State => return Ok(()),
        Request::Operation { op } => (*op, 0),
        Request::Replace {
            op,
            base,
            config_json,
        } => {
            if config_json.len() as u64 > MAX_CONFIG {
                return Err("config too large");
            }
            (*op, *base)
        }
        Request::Repair { op, base } => (*op, *base),
    };
    if valid_id(op).is_none() {
        return Err("bad op");
    }
    if valid_u64(base).is_none() {
        return Err("bad base");
    }
    Ok(())
}

#[derive(Debug, Default, Serialize)]
pub struct Reply {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    op: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    record: Option<OperationRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<StateReply>,
}

impl Reply {
    fn status(status: &'static str, detail: impl Into<String>) -> Self {
        Reply {
            status,
            detail: Some(detail.into()),
            ..Reply::default()
        }
    }
}

/// The parts of the core's published state the app's coordinator consumes.
#[derive(Debug, Serialize)]
pub struct StateReply {
    epoch: u64,
    config_revision: u64,
    /// `readable` (config holds on-disk JSON) or `missing` (core holds no config).
    config_status: &'static str,
    config: Option<String>,
    operations_in_flight: Vec<u64>,
    backend: NativeBackendStatus,
}

async fn dispatch(socket: &str, request: Request) -> Result<Reply, String> {
    let client = CoreClient::connect(socket);
    match client.hello().await {
        Ok(_epoch) => {}
        Err(e) if e.contains("protocol skew") => return Ok(Reply::status("protocol_skew", e)),
        Err(e) => return Ok(Reply::status("unreachable", e)),
    }
    match request {
        Request::State => Ok(Reply {
            status: "ok",
            state: Some(state(&client).await?),
            ..Reply::default()
        }),
        Request::Operation { op } => {
            let record = client.get_operation(OperationId(op)).await?;
            Ok(Reply {
                status: if record.is_some() { "ok" } else { "unknown_op" },
                op: Some(op),
                record,
                state: Some(state(&client).await?),
                ..Reply::default()
            })
        }
        Request::Replace {
            op,
            base,
            config_json,
        } => {
            let config = match config_file::parse_status(&config_json) {
                Ok(ConfigLoad::Present(config)) => config,
                Ok(_) => return Ok(Reply::status("invalid_input", "config not version 1")),
                Err(e) => return Ok(Reply::status("invalid_input", e.to_string())),
            };
            let base = BaseRef { revision: base };
            submit(
                &client,
                OperationId(op),
                base,
                ConfigEdit::Replace { config },
            )
            .await
        }
        Request::Repair { op, base } => {
            let base = BaseRef { revision: base };
            submit(&client, OperationId(op), base, ConfigEdit::RepairActivation).await
        }
    }
}

async fn submit(
    client: &CoreClient,
    op: OperationId,
    base: BaseRef,
    edit: ConfigEdit,
) -> Result<Reply, String> {
    let refused = |status, detail: String| Reply {
        op: Some(op.0),
        ..Reply::status(status, detail)
    };
    match client.submit_edit(op, base, edit).await {
        // Not delivered: says nothing about whether core ran it. The app recovers by
        // querying the same op id, never by resubmitting a new one.
        Err(e) => return Ok(refused("transport", e)),
        Ok(Err(SubmitError::Conflict { field })) => return Ok(refused("conflict", field)),
        Ok(Err(SubmitError::IdReused)) => return Ok(refused("id_reused", String::new())),
        Ok(Err(SubmitError::Unavailable)) => {
            return Ok(refused("refused_unavailable", String::new()));
        }
        Ok(Err(SubmitError::Rejected { reason })) => return Ok(refused("rejected", reason)),
        Ok(Ok(_)) => {}
    }
    let deadline = Instant::now() + SETTLE_WAIT;
    let record = loop {
        let record = client.get_operation(op).await?;
        let settled = record.as_ref().is_some_and(|r| r.settled.is_some());
        if settled || Instant::now() >= deadline {
            break record;
        }
        tokio::time::sleep(SETTLE_POLL).await;
    };
    Ok(Reply {
        status: "accepted",
        op: Some(op.0),
        record,
        state: Some(state(client).await?),
        ..Reply::default()
    })
}

async fn state(client: &CoreClient) -> Result<StateReply, String> {
    let published = client.current().await?;
    let snapshot = published.snapshot;
    let (config_status, config) = match &snapshot.config {
        Some(config) => ("readable", Some(config_file::to_json_string(config))),
        None => ("missing", None),
    };
    Ok(StateReply {
        epoch: published.epoch,
        config_revision: snapshot.config_revision,
        config_status,
        config,
        operations_in_flight: snapshot
            .operations_in_flight
            .iter()
            .map(|op| op.0)
            .collect(),
        backend: snapshot.backend,
    })
}

/// Operation ids are app-chosen, random and non-zero; keep them inside i64 for Kotlin.
fn valid_id(raw: u64) -> Option<u64> {
    valid_u64(raw).filter(|n| *n != 0)
}

fn valid_u64(raw: u64) -> Option<u64> {
    Some(raw).filter(|n| *n < i64::MAX as u64)
}
