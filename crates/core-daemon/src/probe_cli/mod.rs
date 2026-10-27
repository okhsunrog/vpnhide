//! `vpnhide-core probe …` / `vpnhide-core observe …`: one-shot root probes.
//!
//! Core runs these as a child of itself (see `diagnostics`), so a probe that hangs or panics
//! is bounded and killed without touching the daemon. JSON reports and framed diagnostic
//! sections retain the formats shared with the app (`fixtures/core-probes`).

pub(crate) mod observations;
mod probe;

/// Dispatch a probe subcommand. `None` when `args` is not one (the caller runs the daemon).
pub fn run(args: &[String]) -> Option<i32> {
    let code = match args {
        [command, mode] if command == "probe" && mode == "checks" => probe::checks(&[]),
        [command, mode, rest @ ..] if command == "probe" && mode == "routing" => {
            probe::routing(rest)
        }
        [command, mode, rest @ ..] if command == "observe" && mode == "app-vpn-state" => {
            probe::app_vpn_state(rest)
        }
        [command, mode] if command == "observe" && mode == "kpm-state" => observations::kpm_state(),
        [command, mode, rest @ ..] if command == "probe" && is_device_probe(mode) => {
            device_probe(mode, rest)
        }
        [command, ..] if command == "probe" || command == "observe" => {
            eprintln!(
                "usage: vpnhide-core probe checks | probe root-snapshot [--packages] | probe debug-snapshot | probe hook-counters | probe kernel-partitions | probe kernel-images <dir> | probe lsposed-db <dir> | probe logcat-lsposed | probe routing --uid <uid> [--vpn-ifaces <list>] | observe app-vpn-state --uid <uid> | observe kpm-state"
            );
            2
        }
        _ => return None,
    };
    Some(code)
}

const DEVICE_PROBES: &[&str] = &[
    "root-snapshot",
    "debug-snapshot",
    "hook-counters",
    "kernel-partitions",
    "kernel-images",
    "lsposed-db",
    "logcat-lsposed",
];

fn is_device_probe(mode: &str) -> bool {
    DEVICE_PROBES.contains(&mode)
}

/// The diagnostics probes (`device_probes`), each run as core's own bounded child.
fn device_probe(mode: &str, rest: &[String]) -> i32 {
    use crate::device_probes::{Paths, debug_snapshot, kernel, lsposed, root_snapshot};
    use std::io::Write;
    let mut paths = Paths::default();
    let rest = if rest
        .last()
        .is_some_and(|arg| arg == crate::device_probes::KPM_STDIN_FLAG)
    {
        if !matches!(mode, "root-snapshot" | "debug-snapshot" | "hook-counters") {
            eprintln!("probe inputs are not supported for {mode}");
            return 2;
        }
        let inputs = match crate::device_probes::ProbeInputs::read(std::io::stdin().lock()) {
            Ok(inputs) => inputs,
            Err(error) => {
                eprintln!("invalid probe input: {error}");
                return 2;
            }
        };
        paths.kpm_observation = Some(inputs.kpm);
        &rest[..rest.len() - 1]
    } else {
        rest
    };
    let out_dir = || rest.first().map(std::path::PathBuf::from);
    let (code, text) = match (mode, rest) {
        ("root-snapshot", []) => (0, root_snapshot::collect(&paths, false).buf),
        ("root-snapshot", [flag]) if flag == "--packages" => {
            (0, root_snapshot::collect(&paths, true).buf)
        }
        ("debug-snapshot", []) => (0, debug_snapshot::collect(&paths).buf),
        ("hook-counters", []) => (0, debug_snapshot::hook_counters(&paths).buf),
        ("kernel-partitions", []) => kernel::partitions(),
        ("logcat-lsposed", []) => lsposed::boot_log(),
        ("kernel-images", [_]) => kernel::images(&out_dir().expect("dir")),
        ("lsposed-db", [_]) => lsposed::copy_db(&out_dir().expect("dir")),
        _ => {
            eprintln!("usage: vpnhide-core probe {mode}: unexpected arguments");
            return 2;
        }
    };
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(text.as_bytes());
    let _ = stdout.flush();
    code
}
