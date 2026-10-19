//! `vhhelper core …`: the APK's only path to config mutations (core-2.0 hard cutover).
//!
//! The app never writes the canonical config, `/proc/vpnhide_ctl`, Zygisk `targets.txt` or
//! the Ports rules itself. It runs this helper under su; the helper speaks the typed
//! `core-rpc` contract to the root core over its UDS (root ↔ root, so the socket can stay
//! `0600` in a `0700` directory with no `untrusted_app` SELinux grant — nothing an
//! observing app could probe). Core persists, applies the native backend and Ports, and
//! journals the operation; the helper only relays.
//!
//! The config crosses this boundary as on-disk canonical JSON text — the format the app,
//! LSPosed and the core codec ([`vpnhide_core_daemon::config_file`]) already share — so
//! there is exactly one Rust codec and the Kotlin side keeps its tested parser.
//!
//! Every reply is one JSON object on stdout with exit code 0; `status` carries the result.

use std::io::{self, Read};
use std::time::{Duration, Instant};

use serde::Serialize;
use vpnhide_core_daemon::CoreClient;
use vpnhide_core_daemon::config_file::{self, ConfigLoad};
use vpnhide_core_rpc::{
    BaseRef, ConfigEdit, NativeBackendStatus, OperationId, OperationRecord, PRODUCTION_SOCKET_PATH,
    SubmitError,
};

/// Override for host tests / the dev sandbox core. Production uses the module's socket.
pub const SOCKET_ENV: &str = "VPNHIDE_CORE_SOCKET";
/// Bound on the canonical JSON read from stdin (mirrors the mutation lane's cap).
const MAX_CONFIG: u64 = 2 * 1024 * 1024;
/// How long to wait for an accepted operation to settle before reporting it running.
const SETTLE_WAIT: Duration = Duration::from_secs(90);
const SETTLE_POLL: Duration = Duration::from_millis(200);

/// Validate the public command shape before doing any IO.
pub fn valid_command(args: &[String]) -> bool {
    match args {
        [verb] => verb == "state",
        [verb, op] => verb == "operation" && parse_id(op).is_some(),
        [verb, op, base] => {
            matches!(verb.as_str(), "replace" | "repair")
                && parse_id(op).is_some()
                && parse_u64(base).is_some()
        }
        _ => false,
    }
}

pub fn run(args: &[String]) -> i32 {
    let reply = run_inner(args).unwrap_or_else(|e| Reply::status("unavailable", e));
    if let Ok(json) = serde_json::to_string(&reply) {
        println!("{json}");
    }
    0
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

fn run_inner(args: &[String]) -> Result<Reply, String> {
    #[cfg(target_os = "android")]
    if unsafe { libc::geteuid() } != 0 {
        return Err("not root".into());
    }
    // Read the request body before touching the socket: a stalled su stdin must not hold
    // a core connection open.
    let input = if args.first().map(String::as_str) == Some("replace") {
        Some(read_stdin()?)
    } else {
        None
    };
    let socket = std::env::var(SOCKET_ENV)
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| PRODUCTION_SOCKET_PATH.to_owned());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime.block_on(dispatch(&socket, args, input))
}

async fn dispatch(socket: &str, args: &[String], input: Option<String>) -> Result<Reply, String> {
    let client = CoreClient::connect(socket);
    match client.hello().await {
        Ok(_epoch) => {}
        Err(e) if e.contains("protocol skew") => return Ok(Reply::status("protocol_skew", e)),
        Err(e) => return Ok(Reply::status("unreachable", e)),
    }
    match args {
        [verb] if verb == "state" => Ok(Reply {
            status: "ok",
            state: Some(state(&client).await?),
            ..Reply::default()
        }),
        [verb, op] if verb == "operation" => {
            let op = parse_id(op).ok_or("bad op")?;
            let record = client.get_operation(OperationId(op)).await?;
            Ok(Reply {
                status: if record.is_some() { "ok" } else { "unknown_op" },
                op: Some(op),
                record,
                state: Some(state(&client).await?),
                ..Reply::default()
            })
        }
        [verb, op, base] => {
            let op = parse_id(op).ok_or("bad op")?;
            let base = BaseRef {
                revision: parse_u64(base).ok_or("bad base")?,
            };
            let edit = if verb == "replace" {
                match config_file::parse_status(input.as_deref().unwrap_or_default()) {
                    Ok(ConfigLoad::Present(config)) => ConfigEdit::Replace { config },
                    Ok(_) => return Ok(Reply::status("invalid_input", "config not version 1")),
                    Err(e) => return Ok(Reply::status("invalid_input", e.to_string())),
                }
            } else {
                ConfigEdit::RepairActivation
            };
            submit(&client, OperationId(op), base, edit).await
        }
        _ => Err("usage".into()),
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

fn read_stdin() -> Result<String, String> {
    let mut text = String::new();
    io::stdin()
        .take(MAX_CONFIG + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() as u64 > MAX_CONFIG {
        return Err("config too large".into());
    }
    Ok(text)
}

/// Operation ids are app-chosen, random and non-zero; keep them inside i64 for Kotlin.
fn parse_id(raw: &str) -> Option<u64> {
    parse_u64(raw).filter(|n| *n != 0)
}

fn parse_u64(raw: &str) -> Option<u64> {
    raw.parse::<u64>().ok().filter(|n| *n < i64::MAX as u64)
}

#[cfg(test)]
mod tests {
    use super::valid_command;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_owned()).collect()
    }

    #[test]
    fn only_known_shapes_are_admitted() {
        assert!(valid_command(&args(&["state"])));
        assert!(valid_command(&args(&["operation", "7"])));
        assert!(valid_command(&args(&["replace", "7", "0"])));
        assert!(valid_command(&args(&["repair", "7", "3"])));
        assert!(!valid_command(&args(&["replace", "0", "0"])));
        assert!(!valid_command(&args(&["replace", "x", "0"])));
        assert!(!valid_command(&args(&["activate", "native"])));
        assert!(!valid_command(&args(&["state", "extra"])));
        assert!(!valid_command(&args(&["operation"])));
    }
}
