//! Narrow native client for host reset tools. Effects remain in the existing
//! authenticated RPC single writer; this command never runs an offline writer.
use std::{io, path::PathBuf};
use vpnhide_core_client::CoreClient;
use vpnhide_core_rpc::{CoreCommand, OperationId, PhaseOutcome};
use vpnhide_maintenance_model::{RESET_SCHEMA, ResetReply, ResetStatus};
enum Action {
    Allocate,
    Submit(OperationId),
    Query(OperationId),
}
fn parse(args: &[String]) -> io::Result<(Action, String, PathBuf)> {
    let invalid = || {
        io::Error::other(
            "usage: maintenance-reset allocate BOOT [socket] | submit|query OPERATION BOOT [socket]",
        )
    };
    let op = |s: &String| -> io::Result<OperationId> {
        let value = s.parse::<u64>().map_err(|_| invalid())?;
        if value == 0 {
            return Err(invalid());
        }
        Ok(OperationId(value))
    };
    let (action, boot, path) = match args {
        [a, boot] if a == "allocate" => (Action::Allocate, boot, None),
        [a, boot, p] if a == "allocate" => (Action::Allocate, boot, Some(p)),
        [a, id, boot] if a == "submit" => (Action::Submit(op(id)?), boot, None),
        [a, id, boot, p] if a == "submit" => (Action::Submit(op(id)?), boot, Some(p)),
        [a, id, boot] if a == "query" => (Action::Query(op(id)?), boot, None),
        [a, id, boot, p] if a == "query" => (Action::Query(op(id)?), boot, Some(p)),
        _ => return Err(invalid()),
    };
    vpnhide_maintenance_model::DeviceIdentity {
        serial: "native".into(),
        boot_id: boot.clone(),
    }
    .validate()
    .map_err(io::Error::other)?;
    let path = path.map(PathBuf::from).unwrap_or_else(|| {
        crate::app_socket::socket_path(std::path::Path::new(crate::service::APP_DATA_DIR))
    });
    if path.as_os_str().is_empty() {
        return Err(invalid());
    }
    Ok((action, boot.clone(), path))
}
pub async fn run(args: &[String]) -> io::Result<ResetReply> {
    let (action, expected_boot, path) = parse(args)?;
    let operation = match action {
        Action::Allocate => None,
        Action::Submit(op) | Action::Query(op) => Some(op.0),
    };
    let mut reply = ResetReply {
        schema: RESET_SCHEMA,
        boot_id: None,
        epoch: None,
        operation,
        status: ResetStatus::Unknown,
    };
    use std::io::Read;
    let mut boot = String::new();
    std::fs::File::open("/proc/sys/kernel/random/boot_id")?
        .take(64)
        .read_to_string(&mut boot)?;
    reply.boot_id = Some(boot.trim().to_string());
    if reply.boot_id.as_deref() != Some(expected_boot.as_str()) {
        reply.status = ResetStatus::Rejected;
        return Ok(reply); // no connection, allocation or dispatch on another boot
    }
    let client = CoreClient::connect_expected_server_uid(path, unsafe { libc::geteuid() });
    let Ok(epoch) = client.hello().await else {
        return Ok(reply);
    };
    reply.epoch = Some(epoch);
    match action {
        Action::Allocate => match client.allocate_operation_id().await {
            Ok(Ok(op)) => {
                reply.operation = Some(op.0);
                reply.status = ResetStatus::Allocated;
            }
            Ok(Err(_)) => reply.status = ResetStatus::Rejected,
            Err(_) => {}
        },
        Action::Submit(op) => match client.submit_command(op, CoreCommand::FullReset).await {
            Ok(Ok(accepted)) if accepted.op == op => reply.status = ResetStatus::Accepted,
            Ok(Err(_)) => reply.status = ResetStatus::Rejected,
            _ => {}
        },
        Action::Query(op) => match client.get_reset_operation(op).await {
            Ok(Some(record)) if record.op == op => {
                reply.status = match record.settled {
                    Some(PhaseOutcome::Confirmed) => ResetStatus::Confirmed,
                    Some(PhaseOutcome::FailedKnown) => ResetStatus::FailedKnown,
                    Some(PhaseOutcome::Unknown) => ResetStatus::Unknown,
                    _ => ResetStatus::Running,
                }
            }
            Ok(None) => reply.status = ResetStatus::NotFound,
            _ => {}
        },
    }
    Ok(reply)
}
