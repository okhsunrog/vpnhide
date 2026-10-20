use std::fs;
use std::path::Path;
use std::time::Duration;

use super::*;
use crate::diagnostics::{prefixes, sections};

fn tree(root: &Path) -> Paths {
    Paths {
        modules: root.join("modules"),
        modules_update: root.join("modules_update"),
        config_file: root.join("config.json"),
        superkey_file: root.join("superkey"),
        core_boot_status: root.join("boot_status"),
        core_native_boot: root.join("native_boot.json"),
        core_log: root.join("core.log"),
        zygisk_status: root.join("zygisk_status"),
        proc_ctl: root.join("ctl"),
        proc_diag: root.join("diag"),
        lsposed_state: root.join("lsposed_state"),
        boot_id: root.join("boot_id"),
        apatch_dir: root.join("ap"),
        legacy: vec![("legacy_kmod_targets", root.join("legacy_kmod"))],
        core_exe: root.join("no-core"),
    }
}

#[test]
fn root_snapshot_frames_every_required_section_with_shell_compatible_values() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let paths = tree(root);
    let kmod = paths.module("vpnhide_kmod");
    fs::create_dir_all(&kmod).unwrap();
    fs::write(kmod.join("module.prop"), "id=vpnhide_kmod\nversion=v2\n").unwrap();
    fs::write(kmod.join("vpnhide_kmod.ko"), "ko").unwrap();
    fs::write(kmod.join("disable"), "").unwrap();
    fs::create_dir_all(paths.modules_update.join("vpnhide_kpm")).unwrap();
    fs::write(
        paths.modules_update.join("vpnhide_kpm/module.prop"),
        "id=vpnhide_kpm\n",
    )
    .unwrap();
    fs::write(&paths.config_file, "{\"apps\":{}}").unwrap();
    fs::write(&paths.superkey_file, "key\n").unwrap();
    fs::write(&paths.boot_id, "abc-123\n").unwrap();
    fs::write(&paths.proc_ctl, "vpnhide 1 status\nbackend 0x4\n").unwrap();
    fs::write(root.join("legacy_kmod"), "com.example\n").unwrap();
    let lsposed = paths.modules.join("zygisk_lsposed");
    fs::create_dir_all(&lsposed).unwrap();
    fs::write(lsposed.join("module.prop"), "id=zygisk_lsposed\n").unwrap();

    for id in ["vpnhide_ports", "vpnhide_builtin"] {
        fs::create_dir_all(paths.module(id)).unwrap();
    }
    fs::write(paths.module("vpnhide_builtin").join("remove"), "").unwrap();

    let out = root_snapshot::collect(&paths, false);
    assert!(!out.truncated);
    let s = sections(&out.buf, prefixes::ROOT_BEGIN, prefixes::ROOT_END);
    let get = |k: &str| {
        s.get(k)
            .unwrap_or_else(|| panic!("missing {k}: {:?}", s.keys()))
            .clone()
    };
    assert_eq!(get("kmod_prop"), "id=vpnhide_kmod\nversion=v2");
    assert_eq!(get("kmod_module_dir"), "1");
    assert_eq!(get("kpm_module_dir"), "0");
    assert_eq!(get("kmod_artifact_state"), "present");
    assert_eq!(get("kpm_artifact_state"), "missing");
    assert_eq!(get("zygisk_artifact_state"), "missing");
    assert_eq!(get("kmod_disabled"), "1");
    assert_eq!(get("core_disabled"), "0");
    assert_eq!(get("kpm_pending_update"), "1");
    assert_eq!(get("kmod_pending_update"), "0");
    // Ports is still installed; Built-in is already marked for removal.
    assert_eq!(get("obsolete_modules"), "vpnhide_ports");
    assert_eq!(get("canonical_config"), "{\"apps\":{}}");
    assert_eq!(get("superkey_saved"), "1");
    assert_eq!(get("legacy_kmod_targets"), "com.example");
    assert_eq!(get("current_boot_id"), "abc-123");
    assert_eq!(get("kmod_state"), "vpnhide 1 status\nbackend 0x4");
    assert_eq!(get("proc_exists"), "1");
    assert_eq!(get("kpm_state"), "");
    assert!(get("kpatch_runtime").starts_with("apatch_dir=0\n"));
    assert!(["Enforcing", "Permissive", "Disabled"].contains(&get("getenforce").as_str()));
    assert!(!get("kernel_release").is_empty());
    let ident = get("snapshot_shell_uid");
    assert!(ident.starts_with(&format!("uid={}\n", vpnhide_activator::sys::uid())));
    assert!(ident.contains("errno_ctl=ok"));
    let ports = get("ports_chain");
    assert!(
        ports == "0" || ports == "1" || ports.starts_with("error="),
        "{ports}"
    );
    assert_eq!(
        get("lsposed_framework"),
        "installed=1\ndisabled=0\nprobe_ok=1"
    );
    assert_eq!(get("zygisk_status"), "");
    // The per-backend load_status files are gone: core publishes the typed
    // native decision over RPC instead.
    for name in ["kmod_load_status", "kpm_load_status", "core_boot_status"] {
        assert!(!s.contains_key(name), "{name}");
    }
    assert!(
        out.buf
            .contains(&format!("{}module_props=", prefixes::ROOT_TIMING))
    );
    assert!(
        !out.buf.contains(prefixes::PM_USER_BEGIN),
        "no package scan"
    );
}

#[test]
fn an_absent_control_node_reports_enoent() {
    let dir = tempfile::tempdir().unwrap();
    let paths = tree(dir.path());
    let out = root_snapshot::collect(&paths, false);
    let s = sections(&out.buf, prefixes::ROOT_BEGIN, prefixes::ROOT_END);
    assert!(s["snapshot_shell_uid"].contains("errno_ctl=enoent"));
    assert_eq!(s["proc_exists"], "0");
    assert_eq!(s["kmod_state"], "");
}

#[test]
fn package_inventory_never_uses_the_all_users_shortcut() {
    let args = inventory::packages_args(10);
    assert_eq!(args, ["list", "packages", "-U", "-f", "--user", "10"]);
    assert!(!args.iter().any(|a| a == "--all-users" || a == "-a"));
}

#[test]
fn user_ids_merge_plain_and_verbose_listings() {
    let plain = "Users:\n\tUserInfo{0:Owner:c13} running\n\tUserInfo{10:Work:1030} running\n";
    let verbose = "Users:\n  0: id=0, name=Owner\n  1: id=11, name=Clone, flags=x\n";
    assert_eq!(inventory::user_ids(plain, verbose), vec![0, 10, 11]);
    assert_eq!(inventory::user_ids("", ""), vec![0]);
}

#[test]
fn debug_snapshot_redacts_identifiers() {
    assert_eq!(
        debug_snapshot::redact_cmdline(
            "a androidboot.serialno=XYZ androidboot.wifi_macaddr=11:22 b\n"
        ),
        "a androidboot.serialno=<redacted> androidboot.wifi_macaddr=<redacted> b\n"
    );
    assert_eq!(
        debug_snapshot::redact_user_name("\tUserInfo{10:Work profile:1030} running"),
        "\tUserInfo{10:<name>:1030} running"
    );
}

#[test]
fn hook_counters_use_debug_framing_and_mark_missing_nodes() {
    let dir = tempfile::tempdir().unwrap();
    let paths = tree(dir.path());
    let out = debug_snapshot::hook_counters(&paths);
    let s = sections(&out.buf, prefixes::DEBUG_BEGIN, prefixes::DEBUG_END);
    assert_eq!(
        s["kmod_state"],
        format!("(missing: {})", paths.proc_ctl.display())
    );
    assert!(
        s["kpm_state"].ends_with("[exit=127]"),
        "{:?}",
        s["kpm_state"]
    );
}

#[test]
fn debug_snapshot_frames_module_and_network_sections() {
    let dir = tempfile::tempdir().unwrap();
    let paths = tree(dir.path());
    let core = paths.module("vpnhide_core");
    fs::create_dir_all(&core).unwrap();
    fs::write(core.join("vpnhide-core"), "bin").unwrap();
    let out = debug_snapshot::collect(&paths);
    let s = sections(&out.buf, prefixes::DEBUG_BEGIN, prefixes::DEBUG_END);
    for name in [
        "uname",
        "core_module_state",
        "kmod_state",
        "kpatch_runtime",
        "network_operstate",
        "proc_net_route",
        "pm_users",
        "pm_packages",
        "app_scan_diagnostics",
    ] {
        assert!(s.contains_key(name), "missing {name}");
    }
    let state = &s["core_module_state"];
    assert!(
        state.contains("installed=1\ndisabled=0\nremove=0\nupdate=0\n"),
        "{state}"
    );
    // sha256("bin")
    assert!(
        state.contains("51a1f05af85e342e3c849b47d387086476282d5f50dc240c19216d6edfb1eb5a"),
        "{state}"
    );
    assert!(state.ends_with("staged=0"), "{state}");
}

#[test]
fn kernel_images_gzip_each_partition_and_write_a_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let by_name = dir.path().join("by-name");
    let out_dir = dir.path().join("out");
    fs::create_dir_all(&by_name).unwrap();
    fs::create_dir_all(&out_dir).unwrap();
    fs::write(by_name.join("boot_a"), vec![7u8; 4096]).unwrap();
    let (code, _) = kernel::images_from(&by_name, "_a", &out_dir);
    assert_eq!(code, 0);
    let gz = out_dir.join("boot_a.img.gz");
    let mut text = Vec::new();
    use std::io::Read;
    flate2::read::GzDecoder::new(fs::File::open(&gz).unwrap())
        .read_to_end(&mut text)
        .unwrap();
    assert_eq!(text, vec![7u8; 4096]);
    let manifest = fs::read_to_string(out_dir.join("manifest.txt")).unwrap();
    assert!(manifest.contains("### boot\npresent=1\n"));
    assert!(manifest.contains("### init_boot\npresent=0\n"));
    assert!(manifest.contains("file=images/boot_a.img.gz"));
    let (code, _) = kernel::images_from(&dir.path().join("none"), "", &out_dir);
    assert_eq!(code, 1, "no image captured");
}

#[test]
fn lsposed_db_copy_includes_the_wal_when_present() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("modules_config.db");
    fs::write(&src, "db").unwrap();
    fs::write(dir.path().join("modules_config.db-wal"), "wal").unwrap();
    let out_dir = dir.path().join("out");
    fs::create_dir_all(&out_dir).unwrap();
    assert_eq!(lsposed::copy_db_from(&src, &out_dir).0, 0);
    assert_eq!(
        fs::read_to_string(out_dir.join("modules_config.db-wal")).unwrap(),
        "wal"
    );
    assert!(!out_dir.join("modules_config.db-shm").exists());
    assert_eq!(lsposed::copy_db_from(&dir.path().join("x"), &out_dir).0, 1);
}

#[test]
fn bounded_tools_report_missing_binaries_and_timeouts() {
    assert_eq!(
        run("/nonexistent/tool", &[], Duration::from_secs(1)).exit,
        127
    );
    let started = std::time::Instant::now();
    assert_eq!(run("sleep", &["30"], Duration::from_millis(200)).exit, -1);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn ls_line_shows_mode_owner_and_path() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("f");
    fs::write(&file, "12345").unwrap();
    let line = ls_line(&file);
    assert!(line.starts_with("-rw"), "{line}");
    assert!(line.contains(" 5 "), "{line}");
    assert!(line.ends_with(&file.display().to_string()), "{line}");
}

/// Anti-drift guard against the app's parser: every section `RootSnapshotCache` requires
/// is actually emitted (a missing one makes the whole snapshot fail validation).
#[test]
fn root_snapshot_emits_every_section_the_app_requires() {
    let kotlin = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/RootSnapshotCache.kt"
    ))
    .unwrap();
    let block = kotlin
        .split("REQUIRED_ROOT_SNAPSHOT_SECTIONS =")
        .nth(1)
        .and_then(|rest| rest.split(')').next())
        .unwrap();
    let required: Vec<&str> = block.split('"').skip(1).step_by(2).collect();
    assert!(required.len() > 30, "{required:?}");
    let dir = tempfile::tempdir().unwrap();
    let out = root_snapshot::collect(&tree(dir.path()), true);
    let s = sections(&out.buf, prefixes::ROOT_BEGIN, prefixes::ROOT_END);
    let missing: Vec<&&str> = required.iter().filter(|r| !s.contains_key(**r)).collect();
    assert!(missing.is_empty(), "missing {missing:?}");
}

/// Same guard for the bug-report sections the app and humans rely on.
#[test]
fn debug_snapshot_covers_every_backend_state_path() {
    let dir = tempfile::tempdir().unwrap();
    let paths = tree(dir.path());
    let out = debug_snapshot::collect(&paths);
    let s = sections(&out.buf, prefixes::DEBUG_BEGIN, prefixes::DEBUG_END);
    for name in [
        "core_prop",
        "core_module_state",
        "core_boot_status",
        "core_native_boot",
        "core_log",
        "kmod_module_state",
        "kmod_state",
        "kmod_diag",
        "kpm_module_state",
        "kpm_state",
        "zygisk_module_state",
        "zygisk_status",
        "ports_state",
        "lsposed_state",
        "lsposed_files",
        "connectivity_dump",
        "proc_cmdline",
        "legacy_kmod_targets",
    ] {
        assert!(s.contains_key(name), "missing {name}");
    }
    assert!(s["kpm_module_state"].contains("vpnhide.kpm"));
    assert!(s["zygisk_module_state"].contains("zygisk/arm64-v8a.so"));
    assert!(!s.contains_key("debug_logging"));
}
