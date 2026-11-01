//! Real CLI parsing, early refusal and deterministic process-fixture environment.
mod common;
use std::{
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};
use vpnhide_core_client::CoreClient;
use vpnhide_core_rpc::{BaseRef, CanonicalConfig, ConfigEdit};

#[test]
fn invalid_ipc_configuration_refuses_before_touching_durable_paths() {
    use std::os::unix::ffi::OsStringExt;
    for raw in [
        "".into(),
        "bad".into(),
        "123,bad".into(),
        "4294967296".into(),
        std::ffi::OsString::from_vec(vec![255]),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("state/config.json");
        let out = common::core_command()
            .arg(dir.path().join("core.sock"))
            .env("VPNHIDE_CONFIG_PATH", &config)
            .env("VPNHIDE_CORE_ALLOW_UIDS", raw)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2));
        assert!(
            !config.parent().unwrap().exists(),
            "validation must precede locks/journal IO"
        );
        assert!(!dir.path().join("core.sock").exists());
    }
}

#[test]
fn contaminated_parent_cannot_redirect_fixture_or_enable_crash_injection() {
    let dir = tempfile::tempdir().unwrap();
    let result = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "fixture_child", "--nocapture"])
        .env("B4_FIXTURE_ROOT", dir.path())
        .env("VPNHIDE_CONFIG_PATH", dir.path().join("wrong/config.json"))
        .env(
            "VPNHIDE_JOURNAL_PATH",
            dir.path().join("wrong/journal.json"),
        )
        .env("VPNHIDE_APP_DATA_DIR", dir.path().join("wrong/app"))
        .env("VPNHIDE_CORE_ALLOW_UIDS", "bad")
        .env("VPNHIDE_TEST_CRASH_AFTER", "persist")
        .env("VPNHIDE_ROOT_EFFECTS", "1")
        .env("VPNHIDE_PORTS", "1")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!dir.path().join("wrong").exists());
    assert!(dir.path().join("canonical.json").exists());
}

struct Daemon(std::process::Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[tokio::test]
async fn fixture_child() {
    let Some(root) = std::env::var_os("B4_FIXTURE_ROOT") else {
        return;
    };
    let root = Path::new(&root);
    let path = root.join("core.sock");
    let _process = Daemon(
        common::core_command()
            .arg(&path)
            .env("VPNHIDE_CONFIG_PATH", root.join("canonical.json"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let client = CoreClient::connect(&path);
    tokio::time::timeout(Duration::from_secs(5), async {
        while client.hello().await.is_err() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let id = client.allocate_operation_id().await.unwrap().unwrap();
    client
        .submit_edit(
            id,
            BaseRef { revision: 0 },
            ConfigEdit::Replace {
                config: CanonicalConfig::default(),
            },
        )
        .await
        .unwrap()
        .unwrap();
    let record = common::settled(&client, id).await;
    assert_eq!(
        record.settled,
        Some(vpnhide_core_rpc::PhaseOutcome::Confirmed)
    );
}
