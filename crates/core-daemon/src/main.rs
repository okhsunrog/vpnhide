//! Minimal runnable core with two modes:
//!   - (default) `vpnhide-core [socket]` — bind, serve, and run the netlink observation
//!     loop feeding real VPN-presence into the published state.
//!   - `vpnhide-core dump [socket]` — connect as a client, handshake, and print the core's
//!     current snapshot once. A dev tool to read what the observer sees end to end.
//!
//! Skeleton entry point — real boot integration, recovery, and config land with the daemon
//! proper.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use vpnhide_core_daemon::{observe, serve, CoreClient, CoreState, Publisher};
use vpnhide_core_rpc::StateSnapshot;

const DEFAULT_SOCKET: &str = "/data/local/tmp/vpnhide-core.sock";

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let first = args.next().unwrap_or_default();
    if first == "dump" {
        dump(&args.next().unwrap_or_else(|| DEFAULT_SOCKET.to_owned())).await;
        return;
    }
    let path = if first.is_empty() { DEFAULT_SOCKET.to_owned() } else { first };
    run(&path).await;
}

async fn run(path: &str) {
    // A fresh epoch per run: the client adopts on an epoch change after a restart (§6).
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(1);

    let state = CoreState::new(Publisher::new(epoch, StateSnapshot::default()));
    let handle = match serve(Path::new(path), state.clone()) {
        Ok(handle) => handle,
        Err(e) => {
            eprintln!("core: bind failed on {path}: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("core: epoch={epoch} serving on {path} (ctrl-c to stop)");

    // The root observation loop feeds real VPN-presence into the published state. Failing to
    // open netlink (no permission, restricted SELinux) is logged, not fatal: the socket still
    // serves and the app sees routing as never-yet-observed rather than a crash.
    let observed = state.clone();
    tokio::spawn(async move {
        if let Err(e) = observe(observed).await {
            eprintln!("core: observation loop stopped: {e}");
        }
    });

    let _ = tokio::signal::ctrl_c().await;
    eprintln!("core: shutting down");
    handle.shutdown_and_unlink();
}

async fn dump(path: &str) {
    let client = CoreClient::connect_and_mirror(path.to_owned());
    match client.hello().await {
        Ok(epoch) => eprintln!("dump: handshake ok, core epoch={epoch}"),
        Err(e) => {
            eprintln!("dump: handshake failed: {e}");
            std::process::exit(1);
        }
    }
    match client.current().await {
        Ok(p) => println!(
            "epoch={} revision={} config_present={} apps={} observed={} vpn=[{}]",
            p.epoch,
            p.revision,
            p.snapshot.config.is_some(),
            p.snapshot.config.as_ref().map_or(0, |c| c.apps.len()),
            p.snapshot.routing.observed,
            p.snapshot.routing.vpn_interfaces.join(","),
        ),
        Err(e) => {
            eprintln!("dump: read failed: {e}");
            std::process::exit(1);
        }
    }
}
