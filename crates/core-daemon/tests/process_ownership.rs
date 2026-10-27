//! Real process ownership and diagnostic availability with unreadable durable state.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use vpnhide_core_client::CoreClient;

struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn command(socket: &Path, config: &Path, journal: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_vpnhide-core"));
    command
        .arg(socket)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("VPNHIDE_CONFIG_PATH", config)
        .env("VPNHIDE_JOURNAL_PATH", journal)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

async fn ready(client: &CoreClient) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while client.current().await.is_err() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("daemon never became reachable");
}

#[tokio::test]
async fn duplicate_process_cannot_take_state_or_unlink_the_owner_socket() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("core.sock");
    let config = dir.path().join("canonical.json");
    let journal = dir.path().join("operations.json");
    let _first = Daemon(command(&socket, &config, &journal).spawn().unwrap());
    let client = CoreClient::connect(&socket);
    ready(&client).await;
    let epoch = client.hello().await.unwrap();

    // Even a different journal cannot bypass ownership of the canonical resource.
    let mut second = Daemon(
        command(&socket, &config, &dir.path().join("other.json"))
            .spawn()
            .unwrap(),
    );
    let status = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(status) = second.0.try_wait().unwrap() {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(status.code(), Some(73));
    // Open a new connection: an old accepted channel alone would not detect unlinking.
    assert_eq!(CoreClient::connect(&socket).hello().await.unwrap(), epoch);
}

#[tokio::test]
async fn corrupt_journal_keeps_read_only_state_available_and_refuses_mutations() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("core.sock");
    let config = dir.path().join("canonical.json");
    let journal = dir.path().join("operations.json");
    std::fs::write(&journal, b"{corrupt-journal").unwrap();
    let _daemon = Daemon(command(&socket, &config, &journal).spawn().unwrap());
    let client = CoreClient::connect(&socket);
    ready(&client).await;
    let snapshot = client.current().await.unwrap().snapshot;
    assert!(snapshot.startup_error.is_some());
    assert!(snapshot.config.is_none());
    assert!(client.allocate_operation_id().await.unwrap().is_err());
    assert!(!config.exists());
    assert_eq!(std::fs::read(journal).unwrap(), b"{corrupt-journal");
}
