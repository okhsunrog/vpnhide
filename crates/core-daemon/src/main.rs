//! Minimal runnable core: bind the socket, serve, and publish a toggling snapshot every
//! few seconds so a connected client observes `state_since` firing end to end. This is a
//! skeleton entry point — real boot integration, recovery, and the observation loop land
//! with the daemon proper.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use vpnhide_core_daemon::{serve, CoreState, Publisher};
use vpnhide_core_rpc::StateSnapshot;

#[tokio::main]
async fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/data/local/tmp/vpnhide-core.sock".to_owned());

    // A fresh epoch per run: the client adopts on an epoch change after a restart (§6).
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(1);

    let state = CoreState::new(Publisher::new(epoch, StateSnapshot::default()));
    let handle = match serve(Path::new(&path), state.clone()) {
        Ok(handle) => handle,
        Err(e) => {
            eprintln!("core: bind failed on {path}: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("core: epoch={epoch} serving on {path} (ctrl-c to stop)");

    let ticker = state.clone();
    tokio::spawn(async move {
        let mut on = false;
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            on = !on;
            let mut snapshot = ticker.publisher.current().snapshot;
            snapshot.config_available = on;
            ticker.publisher.publish(snapshot);
        }
    });

    let _ = tokio::signal::ctrl_c().await;
    eprintln!("core: shutting down");
    handle.shutdown_and_unlink();
}
