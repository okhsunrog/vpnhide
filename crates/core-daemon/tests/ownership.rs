//! Ownership and conflict behaviour over the real RPC transport: accepted work
//! survives client disconnect; unrelated field edits from two clients merge;
//! same-field stale bases conflict; delayed Native dispatch can be fenced.

mod common;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use vpnhide_core_client::CoreClient;
use vpnhide_core_rpc::{
    BaseRef, CanonicalConfig, ConfigEdit, ConfigToggle, OperationId, PhaseOutcome, SubmitError,
};

struct CoreProcess(Child);

impl Drop for CoreProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn(
    socket: &std::path::Path,
    config_path: &std::path::Path,
    ctl: Option<&std::path::Path>,
    delay_before: Option<&str>,
) -> CoreProcess {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vpnhide-core"));
    cmd.arg(socket)
        .env("VPNHIDE_CONFIG_PATH", config_path)
        .env_remove("VPNHIDE_KPM_LOCK")
        .env_remove("VPNHIDE_KERNEL_MODULES")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match ctl {
        Some(p) => {
            cmd.env("VPNHIDE_KMOD_CTL", p);
        }
        None => {
            cmd.env_remove("VPNHIDE_KMOD_CTL");
        }
    }
    match delay_before {
        Some(point) => {
            cmd.env("VPNHIDE_TEST_DELAY_BEFORE", point)
                .env("VPNHIDE_TEST_DELAY_MS", "300");
        }
        None => {
            cmd.env_remove("VPNHIDE_TEST_DELAY_BEFORE")
                .env_remove("VPNHIDE_TEST_DELAY_MS");
        }
    }
    CoreProcess(cmd.spawn().unwrap())
}

async fn wait_bound(socket: &std::path::Path, child: &mut CoreProcess) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !socket.exists() {
            assert!(child.0.try_wait().unwrap().is_none());
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("bound");
}

#[tokio::test]
async fn accepted_edit_survives_client_disconnect() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("core.sock");
    let config = dir.path().join("config.json");
    let mut child = spawn(&socket, &config, None, Some("persist"));
    wait_bound(&socket, &mut child).await;

    // Submit then abort the waiter while the core is delayed in Persist —
    // simulates client cancel / socket loss after acceptance.
    let submit = tokio::spawn({
        let socket = socket.clone();
        async move {
            let client = CoreClient::connect(&socket);
            common::reserve_operation_ids(&client, 4).await;
            client.hello().await.unwrap();
            client
                .submit_edit(
                    OperationId(1),
                    BaseRef { revision: 0 },
                    ConfigEdit::Replace {
                        config: CanonicalConfig {
                            debug: true,
                            ..Default::default()
                        },
                    },
                )
                .await
        }
    });
    tokio::time::sleep(Duration::from_millis(80)).await;
    submit.abort();
    let _ = submit.await;

    // Give the owned worker time to finish Persist after the client vanished.
    tokio::time::sleep(Duration::from_millis(400)).await;

    let client = CoreClient::connect(&socket);
    common::reserve_operation_ids(&client, 4).await;
    client.hello().await.unwrap();
    let record = client
        .get_operation(OperationId(1))
        .await
        .unwrap()
        .expect("core still owns the accepted operation");
    assert_eq!(record.settled, Some(PhaseOutcome::Confirmed));
    assert!(config.exists());
}

#[tokio::test]
async fn two_clients_merge_unrelated_fields_and_conflict_on_same_field() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("core.sock");
    let config = dir.path().join("config.json");
    let mut child = spawn(&socket, &config, None, None);
    wait_bound(&socket, &mut child).await;

    let a = CoreClient::connect(&socket);
    common::reserve_operation_ids(&a, 4).await;
    let b = CoreClient::connect(&socket);
    a.hello().await.unwrap();
    b.hello().await.unwrap();

    a.submit_edit(
        OperationId(1),
        BaseRef { revision: 0 },
        ConfigEdit::Replace {
            config: CanonicalConfig::default(),
        },
    )
    .await
    .unwrap()
    .unwrap();

    a.submit_edit(
        OperationId(2),
        BaseRef { revision: 1 },
        ConfigEdit::Toggle {
            field: ConfigToggle::Debug,
            enabled: true,
        },
    )
    .await
    .unwrap()
    .unwrap();

    // Client B still at revision 1 — unrelated AutoHideName merges.
    b.submit_edit(
        OperationId(3),
        BaseRef { revision: 1 },
        ConfigEdit::Toggle {
            field: ConfigToggle::AutoHideName,
            enabled: true,
        },
    )
    .await
    .unwrap()
    .unwrap();

    // Same field with stale base conflicts.
    let conflict = b
        .submit_edit(
            OperationId(4),
            BaseRef { revision: 1 },
            ConfigEdit::Toggle {
                field: ConfigToggle::Debug,
                enabled: false,
            },
        )
        .await
        .unwrap();
    assert!(matches!(conflict, Err(SubmitError::Conflict { field }) if field == "debug"));
}
