//! Runnable core:
//!   - (default) `vpnhide-core [socket]` — bind, serve, observation loop; with
//!     `VPNHIDE_BOOT_APPLY=1` re-applies Native/Ports once after load (module boot).
//!   - `vpnhide-core boot-apply` — load, fence/reconcile, re-apply, write boot_status, exit.
//!   - `vpnhide-core dump [socket]` — connect as a client and print the current snapshot.
//!   - `vpnhide-core project [config.json]` — print control-v2 wire (**read-only**).
//!   - `vpnhide-core status [ctl]` — print telemetry-v1 status (**read-only**).

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use vpnhide_core_daemon::config_file::{self, config_path_from_env};
use vpnhide_core_daemon::journal::journal_path_from_env;
use vpnhide_core_daemon::{CoreClient, CoreState, Publisher, observe, serve};
use vpnhide_core_daemon::{activation, backend};
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
        "boot-apply" => {
            boot_apply_once();
            return;
        }
        _ => {}
    }
    let path = if first.is_empty() {
        DEFAULT_SOCKET.to_owned()
    } else {
        first
    };
    run(&path).await;
}

/// Read-only: detect the active native backend, project a config file to *its* control-v2
/// wire, and print it — the centralized flow (detect → project for the detected family).
fn project(config_arg: Option<String>) {
    let path = config_arg
        .map(std::path::PathBuf::from)
        .unwrap_or_else(config_path_from_env);
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
    let ctl = activation::kmod_ctl_path_from_env().unwrap_or_else(|| DEFAULT_CTL.into());
    let modules = backend::kernel_modules_from_env();
    let kpm_lock = backend::kpm_lock_from_env();
    let observed = backend::detect_managed(
        &ctl,
        modules
            .as_deref()
            .unwrap_or_else(|| Path::new(backend::MODULES_ROOT)),
        kpm_lock.as_deref(),
    );
    let Some(active) = observed.active else {
        eprintln!(
            "project: no compatible active native backend (kernel node: {})",
            ctl.display()
        );
        std::process::exit(1);
    };
    match activation::project_native_wire(&config, active) {
        Ok(wire) => print!("{wire}"),
        Err(e) => {
            eprintln!("project: {e}");
            std::process::exit(1);
        }
    }
}

/// Read-only: read a kernel control node's telemetry-v1 status and print it, plus the
/// detected native-backend identity (active + installed).
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
    let observed = backend::detect(Path::new(ctl), Path::new(backend::MODULES_ROOT));
    println!(
        "active={:?} installed={:?}",
        observed.active, observed.installed
    );
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
    // Private operation journal (§4 tier 3 / §5). Overridable via VPNHIDE_JOURNAL_PATH.
    let journal_path = journal_path_from_env();
    // The Native phase's control node (§4/§9.1). Unset via VPNHIDE_KMOD_CTL = no native write,
    // so the dev sandbox core never touches the real /proc/vpnhide_ctl.
    let kmod_ctl_path = vpnhide_core_daemon::activation::kmod_ctl_path_from_env();
    let state = match CoreState::load_with_journal(
        Publisher::new(epoch, StateSnapshot::default()),
        config_path.clone(),
        Some(journal_path.clone()),
        kmod_ctl_path.clone(),
    ) {
        Ok(state) => state,
        Err(e) => {
            eprintln!("core: config load failed on {}: {e}", config_path.display());
            std::process::exit(1);
        }
    };
    // Detect the native backend once at startup and publish it (§4): the app reads the active
    // backend from the replica instead of joining per-backend status files. Read-only — safe
    // against the real control node even when native delivery is disabled. A later refresh
    // trigger (module change / boot) can re-detect; identity changes rarely, not per-tick.
    let modules = backend::kernel_modules_from_env();
    let kpm_lock = backend::kpm_lock_from_env();
    let observed_backend = backend::detect_managed(
        kmod_ctl_path
            .as_deref()
            .unwrap_or_else(|| Path::new(DEFAULT_CTL)),
        modules
            .as_deref()
            .unwrap_or_else(|| Path::new(backend::MODULES_ROOT)),
        kpm_lock.as_deref(),
    );
    state
        .publisher
        .update(|snapshot| snapshot.backend = observed_backend.clone());

    if vpnhide_core_daemon::boot::boot_apply_enabled() {
        let report = vpnhide_core_daemon::boot::reapply(&state);
        report.write_status_file(&vpnhide_core_daemon::boot::default_boot_status_path());
        eprintln!(
            "core: boot-apply status={:?} detail={}",
            report.status, report.detail
        );
    }

    // Finish initial channel observation before admitting mutations; startup
    // must not contend with the first request for the KPM control lock.
    //
    // Production: the socket lives in the app's data dir and is re-bound across
    // reinstalls (app_socket). Host/dev: a plain path socket.
    let app_dir = vpnhide_core_daemon::app_socket::app_data_dir_from_env();
    let handle = match &app_dir {
        Some(dir) => {
            tokio::spawn(vpnhide_core_daemon::app_socket::supervise(
                dir.clone(),
                state.clone(),
            ));
            None
        }
        None => match serve(Path::new(path), state.clone()) {
            Ok(handle) => Some(handle),
            Err(e) => {
                eprintln!("core: bind failed on {path}: {e}");
                std::process::exit(1);
            }
        },
    };
    let path = app_dir
        .as_deref()
        .map(vpnhide_core_daemon::app_socket::socket_path)
        .map_or_else(|| path.to_owned(), |p| p.display().to_string());

    eprintln!(
        "core: epoch={epoch} serving on {path} config={} ({}) journal={} native_ctl={} backend={:?} (ctrl-c to stop)",
        config_path.display(),
        if state.has_config() { "loaded" } else { "none" },
        journal_path.display(),
        kmod_ctl_path
            .as_ref()
            .map_or_else(|| "disabled".to_string(), |p| p.display().to_string()),
        observed_backend.active,
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
    if let Some(handle) = handle {
        handle.shutdown_and_unlink();
    }
}

/// One-shot boot path used by the Magisk module before or instead of long-run serve.
fn boot_apply_once() {
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(1);
    let config_path = config_path_from_env();
    let journal_path = journal_path_from_env();
    let kmod_ctl_path = activation::kmod_ctl_path_from_env();
    let state = match CoreState::load_with_journal(
        Publisher::new(epoch, StateSnapshot::default()),
        config_path.clone(),
        Some(journal_path),
        kmod_ctl_path,
    ) {
        Ok(state) => state,
        Err(e) => {
            eprintln!(
                "boot-apply: config load failed on {}: {e}",
                config_path.display()
            );
            let report = vpnhide_core_daemon::boot::BootApplyReport {
                status: vpnhide_core_daemon::boot::BootApplyStatus::Refused {
                    reason: e.to_string(),
                },
                op: None,
                detail: format!("load failed: {e}"),
            };
            report.write_status_file(&vpnhide_core_daemon::boot::default_boot_status_path());
            std::process::exit(1);
        }
    };
    let report = vpnhide_core_daemon::boot::reapply(&state);
    report.write_status_file(&vpnhide_core_daemon::boot::default_boot_status_path());
    eprintln!(
        "boot-apply: status={:?} detail={}",
        report.status, report.detail
    );
    match report.status {
        vpnhide_core_daemon::boot::BootApplyStatus::Applied
        | vpnhide_core_daemon::boot::BootApplyStatus::NoIntent => {}
        _ => std::process::exit(2),
    }
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
            "epoch={} revision={} config_present={} apps={} routing_freshness={:?} gen={} vpn=[{}] last_good=[{}] candidates=[{}] framework={:?} uid={:?} backend_active={:?} selected={:?} applied={:?} installed={:?}",
            p.epoch,
            p.revision,
            p.snapshot.config.is_some(),
            p.snapshot.config.as_ref().map_or(0, |c| c.apps.len()),
            p.snapshot.routing.freshness,
            p.snapshot.routing.generation,
            p.snapshot.routing.vpn_interfaces.join(","),
            p.snapshot.routing.last_good_vpn_interfaces.join(","),
            p.snapshot.routing.netlink_candidates.join(","),
            p.snapshot.routing.framework.status,
            p.snapshot.routing.uid.as_ref().map(|u| u.state),
            p.snapshot.backend.active,
            p.snapshot.backend.selected,
            p.snapshot.backend.applied,
            p.snapshot.backend.installed,
        ),
        Err(e) => {
            eprintln!("dump: read failed: {e}");
            std::process::exit(1);
        }
    }
}
