//! Minimal runnable core:
//!   - (default) `vpnhide-core [socket]` — bind, serve, and run the netlink observation loop.
//!   - `vpnhide-core dump [socket]` — connect as a client and print the current snapshot.
//!   - `vpnhide-core project [config.json]` — print the built-in control-v2 wire projected
//!     from a config file (default: `$VPNHIDE_CONFIG_PATH`). **Read-only**: it only projects
//!     (resolving uids from `pm`), never delivers — safe to run against the real config.
//!   - `vpnhide-core status [ctl]` — read a kernel control node and print its parsed
//!     telemetry-v1 status (default `/proc/vpnhide_ctl`). **Read-only.**
//!
//! Skeleton entry point — real boot integration and recovery land with the daemon proper.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use vpnhide_core_daemon::activation;
use vpnhide_core_daemon::config_file::{self, config_path_from_env};
use vpnhide_core_daemon::{observe, serve, CoreClient, CoreState, Publisher};
use vpnhide_core_rpc::StateSnapshot;

const DEFAULT_SOCKET: &str = "/data/local/tmp/vpnhide-core.sock";
const DEFAULT_CTL: &str = "/proc/vpnhide_ctl";

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let first = args.next().unwrap_or_default();
    match first.as_str() {
        "dump" => {
            dump(&args.next().unwrap_or_else(|| DEFAULT_SOCKET.to_owned())).await;
            return;
        }
        "project" => {
            project(args.next());
            return;
        }
        "status" => {
            status(&args.next().unwrap_or_else(|| DEFAULT_CTL.to_owned()));
            return;
        }
        _ => {}
    }
    let path = if first.is_empty() { DEFAULT_SOCKET.to_owned() } else { first };
    run(&path).await;
}

/// Read-only: project a config file to the built-in control-v2 wire and print it.
fn project(config_arg: Option<String>) {
    let path = config_arg.map(std::path::PathBuf::from).unwrap_or_else(config_path_from_env);
    let config = match config_file::read(&path) {
        Ok(Some(config)) => config,
        Ok(None) => {
            eprintln!("project: no usable config at {}", path.display());
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("project: read failed on {}: {e}", path.display());
            std::process::exit(1);
        }
    };
    match activation::project_builtin_wire(&config) {
        Ok(wire) => print!("{wire}"),
        Err(e) => {
            eprintln!("project: {e}");
            std::process::exit(1);
        }
    }
}

/// Read-only: read a kernel control node's telemetry-v1 status and print it.
fn status(ctl: &str) {
    match activation::read_status(Path::new(ctl)) {
        Ok(s) => println!(
            "backend=0x{:x} kver=0x{:x} hooks=0x{:x} error=0x{:x}",
            s.backend, s.kver, s.hooks, s.error,
        ),
        Err(e) => {
            eprintln!("status: {e} ({ctl})");
            std::process::exit(1);
        }
    }
}

async fn run(path: &str) {
    // A fresh epoch per run: the client adopts on an epoch change after a restart (§6).
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(1);

    // The durable canonical config (§4 tier-1). Path is overridable via VPNHIDE_CONFIG_PATH
    // so the dev sandbox core never touches the real /data/system file.
    let config_path = config_path_from_env();
    // The Native phase's control node (§4/§9.1). Unset via VPNHIDE_KMOD_CTL = no native write,
    // so the dev sandbox core never touches the real /proc/vpnhide_ctl.
    let kmod_ctl_path = vpnhide_core_daemon::activation::kmod_ctl_path_from_env();
    let state = match CoreState::load(
        Publisher::new(epoch, StateSnapshot::default()),
        config_path.clone(),
        kmod_ctl_path.clone(),
    ) {
        Ok(state) => state,
        Err(e) => {
            eprintln!("core: config load failed on {}: {e}", config_path.display());
            std::process::exit(1);
        }
    };
    let handle = match serve(Path::new(path), state.clone()) {
        Ok(handle) => handle,
        Err(e) => {
            eprintln!("core: bind failed on {path}: {e}");
            std::process::exit(1);
        }
    };
    eprintln!(
        "core: epoch={epoch} serving on {path} config={} ({}) native_ctl={} (ctrl-c to stop)",
        config_path.display(),
        if state.has_config() { "loaded" } else { "none" },
        kmod_ctl_path.as_ref().map_or_else(|| "disabled".to_string(), |p| p.display().to_string()),
    );

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
