//! Real native CLI -> authenticated RPC -> durable writer, without a device.
mod common;
use std::{fs, path::Path, time::Duration};
use vpnhide_core_daemon::{CoreState, Publisher, auth, effects::EffectPaths, serve_with_allowlist};
use vpnhide_core_rpc::{CanonicalConfig, StateSnapshot};
use vpnhide_maintenance_model::{ResetReply, ResetStatus};
fn cli(action: &str, op: Option<u64>, socket: &Path) -> ResetReply {
    let mut command = common::core_command();
    command.args(["maintenance-reset", action]);
    if let Some(op) = op {
        command.arg(op.to_string());
    }
    command
        .arg(
            fs::read_to_string("/proc/sys/kernel/random/boot_id")
                .unwrap()
                .trim(),
        )
        .arg(socket);
    let out = vpnhide_bounded_process::capture_command(
        &mut command,
        None,
        Duration::from_secs(10),
        16 * 1024,
    )
    .unwrap();
    assert!(
        !out.timed_out && !out.truncated && out.output.status.success(),
        "{}",
        String::from_utf8_lossy(&out.output.stderr)
    );
    serde_json::from_slice(&out.output.stdout).unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reset_cli_uses_existing_writer_and_preserves_journal_and_module_installation() {
    let arena = tempfile::tempdir().unwrap();
    let root = arena.path().join("device");
    let config = root.join("data/system/vpnhide_config.json");
    let journal = root.join("data/adb/vpnhide/core/operations.json");
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::create_dir_all(journal.parent().unwrap()).unwrap();
    fs::write(
        &config,
        serde_json::to_vec(&CanonicalConfig::default()).unwrap(),
    )
    .unwrap();
    fs::write(root.join("data/adb/vpnhide/superkey"), b"test-only-secret").unwrap();
    let module = root.join("data/adb/modules/vpnhide_core/module.prop");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    fs::write(&module, b"keep installed module").unwrap();
    let state = CoreState::load_with_journal(
        Publisher::new(731, StateSnapshot::default()),
        config.clone(),
        Some(journal.clone()),
        None,
    )
    .unwrap();
    state.set_effect_paths(Some(EffectPaths { root: root.clone() }));
    let socket = arena.path().join("core.sock");
    let handle = serve_with_allowlist(
        &socket,
        state.clone(),
        auth::Allowlist::Uids(std::collections::BTreeSet::from([unsafe {
            libc::geteuid()
        }])),
    )
    .unwrap();
    let allocated = cli("allocate", None, &socket);
    assert_eq!(allocated.status, ResetStatus::Allocated);
    let op = allocated.operation.unwrap();
    assert_eq!(allocated.epoch, Some(731));
    assert_eq!(
        cli("submit", Some(op), &socket).status,
        ResetStatus::Accepted
    );
    let receipt = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let reply = cli("query", Some(op), &socket);
            if reply.status != ResetStatus::Running {
                break reply;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(receipt.status, ResetStatus::Confirmed);
    assert!(!config.exists());
    assert!(!root.join("data/adb/vpnhide/superkey").exists());
    assert!(state.publisher.current().snapshot.config.is_none());
    assert!(journal.is_file());
    assert_eq!(fs::read(&module).unwrap(), b"keep installed module");
    let before = fs::read(&journal).unwrap();
    assert_eq!(
        cli("query", Some(op), &socket).status,
        ResetStatus::Confirmed
    );
    assert_eq!(
        fs::read(&journal).unwrap(),
        before,
        "receipt query is not a second reset"
    );
    // A fresh configuration invalidates the old reset's current-state proof;
    // its own confirmed edit receipt also cannot impersonate FullReset.
    let client = vpnhide_core_client::CoreClient::connect(&socket);
    let edit = client.allocate_operation_id().await.unwrap().unwrap();
    client
        .submit_edit(
            edit,
            vpnhide_core_rpc::BaseRef {
                revision: state.config_revision(),
            },
            vpnhide_core_rpc::ConfigEdit::Replace {
                config: CanonicalConfig::default(),
            },
        )
        .await
        .unwrap()
        .unwrap();
    let edited = common::settled(&client, edit).await;
    assert_eq!(
        edited.settled,
        Some(vpnhide_core_rpc::PhaseOutcome::Confirmed)
    );
    assert_eq!(
        cli("query", Some(edit.0), &socket).status,
        ResetStatus::NotFound
    );
    assert_eq!(
        cli("query", Some(op), &socket).status,
        ResetStatus::NotFound
    );
    handle.shutdown_and_unlink();
    state.shutdown();
}
#[test]
fn wrong_boot_is_rejected_before_connecting_or_starting_a_writer() {
    let arena = tempfile::tempdir().unwrap();
    let socket = arena.path().join("missing.sock");
    let actual = fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap();
    let wrong = if actual.trim() == "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa" {
        "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"
    } else {
        "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
    };
    let out = common::core_command()
        .args(["maintenance-reset", "submit", "17", wrong])
        .arg(&socket)
        .current_dir(arena.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let reply: ResetReply = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(reply.status, ResetStatus::Rejected);
    assert_eq!(reply.boot_id.as_deref(), Some(actual.trim()));
    assert!(reply.epoch.is_none());
    assert!(!socket.exists());
}
#[test]
fn malformed_reset_cli_never_starts_a_daemon_or_touches_durable_paths() {
    let arena = tempfile::tempdir().unwrap();
    for args in [
        vec!["maintenance-reset"],
        vec!["maintenance-reset", "submit", "0"],
        vec!["maintenance-reset", "query", "bad"],
        vec!["maintenance-reset", "anything"],
    ] {
        let config = arena.path().join("must-not-exist/config.json");
        let out = common::core_command()
            .args(args)
            .env("VPNHIDE_CONFIG_PATH", &config)
            .current_dir(arena.path())
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2));
        assert!(!config.parent().unwrap().exists());
        assert!(!arena.path().join("maintenance-reset").exists());
    }
}
#[test]
fn reset_capability_probe_requires_the_exact_cli_schema_and_stays_read_only() {
    let arena = tempfile::tempdir().unwrap();
    for schema in ["0", "999", "bad"] {
        let out = common::core_command()
            .args(["probe", "maintenance-reset-ready", schema])
            .current_dir(arena.path())
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2));
    }
    let out = common::core_command()
        .args([
            "probe",
            "maintenance-reset-ready",
            &vpnhide_maintenance_model::RESET_SCHEMA.to_string(),
        ])
        .current_dir(arena.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
    assert_eq!(fs::read_dir(arena.path()).unwrap().count(), 0);
}
