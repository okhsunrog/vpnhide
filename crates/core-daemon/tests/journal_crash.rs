//! Crash-window coverage for the durable journal: kill the core at phase
//! boundaries, restart, and assert intent/identity/uncertainty without effect
//! replay. Uses `VPNHIDE_TEST_CRASH_AFTER` (production never sets it).

mod common;
use std::fs;
use std::process::{Child, Stdio};
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
        maintenance: None,
        maintenance_report: None,
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
    let mut cmd = common::core_command();
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

async fn wait_revocation_crash(child: &mut CoreProcess) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                assert_eq!(status.code(), Some(97));
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("revocation crash boundary reached");
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
    // Durable admission may arrive before the injected crash.
    let _ = submit;
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
    common::settled(&client, OperationId(2)).await;
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
    let _ = submit; // Accepted or a lost reply are both recoverable by ID.
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
    let _ = submit; // Accepted or a lost reply are both recoverable by ID.
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

#[tokio::test]
async fn durable_admission_precedes_slow_effect_and_survives_disconnect() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("core.sock");
    let config = dir.path().join("config.json");
    let mut child = CoreProcess(
        common::core_command()
            .arg(&socket)
            .env("VPNHIDE_CONFIG_PATH", &config)
            .env("VPNHIDE_TEST_DELAY_BEFORE", "persist")
            .env("VPNHIDE_TEST_DELAY_MS", "2000")
            .env_remove("VPNHIDE_KPM_LOCK")
            .env_remove("VPNHIDE_KERNEL_MODULES")
            .env_remove("VPNHIDE_KMOD_CTL")
            .env_remove("VPNHIDE_BOOT_APPLY")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    wait_bound(&socket, &mut child).await;
    let client = CoreClient::connect(&socket);
    let op = client.allocate_operation_id().await.unwrap().unwrap();
    let edit = ConfigEdit::Replace {
        config: CanonicalConfig {
            debug: true,
            ..Default::default()
        },
    };
    let accepted = tokio::time::timeout(
        Duration::from_secs(1),
        client.submit_edit(op, BaseRef { revision: 0 }, edit.clone()),
    )
    .await
    .expect("Accepted must not wait for the two-second external effect")
    .unwrap()
    .unwrap();
    assert_eq!(accepted.op, op);
    assert!(!config.exists());
    let disk = journal::read(&dir.path().join("core_operations.json")).unwrap();
    assert_eq!(disk.get(op).unwrap().record.settled, None);
    assert!(
        client
            .current()
            .await
            .unwrap()
            .snapshot
            .operations_in_flight
            .contains(&op)
    );
    drop(client);
    let reconnected = CoreClient::connect(&socket);
    let record = common::settled(&reconnected, op).await;
    assert_eq!(record.settled, Some(PhaseOutcome::Confirmed));
    reconnected
        .submit_edit(op, BaseRef { revision: 0 }, edit)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reconnected.get_operation(op).await.unwrap(), Some(record));
    assert_eq!(
        reconnected
            .current()
            .await
            .unwrap()
            .snapshot
            .config_revision,
        1
    );
}

/// Exercise real process death, not a synthetic journal alone. The fake loader
/// exposes identity but cannot establish actual kernel/Android compatibility.
#[tokio::test]
async fn startup_crash_repair_never_replays_a_load_in_the_same_boot() {
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::PermissionsExt;
    for point in [
        "startup-intent",
        "startup-before-effects",
        "startup-after-effects",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let socket = root.join("core.sock");
        let config_path = root.join("config.json");
        let journal_path = root.join("core_operations.json");
        let ctl = root.join("ctl");
        let module = root.join("modules/vpnhide_kmod");
        fs::create_dir_all(&module).unwrap();
        fs::write(module.join("module.prop"), "id=vpnhide_kmod\n").unwrap();
        let artifact = module.join("vpnhide_kmod.ko");
        fs::write(&artifact, b"first payload").unwrap();
        let config = CanonicalConfig::default();
        config_file::write_atomic(&config_path, &config).unwrap();
        common::link_host_tool(root, "cat");
        let loader = root.join("insmod");
        fs::write(&loader, "#!/bin/sh\ncat \"$1\" > \"$VH_ROOT/loaded\" || exit 1\nprintf 'load\\n' >> \"$VH_ROOT/loads\"\nprintf 'vpnhide 1 status\\nbackend 0x0\\nkver 0x0\\nhooks 0x0\\nerror 0x0\\n' > \"$VPNHIDE_KMOD_CTL\"\n").unwrap();
        fs::set_permissions(&loader, fs::Permissions::from_mode(0o700)).unwrap();
        let spawn = |crash: Option<&str>| {
            let mut command = common::core_command();
            command
                .arg(&socket)
                .env("PATH", root)
                .env("VH_ROOT", root)
                .env("VPNHIDE_CONFIG_PATH", &config_path)
                .env("VPNHIDE_CORE_OWNER_DIR", root.join("owner"))
                .env("VPNHIDE_KMOD_CTL", &ctl)
                .env("VPNHIDE_KERNEL_MODULES", root.join("modules"))
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            if let Some(crash) = crash {
                command.env("VPNHIDE_TEST_CRASH_AFTER", crash);
            }
            CoreProcess(command.spawn().unwrap())
        };
        let mut crashed = spawn(Some(point));
        let exit = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(exit) = crashed.0.try_wait().unwrap() {
                    break exit;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(exit.code(), Some(97), "{point}");
        let interrupted = journal::read(&journal_path).unwrap();
        let attempt = interrupted.native_startup.clone().unwrap();
        assert!(attempt.completed.is_none());
        assert_eq!(
            attempt
                .intent
                .as_ref()
                .unwrap()
                .payload
                .as_ref()
                .unwrap()
                .sha256,
            format!("{:x}", Sha256::digest(b"first payload"))
        );
        let after = point == "startup-after-effects";
        assert_eq!(root.join("loads").exists(), after);
        // Replacing a source never alters the interrupted intent or authorizes replay.
        fs::write(&artifact, b"second payload").unwrap();
        let mut restarted = spawn(None);
        wait_bound(&socket, &mut restarted).await;
        let client = CoreClient::connect(&socket);
        common::reserve_operation_ids(&client, 1).await;
        assert!(
            client
                .current()
                .await
                .unwrap()
                .snapshot
                .held_resources
                .contains(&ConfigPhase::Native)
        );
        client
            .submit_edit(
                OperationId(1),
                BaseRef { revision: 1 },
                ConfigEdit::RepairActivation,
            )
            .await
            .unwrap()
            .unwrap();
        let repaired = common::settled(&client, OperationId(1)).await;
        assert_eq!(
            repaired.settled,
            Some(if after {
                PhaseOutcome::Confirmed
            } else {
                PhaseOutcome::FailedKnown
            }),
            "{point}"
        );
        assert_eq!(
            fs::read_to_string(root.join("loads")).unwrap_or_default(),
            if after { "load\n" } else { "" }
        );
        assert_eq!(
            config_file::read(&config_path).unwrap(),
            Some(config.clone())
        );
        let stored = journal::read(&journal_path).unwrap();
        if after {
            assert!(!stored.native_startup_uncertain());
            assert_eq!(stored.native_attempts.last().unwrap().attempt, attempt);
            assert_eq!(
                stored.native_attempts.last().unwrap().superseded_by,
                Some(OperationId(1))
            );
            assert_eq!(fs::read(root.join("loaded")).unwrap(), b"first payload");
        } else {
            assert!(stored.native_startup_uncertain());
            drop(client);
            drop(restarted);
            // Simulate only the boot-generation boundary in this host fixture.
            // A real phone must reboot; no runtime API can mutate its boot ID.
            let mut rebooted = stored;
            rebooted.native_startup.as_mut().unwrap().boot_id = "prior-boot".into();
            rebooted
                .native_startup
                .as_mut()
                .unwrap()
                .intent
                .as_mut()
                .unwrap()
                .boot_id = "prior-boot".into();
            rebooted.native_load_budget.as_mut().unwrap().boot_id = "prior-boot".into();
            journal::write_atomic(&journal_path, &rebooted).unwrap();
            let _ = fs::remove_file(&socket);
            let mut restarted = spawn(None);
            wait_bound(&socket, &mut restarted).await;
            let client = CoreClient::connect(&socket);
            common::reserve_operation_ids(&client, 2).await;
            assert!(!root.join("loads").exists(), "reboot alone must not retry");
            client
                .submit_edit(
                    OperationId(2),
                    BaseRef { revision: 1 },
                    ConfigEdit::RepairActivation,
                )
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                common::settled(&client, OperationId(2)).await.settled,
                Some(PhaseOutcome::Confirmed)
            );
            assert_eq!(fs::read(root.join("loaded")).unwrap(), b"second payload");
            assert_eq!(fs::read_to_string(root.join("loads")).unwrap(), "load\n");
            let stored = journal::read(&journal_path).unwrap();
            assert!(!stored.native_startup_uncertain());
            assert!(
                stored
                    .native_attempts
                    .last()
                    .unwrap()
                    .attempt
                    .completed
                    .is_none()
            );
            assert_eq!(
                stored
                    .native_startup
                    .unwrap()
                    .intent
                    .unwrap()
                    .payload
                    .unwrap()
                    .sha256,
                format!("{:x}", Sha256::digest(b"second payload"))
            );
        }
    }
}

#[tokio::test]
async fn revocation_crash_windows_recover_exact_grant_without_config_or_backend_publication() {
    use journal::{
        NativeIntent, ZygiskAuthorityRecord, ZygiskAuthorizationIntent,
        revocation::{RevocationState, ZygiskRevocation},
    };
    use std::os::unix::fs::MetadataExt;
    use vpnhide_core_rpc::NativeBackend;
    use vpnhide_protocol::zygisk_activation::{Authorization, FILE_NAME};
    for point in [
        "revocation-admission",
        "revocation-intent",
        "revocation-effect",
        "revocation-completion",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let module = dir.path().join("module");
        fs::create_dir_all(module.join("zygisk")).unwrap();
        fs::write(
            module.join("module.prop"),
            "id=vpnhide_core\nvpnhideControl=2\nvpnhideActivation=1\n",
        )
        .unwrap();
        // Metadata-only payload fixture. No loader is enabled or executed here.
        let mut elf = [0; 64];
        elf[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        elf[16..18].copy_from_slice(&3u16.to_le_bytes());
        elf[18..20].copy_from_slice(&183u16.to_le_bytes());
        fs::write(module.join("zygisk/arm64-v8a.so"), elf).unwrap();
        let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .unwrap()
            .trim()
            .to_owned();
        let wire = "vpnhide 2 config\ndebug 0\ndefault 0\nend 0\n".to_owned();
        let auth = Authorization {
            boot_id: boot.clone(),
            generation: 42,
            wire: wire.clone(),
        };
        fs::write(module.join(FILE_NAME), auth.encode().unwrap()).unwrap();
        let meta = fs::metadata(&module).unwrap();
        let authority = ZygiskAuthorityRecord {
            operation: OperationId(42),
            intent: NativeIntent {
                backend: NativeBackend::Zygisk,
                wire,
                zygisk_module: Some(module.clone()),
                zygisk_authorization: Some(ZygiskAuthorizationIntent {
                    boot_id: boot.clone(),
                    generation: 42,
                    directory_identity: (meta.dev(), meta.ino()),
                }),
            },
        };
        let stored = journal::Journal {
            issued_through: 42,
            config_revision: 9,
            zygisk_authority: Some(authority.clone()),
            zygisk_revocation: Some(ZygiskRevocation {
                operation: Some(OperationId(42)),
                current_boot: boot,
                authority: authority.clone(),
                state: RevocationState::Unknown,
            }),
            ..Default::default()
        };
        let config = dir.path().join("config.json");
        let socket = dir.path().join("core.sock");
        let journal_path = dir.path().join("core_operations.json");
        journal::write_atomic(&journal_path, &stored).unwrap();
        let mut child = spawn_core(&socket, &config, Some(point), None);
        wait_bound(&socket, &mut child).await;
        let client = CoreClient::connect(&socket);
        client.hello().await.unwrap();
        let op = client.allocate_operation_id().await.unwrap().unwrap();
        assert_eq!(op, OperationId(43));
        let _uncertain_reply = client
            .submit_edit(op, BaseRef { revision: 9 }, ConfigEdit::RepairActivation)
            .await;
        wait_revocation_crash(&mut child).await;
        assert_eq!(
            module.join(FILE_NAME).exists(),
            matches!(point, "revocation-admission" | "revocation-intent")
        );
        fs::remove_file(&socket).ok();
        let retry_crash = (point != "revocation-completion").then_some("revocation-completion");
        let mut restarted = spawn_core(&socket, &config, retry_crash, None);
        wait_bound(&socket, &mut restarted).await;
        let client = CoreClient::connect(&socket);
        client.hello().await.unwrap();
        let receipt = client.get_operation(op).await.unwrap().unwrap();
        let (client, _final_process) = if point == "revocation-completion" {
            assert_eq!(receipt.settled, Some(PhaseOutcome::Confirmed));
            (client, None)
        } else {
            assert_eq!(receipt.settled, Some(PhaseOutcome::Unknown));
            let repair = client.allocate_operation_id().await.unwrap().unwrap();
            let _uncertain_retry = client
                .submit_edit(
                    repair,
                    BaseRef { revision: 9 },
                    ConfigEdit::RepairActivation,
                )
                .await;
            wait_revocation_crash(&mut restarted).await;
            fs::remove_file(&socket).ok();
            let mut final_process = spawn_core(&socket, &config, None, None);
            wait_bound(&socket, &mut final_process).await;
            let replacement = CoreClient::connect(&socket);
            replacement.hello().await.unwrap();
            assert_eq!(
                replacement
                    .get_operation(repair)
                    .await
                    .unwrap()
                    .unwrap()
                    .settled,
                Some(PhaseOutcome::Confirmed)
            );
            assert_eq!(
                replacement
                    .get_operation(op)
                    .await
                    .unwrap()
                    .unwrap()
                    .settled,
                Some(PhaseOutcome::Unknown),
                "historical Unknown is immutable"
            );
            (replacement, Some(final_process))
        };
        let publication = client.state_since(0, 0).await.unwrap();
        assert!(publication.snapshot.config.is_none());
        assert_eq!(publication.snapshot.config_revision, 9);
        assert_eq!(publication.snapshot.backend.applied, None);
        assert!(
            !publication
                .snapshot
                .apply_evidence
                .iter()
                .any(|e| e.phase == ConfigPhase::Native)
        );
        assert!(
            publication
                .snapshot
                .zygisk_revocation
                .as_ref()
                .is_some_and(|r| !r.pending())
        );
        assert!(!config.exists());
        assert!(!module.join(FILE_NAME).exists());
        let stored = journal::read(&journal_path).unwrap();
        assert_eq!(stored.zygisk_authority, Some(authority));
        assert!(stored.native_startup.is_none());
        assert!(stored.native_load_budget.is_none());
        assert!(stored.kmod_preflights.is_none());
        assert!(
            stored.held_resources().is_empty(),
            "only this exact narrow repair was superseded"
        );
    }
}
