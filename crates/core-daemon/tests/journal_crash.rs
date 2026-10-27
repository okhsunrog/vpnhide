//! Crash-window coverage for the durable journal: kill the core at phase
//! boundaries, restart, and assert intent/identity/uncertainty without effect
//! replay. Uses `VPNHIDE_TEST_CRASH_AFTER` (production never sets it).

mod common;
use std::fs;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use vpnhide_core_client::CoreClient;
use vpnhide_core_daemon::{config_file, journal};
use vpnhide_core_rpc::{
    BaseRef, CanonicalConfig, ConfigEdit, ConfigPhase, ConfigToggle, OperationId, PhaseOutcome,
    SubmitError,
};

#[tokio::test]
async fn completed_failure_with_skipped_ports_does_not_block_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("core.sock");
    let config_path = dir.path().join("config.json");
    let config = CanonicalConfig::default();
    config_file::write_atomic(&config_path, &config).unwrap();
    let mut stored = journal::Journal {
        config_revision: 1,
        ..Default::default()
    };
    stored.upsert(journal::JournalEntry {
        admission: None,
        command: None,
        superseded_by: None,
        superseded_phases: Vec::new(),
        finalized_unknown: false,
        base: BaseRef { revision: 0 },
        edit: ConfigEdit::Replace {
            config: config.clone(),
        },
        candidate: Some(config),
        native_intent: None,
        record: vpnhide_core_rpc::OperationRecord {
            op: OperationId(1),
            phases: vec![
                vpnhide_core_rpc::PhaseRecord {
                    phase: ConfigPhase::Persist,
                    outcome: PhaseOutcome::Confirmed,
                },
                vpnhide_core_rpc::PhaseRecord {
                    phase: ConfigPhase::Native,
                    outcome: PhaseOutcome::FailedKnown,
                },
                vpnhide_core_rpc::PhaseRecord {
                    phase: ConfigPhase::Ports,
                    outcome: PhaseOutcome::NotAttempted,
                },
            ],
            settled: Some(PhaseOutcome::FailedKnown),
            ports: None,
        },
    });
    journal::write_atomic(&config_path.with_file_name("core_operations.json"), &stored).unwrap();
    let mut child = spawn_core(&socket, &config_path, None, None);
    wait_bound(&socket, &mut child).await;
    let client = CoreClient::connect(&socket);
    common::reserve_operation_ids(&client, 8).await;
    client.hello().await.unwrap();
    let failed = client.get_operation(OperationId(1)).await.unwrap().unwrap();
    assert_eq!(failed.settled, Some(PhaseOutcome::FailedKnown));
    assert!(
        client
            .submit_edit(
                OperationId(2),
                BaseRef { revision: 1 },
                ConfigEdit::RepairActivation,
            )
            .await
            .unwrap()
            .is_ok(),
        "a historical known failure must not hold new operations"
    );
}

struct CoreProcess(Child);

impl Drop for CoreProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_core(
    socket: &std::path::Path,
    config_path: &std::path::Path,
    crash_after: Option<&str>,
    kmod_ctl: Option<&std::path::Path>,
) -> CoreProcess {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vpnhide-core"));
    cmd.arg(socket)
        .env("VPNHIDE_CONFIG_PATH", config_path)
        .env_remove("VPNHIDE_KPM_LOCK")
        .env_remove("VPNHIDE_KERNEL_MODULES")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match kmod_ctl {
        Some(ctl) => {
            cmd.env("VPNHIDE_KMOD_CTL", ctl);
        }
        None => {
            cmd.env_remove("VPNHIDE_KMOD_CTL");
        }
    }
    match crash_after {
        Some(point) => {
            cmd.env("VPNHIDE_TEST_CRASH_AFTER", point);
        }
        None => {
            cmd.env_remove("VPNHIDE_TEST_CRASH_AFTER");
        }
    }
    CoreProcess(cmd.spawn().unwrap())
}

async fn wait_bound(socket: &std::path::Path, child: &mut CoreProcess) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !socket.exists() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "core exited before binding"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("core bound its socket");
}

#[tokio::test]
async fn crash_after_persist_intent_reconciles_to_failed_without_replay() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("core.sock");
    let config_path = dir.path().join("config.json");
    // Pre-existing stale config (Persist intent recorded, write never happens).
    let stale = CanonicalConfig::default();
    config_file::write_atomic(&config_path, &stale).unwrap();

    let mut child = spawn_core(&socket, &config_path, Some("persist-intent"), None);
    wait_bound(&socket, &mut child).await;
    let client = CoreClient::connect(&socket);
    common::reserve_operation_ids(&client, 8).await;
    client.hello().await.unwrap();
    let submit = client
        .submit_edit(
            OperationId(1),
            BaseRef { revision: 1 },
            ConfigEdit::Toggle {
                field: ConfigToggle::Debug,
                enabled: true,
            },
        )
        .await;
    // Core crashes mid-RPC; connection error is expected.
    assert!(submit.is_err() || submit.unwrap().is_err());
    let _ = child.0.wait();

    let journal_path = config_path.with_file_name("core_operations.json");
    assert!(
        journal_path.exists(),
        "intent must be durable before Persist"
    );
    let on_disk = journal::read(&journal_path).unwrap();
    let entry = on_disk.get(OperationId(1)).expect("intent recorded");
    assert!(
        entry
            .record
            .phases
            .iter()
            .any(|p| p.phase == ConfigPhase::Persist
                && matches!(p.outcome, PhaseOutcome::Running | PhaseOutcome::Unknown)),
        "Persist must have been Running at the crash window"
    );

    // Restart without crash injection: fence + reconcile against stale file.
    let _ = fs::remove_file(&socket);
    let mut child = spawn_core(&socket, &config_path, None, None);
    wait_bound(&socket, &mut child).await;
    let client = CoreClient::connect(&socket);
    common::reserve_operation_ids(&client, 8).await;
    client.hello().await.unwrap();
    let record = client
        .get_operation(OperationId(1))
        .await
        .unwrap()
        .expect("operation identity preserved");
    assert_eq!(
        record.phases[0].outcome,
        PhaseOutcome::FailedKnown,
        "stale config proves Persist did not land"
    );
    assert_eq!(record.settled, Some(PhaseOutcome::FailedKnown));
    // Hold released — a new edit against the still-stale revision may proceed.
    assert!(
        client
            .submit_edit(
                OperationId(2),
                BaseRef { revision: 1 },
                ConfigEdit::Toggle {
                    field: ConfigToggle::Debug,
                    enabled: true,
                },
            )
            .await
            .unwrap()
            .is_ok()
    );
    // Config only matches the successful second edit.
    let loaded = config_file::read(&config_path).unwrap().unwrap();
    assert!(loaded.debug);
}

#[tokio::test]
async fn crash_after_persist_reconciles_confirmed_and_holds_unfinished_native() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("core.sock");
    let config_path = dir.path().join("config.json");
    let ctl = dir.path().join("ctl");
    // Live-shaped builtin node so Native is planned; delivery would rewrite it.
    fs::write(
        &ctl,
        "vpnhide 1 status\nbackend 0x4\nkver 0x0\nhooks 0x0\nerror 0x0\n",
    )
    .unwrap();
    let ctl_before = fs::read_to_string(&ctl).unwrap();

    let mut child = spawn_core(&socket, &config_path, Some("persist"), Some(&ctl));
    wait_bound(&socket, &mut child).await;
    let client = CoreClient::connect(&socket);
    common::reserve_operation_ids(&client, 8).await;
    client.hello().await.unwrap();
    let candidate = CanonicalConfig {
        debug: true,
        ..Default::default()
    };
    let submit = client
        .submit_edit(
            OperationId(7),
            BaseRef { revision: 0 },
            ConfigEdit::Replace {
                config: candidate.clone(),
            },
        )
        .await;
    assert!(submit.is_err() || submit.unwrap().is_err());
    let _ = child.0.wait();

    // Persist must have landed; Native must not have run (ctl unchanged).
    assert_eq!(
        config_file::read(&config_path).unwrap(),
        Some(candidate.clone())
    );
    assert_eq!(
        fs::read_to_string(&ctl).unwrap(),
        ctl_before,
        "Native must not have been dispatched after the Persist crash window"
    );

    let _ = fs::remove_file(&socket);
    let mut child = spawn_core(&socket, &config_path, None, Some(&ctl));
    wait_bound(&socket, &mut child).await;
    let client = CoreClient::connect(&socket);
    common::reserve_operation_ids(&client, 8).await;
    client.hello().await.unwrap();
    let record = client
        .get_operation(OperationId(7))
        .await
        .unwrap()
        .expect("identity");
    assert_eq!(
        record
            .phases
            .iter()
            .find(|p| p.phase == ConfigPhase::Persist)
            .unwrap()
            .outcome,
        PhaseOutcome::Confirmed,
        "config file matching candidate confirms Persist"
    );
    // Native was NotAttempted or Unknown — either way settled stays Unknown (Held).
    assert_eq!(record.settled, Some(PhaseOutcome::Unknown));
    let blocked = client
        .submit_edit(
            OperationId(8),
            BaseRef { revision: 1 },
            ConfigEdit::Toggle {
                field: ConfigToggle::AutoHideName,
                enabled: true,
            },
        )
        .await
        .unwrap();
    assert!(matches!(blocked, Err(SubmitError::Unavailable)));
    // Still no Native replay on the second boot.
    assert_eq!(fs::read_to_string(&ctl).unwrap(), ctl_before);
}

#[tokio::test]
async fn crash_after_native_intent_keeps_unknown_without_replay() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("core.sock");
    let config_path = dir.path().join("config.json");
    let ctl = dir.path().join("ctl");
    fs::write(
        &ctl,
        "vpnhide 1 status\nbackend 0x4\nkver 0x0\nhooks 0x0\nerror 0x0\n",
    )
    .unwrap();
    let ctl_before = fs::read_to_string(&ctl).unwrap();

    let mut child = spawn_core(&socket, &config_path, Some("native-intent"), Some(&ctl));
    wait_bound(&socket, &mut child).await;
    let client = CoreClient::connect(&socket);
    common::reserve_operation_ids(&client, 8).await;
    client.hello().await.unwrap();
    let submit = client
        .submit_edit(
            OperationId(3),
            BaseRef { revision: 0 },
            ConfigEdit::Replace {
                config: CanonicalConfig {
                    debug: true,
                    ..Default::default()
                },
            },
        )
        .await;
    assert!(submit.is_err() || submit.unwrap().is_err());
    let _ = child.0.wait();
    assert_eq!(fs::read_to_string(&ctl).unwrap(), ctl_before);

    let _ = fs::remove_file(&socket);
    let mut child = spawn_core(&socket, &config_path, None, Some(&ctl));
    wait_bound(&socket, &mut child).await;
    let client = CoreClient::connect(&socket);
    common::reserve_operation_ids(&client, 8).await;
    client.hello().await.unwrap();
    let record = client.get_operation(OperationId(3)).await.unwrap().unwrap();
    assert_eq!(
        record
            .phases
            .iter()
            .find(|p| p.phase == ConfigPhase::Persist)
            .unwrap()
            .outcome,
        PhaseOutcome::Confirmed
    );
    assert_eq!(
        record
            .phases
            .iter()
            .find(|p| p.phase == ConfigPhase::Native)
            .unwrap()
            .outcome,
        PhaseOutcome::Unknown,
        "kernel Native has no reliable apply receipt — stay Unknown"
    );
    assert_eq!(record.settled, Some(PhaseOutcome::Unknown));
    assert_eq!(fs::read_to_string(&ctl).unwrap(), ctl_before);
}
