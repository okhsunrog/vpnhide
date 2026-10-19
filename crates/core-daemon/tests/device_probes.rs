//! The diagnostics probes run as core's own child (`vpnhide-core probe …`), in Rust.
use std::process::Command;

use vpnhide_core_daemon::diagnostics::{prefixes, sections};

fn probe(args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_vpnhide-core"))
        .arg("probe")
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[test]
fn root_snapshot_child_frames_sections_and_timings() {
    let (code, text) = probe(&["root-snapshot"]);
    assert_eq!(code, 0);
    let s = sections(&text, prefixes::ROOT_BEGIN, prefixes::ROOT_END);
    for name in [
        "kmod_prop",
        "current_boot_id",
        "snapshot_shell_uid",
        "ports_chain",
    ] {
        assert!(s.contains_key(name), "missing {name}");
    }
    assert!(text.contains(prefixes::ROOT_TIMING));
    assert!(
        !s.contains_key("pm_packages"),
        "no inventory without --packages"
    );
}

#[test]
fn hook_counters_child_uses_debug_framing() {
    let (code, text) = probe(&["hook-counters"]);
    assert_eq!(code, 0);
    let s = sections(&text, prefixes::DEBUG_BEGIN, prefixes::DEBUG_END);
    assert!(s.contains_key("kmod_state") && s.contains_key("kpm_state"));
}

#[test]
fn unexpected_probe_arguments_are_refused() {
    assert_eq!(probe(&["root-snapshot", "--evil"]).0, 2);
    assert_eq!(probe(&["kernel-images"]).0, 2);
}
