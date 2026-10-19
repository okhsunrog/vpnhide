//! `collect_diagnostics`: the fixed set of root probes the app may ask for (protocol v5).
//!
//! The app runs no root process. Every root read it needs is a [`DiagnosticProbe`]; each maps
//! to a script compiled into this binary (`probes/shell/*.sh`), a fixed `logcat`/`dmesg`
//! invocation, or a `vpnhide-core probe …` child. No caller-supplied command, script or path
//! exists. Probes run concurrently, each bounded in time and output.
//!
//! Output up to [`INLINE_MAX`] travels inline; anything larger (debug snapshot, logcat) and
//! every artifact (kernel images, LSPosed db copies) is written to the export dir inside the
//! app's own data dir, owned by the app, so the reply stays far below the transport frame.

use std::collections::BTreeMap;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use vpnhide_core_rpc::{
    DiagnosticProbe, DiagnosticsReport, DiagnosticsRequest, PROBE_MAX_RUNTIME, ProbeOutput,
};

/// Inline output cap per probe; larger output goes to a file in the export dir.
pub const INLINE_MAX: usize = 64 * 1024;
/// Hard cap on any probe's captured stdout.
pub const OUTPUT_MAX: usize = 32 * 1024 * 1024;
/// Total inline bytes per report, well inside the 1 MiB frame.
const REPORT_INLINE_BUDGET: usize = 640 * 1024;
/// The export subdir (inside the app data dir) core writes probe files to.
pub const EXPORT_DIR_NAME: &str = "vpnhide-core-export";

const PACKAGE_INVENTORY: &str = include_str!("../probes/shell/package_inventory.sh");
const ROOT_SNAPSHOT: &str = include_str!("../probes/shell/root_snapshot.sh");
const DEBUG_SNAPSHOT: &str = include_str!("../probes/shell/debug_snapshot.sh");
const HOOK_COUNTERS: &str = include_str!("../probes/shell/hook_counters.sh");
const KERNEL_PARTITIONS: &str = include_str!("../probes/shell/kernel_partitions.sh");
const KERNEL_IMAGES: &str = include_str!("../probes/shell/kernel_images.sh");
const LSPOSED_DB: &str = include_str!("../probes/shell/lsposed_db.sh");
const LOGCAT_LSPOSED_BOOT: &str = include_str!("../probes/shell/logcat_lsposed_boot.sh");

/// Section framing the app's parsers expect (`RootSnapshotCache` / `DebugShellSnapshot` /
/// `PackageInventoryData`). Shared fixture: `fixtures/core-probes/prefixes.json`.
pub mod prefixes {
    pub const ROOT_BEGIN: &str = "__VPNHIDE_ROOT_SECTION_BEGIN__:";
    pub const ROOT_END: &str = "__VPNHIDE_ROOT_SECTION_END__:";
    pub const ROOT_TIMING: &str = "__VPNHIDE_ROOT_TIMING__:";
    pub const DEBUG_BEGIN: &str = "__VPNHIDE_DEBUG_SECTION_BEGIN__:";
    pub const DEBUG_END: &str = "__VPNHIDE_DEBUG_SECTION_END__:";
    pub const PM_USERS_STATUS: &str = "__VPNHIDE_PM_USERS_STATUS__:";
    pub const PM_USER_BEGIN: &str = "__VPNHIDE_PM_USER_BEGIN__:";
    pub const PM_USER_END: &str = "__VPNHIDE_PM_USER_END__:";
}

/// The app's own log tags (`LogTags`) plus the native components', for `LogcatAppTags`.
const APP_LOG_TAGS: &[&str] = &[
    "VPNHideTest:*",
    "VpnHide:*",
    "VpnHide-Dashboard:*",
    "VpnHide-Startup:*",
    "VpnHide-LSPosed:*",
    "VpnHide-Diag:*",
    "VpnHide-Logcat:*",
    "VpnHide-Update:*",
    "VpnHideAgentBridge:*",
    "vpnhide:*",
    "vpnhide_ports:*",
    "vpnhide-zygisk:*",
    "shadowhook_tag:*",
];

/// Where probes run and where they may write. Production: `from_env`.
#[derive(Debug, Clone)]
pub struct ProbeEnv {
    pub shell: PathBuf,
    /// This binary, for `vpnhide-core probe …` children.
    pub core_exe: PathBuf,
    /// The app data dir (export dir parent, artifact owner). `None`: no file outputs.
    pub app_dir: Option<PathBuf>,
}

impl ProbeEnv {
    pub fn from_env() -> Self {
        let shell = ["/system/bin/sh", "/bin/sh"]
            .iter()
            .map(PathBuf::from)
            .find(|p| p.exists())
            .unwrap_or_else(|| PathBuf::from("sh"));
        ProbeEnv {
            shell,
            core_exe: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("vpnhide-core")),
            app_dir: crate::app_socket::app_data_dir_from_env(),
        }
    }

    fn export_dir(&self) -> Option<PathBuf> {
        self.app_dir.as_ref().map(|d| d.join(EXPORT_DIR_NAME))
    }
}

/// Fixed script inputs: module dirs, state files, framing. Mirrors what the app used to pass.
fn snapshot_vars(begin: &str, end: &str, stderr_to_stdout: bool) -> Vec<(&'static str, String)> {
    let legacy = [
        ("legacy_kmod_targets", "/data/adb/vpnhide_kmod/targets.txt"),
        ("legacy_kpm_targets", "/data/adb/vpnhide_kpm/targets.txt"),
        (
            "legacy_zygisk_targets",
            "/data/adb/vpnhide_zygisk/targets.txt",
        ),
        (
            "legacy_lsposed_targets",
            "/data/adb/vpnhide_lsposed/targets.txt",
        ),
        (
            "legacy_ports_observers",
            "/data/adb/vpnhide_ports/observers.txt",
        ),
        ("legacy_hidden_pkgs", "/data/system/vpnhide_hidden_pkgs.txt"),
        (
            "legacy_observer_uids",
            "/data/system/vpnhide_observer_uids.txt",
        ),
    ]
    .iter()
    .map(|(name, path)| format!("{name}={path}"))
    .collect::<Vec<_>>()
    .join(" ");
    let fixed: &[(&'static str, &str)] = &[
        ("VPNHIDE_PM_USERS_STATUS", prefixes::PM_USERS_STATUS),
        ("VPNHIDE_PM_USER_BEGIN", prefixes::PM_USER_BEGIN),
        ("VPNHIDE_PM_USER_END", prefixes::PM_USER_END),
        ("VPNHIDE_KMOD_DIR", "/data/adb/modules/vpnhide_kmod"),
        ("VPNHIDE_BUILTIN_DIR", "/data/adb/modules/vpnhide_builtin"),
        ("VPNHIDE_KPM_DIR", "/data/adb/modules/vpnhide_kpm"),
        ("VPNHIDE_ZYGISK_DIR", "/data/adb/modules/vpnhide_zygisk"),
        ("VPNHIDE_PORTS_DIR", "/data/adb/modules/vpnhide_ports"),
        (
            "VPNHIDE_KMOD_ACTIVATOR",
            "/data/adb/modules/vpnhide_kmod/activator",
        ),
        (
            "VPNHIDE_BUILTIN_ACTIVATOR",
            "/data/adb/modules/vpnhide_builtin/activator",
        ),
        (
            "VPNHIDE_KPM_ACTIVATOR",
            "/data/adb/modules/vpnhide_kpm/activator",
        ),
        (
            "VPNHIDE_ZYGISK_ACTIVATOR",
            "/data/adb/modules/vpnhide_zygisk/activator",
        ),
        (
            "VPNHIDE_PORTS_ACTIVATOR",
            "/data/adb/modules/vpnhide_ports/activator",
        ),
        ("VPNHIDE_CONFIG_FILE", "/data/system/vpnhide_config.json"),
        ("VPNHIDE_SUPERKEY_FILE", crate::effects::SUPERKEY_FILE),
        (
            "VPNHIDE_KMOD_LOAD_STATUS",
            "/data/adb/vpnhide_kmod/load_status",
        ),
        (
            "VPNHIDE_KMOD_LOAD_DMESG",
            "/data/adb/vpnhide_kmod/load_dmesg",
        ),
        (
            "VPNHIDE_BUILTIN_LOAD_STATUS",
            "/data/adb/vpnhide_builtin/load_status",
        ),
        (
            "VPNHIDE_ZYGISK_STATUS",
            "/data/user/0/dev.okhsunrog.vpnhide/files/vpnhide_zygisk_active",
        ),
        (
            "VPNHIDE_KPM_LOAD_STATUS",
            "/data/adb/vpnhide_kpm/load_status",
        ),
        (
            "VPNHIDE_PORTS_LOAD_STATUS",
            "/data/adb/vpnhide_ports/load_status",
        ),
        ("VPNHIDE_PORTS_LOAD_LOG", "/data/adb/vpnhide_ports/load_log"),
        ("VPNHIDE_PROC_CTL", "/proc/vpnhide_ctl"),
        ("VPNHIDE_PROC_DIAG", "/proc/vpnhide_diag"),
        (
            "VPNHIDE_LSPOSED_STATE",
            "/data/system/vpnhide_lsposed_state",
        ),
        (
            "VPNHIDE_LEGACY_HOOK_STATUS",
            "/data/system/vpnhide_hook_active",
        ),
    ];
    let mut vars: Vec<(&'static str, String)> =
        fixed.iter().map(|(k, v)| (*k, (*v).to_owned())).collect();
    vars.push(("VPNHIDE_SECTION_BEGIN", begin.to_owned()));
    vars.push(("VPNHIDE_SECTION_END", end.to_owned()));
    vars.push((
        "VPNHIDE_PM_STDERR_TO_STDOUT",
        if stderr_to_stdout { "1" } else { "0" }.to_owned(),
    ));
    vars.push(("VPNHIDE_LEGACY_SECTIONS", legacy));
    vars
}

/// What one probe run produced, before placement (inline vs file).
#[derive(Debug, Default)]
struct Captured {
    exit: i32,
    stdout: Vec<u8>,
    truncated: bool,
    artifacts: Vec<PathBuf>,
}

/// Run every requested probe concurrently and assemble the report. Blocking; call from a
/// blocking context.
pub fn collect(env: &ProbeEnv, request: &DiagnosticsRequest) -> DiagnosticsReport {
    let dmesg_cleared = request.clear_dmesg && clear_dmesg();
    let captured: Vec<(DiagnosticProbe, Captured)> = std::thread::scope(|scope| {
        let handles: Vec<_> = request
            .probes
            .iter()
            .map(|probe| (probe.clone(), scope.spawn(|| run_probe(env, probe))))
            .collect();
        handles
            .into_iter()
            .map(|(probe, h)| {
                let out = h.join().unwrap_or(Captured {
                    exit: -1,
                    ..Captured::default()
                });
                (probe, out)
            })
            .collect()
    });
    let mut budget = REPORT_INLINE_BUDGET;
    let outputs = captured
        .into_iter()
        .map(|(probe, c)| place(env, probe, c, &mut budget))
        .collect();
    DiagnosticsReport {
        outputs,
        dmesg_cleared,
    }
}

fn clear_dmesg() -> bool {
    Command::new("dmesg")
        .arg("-c")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn run_probe(env: &ProbeEnv, probe: &DiagnosticProbe) -> Captured {
    let shell = |script: String, vars: Vec<(&'static str, String)>, timeout: Duration| {
        let mut cmd = Command::new(&env.shell);
        cmd.arg("-c").arg(script).envs(vars);
        run_bounded(cmd, timeout)
    };
    match probe {
        DiagnosticProbe::RootSnapshot { packages } => {
            let mut vars = snapshot_vars(prefixes::ROOT_BEGIN, prefixes::ROOT_END, false);
            vars.push(("VPNHIDE_TIMING", prefixes::ROOT_TIMING.to_owned()));
            vars.push((
                "VPNHIDE_WITH_PM",
                if *packages { "1" } else { "0" }.to_owned(),
            ));
            vars.push(("VPNHIDE_KPM_PROBE_PATH", env.core_exe.display().to_string()));
            shell(
                format!("{PACKAGE_INVENTORY}\n{ROOT_SNAPSHOT}"),
                vars,
                Duration::from_secs(30),
            )
        }
        DiagnosticProbe::DebugSnapshot => shell(
            format!("{PACKAGE_INVENTORY}\n{DEBUG_SNAPSHOT}"),
            snapshot_vars(prefixes::DEBUG_BEGIN, prefixes::DEBUG_END, true),
            Duration::from_secs(120),
        ),
        DiagnosticProbe::HookCounters => shell(
            HOOK_COUNTERS.to_owned(),
            snapshot_vars(prefixes::DEBUG_BEGIN, prefixes::DEBUG_END, true),
            Duration::from_secs(8),
        ),
        DiagnosticProbe::Dmesg => run_bounded(Command::new("dmesg"), Duration::from_secs(15)),
        DiagnosticProbe::LogcatAppTags => {
            let mut cmd = Command::new("logcat");
            cmd.args(["-d", "-b", "all", "-v", "threadtime", "-s"])
                .args(APP_LOG_TAGS);
            run_bounded(cmd, Duration::from_secs(30))
        }
        DiagnosticProbe::LogcatLsposedBoot => shell(
            LOGCAT_LSPOSED_BOOT.to_owned(),
            Vec::new(),
            Duration::from_secs(15),
        ),
        DiagnosticProbe::LogcatSince { since } => {
            if !valid_logcat_time(since) {
                return Captured {
                    exit: -1,
                    stdout: b"invalid since".to_vec(),
                    ..Captured::default()
                };
            }
            let mut cmd = Command::new("logcat");
            cmd.args(["-d", "-b", "all", "-v", "threadtime", "-T", since]);
            run_bounded(cmd, Duration::from_secs(60))
        }
        DiagnosticProbe::KernelPartitions => shell(
            KERNEL_PARTITIONS.to_owned(),
            Vec::new(),
            Duration::from_secs(30),
        ),
        DiagnosticProbe::KernelImages => with_out_dir(env, "kernel-images", |dir| {
            shell(
                KERNEL_IMAGES.to_owned(),
                vec![("VPNHIDE_OUT_DIR", dir.display().to_string())],
                PROBE_MAX_RUNTIME,
            )
        }),
        DiagnosticProbe::LsposedModulesDb => with_out_dir(env, "lsposed-db", |dir| {
            shell(
                LSPOSED_DB.to_owned(),
                vec![("VPNHIDE_OUT_DIR", dir.display().to_string())],
                Duration::from_secs(15),
            )
        }),
        DiagnosticProbe::GroundTruthChecks => {
            let mut cmd = Command::new(&env.core_exe);
            cmd.args(["probe", "checks"]);
            run_bounded(cmd, Duration::from_secs(20))
        }
        DiagnosticProbe::AppVpnState { uid } => {
            let mut cmd = Command::new(&env.core_exe);
            cmd.args(["observe", "app-vpn-state", "--uid"])
                .arg(uid.to_string());
            run_bounded(cmd, Duration::from_secs(20))
        }
        DiagnosticProbe::KpmList => {
            let mut cmd = Command::new(&env.core_exe);
            cmd.args(["observe", "kpm-list"]);
            run_bounded(cmd, Duration::from_secs(10))
        }
    }
}

/// `MM-DD HH:MM:SS.mmm` (logcat's threadtime stamp) and nothing else.
fn valid_logcat_time(raw: &str) -> bool {
    let b = raw.as_bytes();
    b.len() == 18
        && b.iter().enumerate().all(|(i, c)| match i {
            2 => *c == b'-',
            5 => *c == b' ',
            8 | 11 => *c == b':',
            14 => *c == b'.',
            _ => c.is_ascii_digit(),
        })
}

/// Give a file-producing probe a fresh, private dir under the export dir, then hand its
/// contents to the app.
fn with_out_dir(env: &ProbeEnv, name: &str, run: impl FnOnce(&Path) -> Captured) -> Captured {
    let Some(export) = env.export_dir() else {
        return Captured {
            exit: -1,
            stdout: b"no app data dir".to_vec(),
            ..Captured::default()
        };
    };
    let dir = export.join(name);
    let _ = std::fs::remove_dir_all(&dir);
    if let Err(e) = prepare_dir(env, &export).and_then(|()| prepare_dir(env, &dir)) {
        return Captured {
            exit: -1,
            stdout: format!("export dir: {e}").into_bytes(),
            ..Captured::default()
        };
    }
    let mut out = run(&dir);
    let mut artifacts: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|it| it.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    artifacts.sort();
    for path in &artifacts {
        hand_to_app(env, path, 0o600);
    }
    out.artifacts = artifacts;
    out
}

fn prepare_dir(env: &ProbeEnv, dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    hand_to_app(env, dir, 0o700);
    Ok(())
}

/// Owner = the app data dir's owner, `mode`, and the dir's SELinux label — the same
/// treatment as the socket, so only the app can read what core exported for it.
fn hand_to_app(env: &ProbeEnv, path: &Path, mode: u32) {
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
    if let Some(app_dir) = &env.app_dir {
        if let Err(e) = crate::auth::hand_to_owner_of(path, app_dir) {
            eprintln!("core: export ownership {}: {e}", path.display());
        }
    }
}

/// Spawn, drain stdout up to [`OUTPUT_MAX`], and kill on `timeout`. stderr is discarded.
fn run_bounded(mut cmd: Command, timeout: Duration) -> Captured {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(_) => {
            return Captured {
                exit: -1,
                ..Captured::default()
            };
        }
    };
    let mut stdout = child.stdout.take().expect("piped");
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let mut truncated = false;
        let mut chunk = [0u8; 64 * 1024];
        loop {
            match stdout.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let room = OUTPUT_MAX.saturating_sub(buf.len());
                    if n > room {
                        buf.extend_from_slice(&chunk[..room]);
                        truncated = true;
                    } else {
                        buf.extend_from_slice(&chunk[..n]);
                    }
                }
            }
        }
        (buf, truncated)
    });
    let deadline = Instant::now() + timeout;
    let (exit, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (status.code().unwrap_or(-1), false),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break (-1, true);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => break (-1, false),
        }
    };
    // A grandchild holding stdout open (e.g. a backgrounded pipeline) must not hang us.
    let (stdout, truncated) = join_with_deadline(reader, Duration::from_secs(2));
    Captured {
        exit,
        stdout,
        truncated: truncated || timed_out,
        artifacts: Vec::new(),
    }
}

fn join_with_deadline(
    handle: std::thread::JoinHandle<(Vec<u8>, bool)>,
    wait: Duration,
) -> (Vec<u8>, bool) {
    let deadline = Instant::now() + wait;
    while !handle.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    if handle.is_finished() {
        handle.join().unwrap_or_default()
    } else {
        (Vec::new(), true)
    }
}

static FILE_SEQ: AtomicU64 = AtomicU64::new(0);

/// Inline small output (within the report budget); write the rest to the export dir.
fn place(env: &ProbeEnv, probe: DiagnosticProbe, c: Captured, budget: &mut usize) -> ProbeOutput {
    let text = String::from_utf8_lossy(&c.stdout).into_owned();
    let artifacts = c
        .artifacts
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    if text.len() <= INLINE_MAX && text.len() <= *budget {
        *budget -= text.len();
        return ProbeOutput {
            probe,
            exit: c.exit,
            output: text,
            file: None,
            artifacts,
            truncated: c.truncated,
        };
    }
    let file = env.export_dir().and_then(|export| {
        prepare_dir(env, &export).ok()?;
        let seq = FILE_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = export.join(format!("{}-{seq}.txt", probe_slug(&probe)));
        std::fs::write(&path, text.as_bytes()).ok()?;
        hand_to_app(env, &path, 0o600);
        Some(path.display().to_string())
    });
    match file {
        Some(file) => ProbeOutput {
            probe,
            exit: c.exit,
            output: String::new(),
            file: Some(file),
            artifacts,
            truncated: c.truncated,
        },
        // No export dir (host/dev): inline what fits and flag the cut.
        None => {
            let cap = INLINE_MAX.min(*budget);
            let mut end = cap.min(text.len());
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            *budget -= end;
            ProbeOutput {
                probe,
                exit: c.exit,
                output: text[..end].to_owned(),
                file: None,
                artifacts,
                truncated: true,
            }
        }
    }
}

fn probe_slug(probe: &DiagnosticProbe) -> &'static str {
    match probe {
        DiagnosticProbe::RootSnapshot { .. } => "root-snapshot",
        DiagnosticProbe::DebugSnapshot => "debug-snapshot",
        DiagnosticProbe::HookCounters => "hook-counters",
        DiagnosticProbe::Dmesg => "dmesg",
        DiagnosticProbe::LogcatAppTags => "logcat-app",
        DiagnosticProbe::LogcatLsposedBoot => "logcat-lsposed",
        DiagnosticProbe::LogcatSince { .. } => "logcat-since",
        DiagnosticProbe::KernelPartitions => "kernel-partitions",
        DiagnosticProbe::KernelImages => "kernel-images",
        DiagnosticProbe::LsposedModulesDb => "lsposed-db",
        DiagnosticProbe::GroundTruthChecks => "checks",
        DiagnosticProbe::AppVpnState { .. } => "app-vpn-state",
        DiagnosticProbe::KpmList => "kpm-list",
    }
}

/// Section map of framed output, for tests and tooling.
pub fn sections(raw: &str, begin: &str, end: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut current: Option<(String, Vec<&str>)> = None;
    for line in raw.lines() {
        if let Some(name) = line.strip_prefix(begin) {
            current = Some((name.to_owned(), Vec::new()));
        } else if let Some(name) = line.strip_prefix(end) {
            if let Some((open, body)) = current.take() {
                if open == name {
                    out.insert(open, body.join("\n"));
                }
            }
        } else if let Some((_, body)) = current.as_mut() {
            body.push(line);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host_env(app_dir: Option<PathBuf>) -> ProbeEnv {
        ProbeEnv {
            shell: PathBuf::from("/bin/sh"),
            core_exe: PathBuf::from("/nonexistent/vpnhide-core"),
            app_dir,
        }
    }

    #[test]
    fn root_snapshot_frames_every_required_section() {
        let report = collect(
            &host_env(None),
            &DiagnosticsRequest {
                probes: vec![DiagnosticProbe::RootSnapshot { packages: false }],
                clear_dmesg: false,
            },
        );
        let out = &report.outputs[0];
        assert!(!out.truncated, "{out:?}");
        let s = sections(&out.output, prefixes::ROOT_BEGIN, prefixes::ROOT_END);
        for name in [
            "kmod_prop",
            "canonical_config",
            "current_boot_id",
            "kmod_state",
            "kpm_runtime_modules",
            "snapshot_shell_uid",
            "proc_exists",
            "ports_chain",
            "lsposed_framework",
            "legacy_kmod_targets",
        ] {
            assert!(s.contains_key(name), "missing {name}: {:?}", s.keys());
        }
        assert!(out.output.contains(prefixes::ROOT_TIMING));
        assert!(!report.dmesg_cleared);
    }

    #[test]
    fn hook_counters_and_debug_snapshot_use_debug_framing() {
        let report = collect(
            &host_env(None),
            &DiagnosticsRequest {
                probes: vec![DiagnosticProbe::HookCounters],
                clear_dmesg: false,
            },
        );
        assert!(report.outputs[0].output.contains(prefixes::DEBUG_BEGIN));
    }

    #[test]
    fn every_probe_script_is_valid_posix_sh() {
        // The device shell is mksh; a syntax slip kills a whole batched probe silently.
        for (name, script) in [
            (
                "package_inventory+root_snapshot",
                format!("{PACKAGE_INVENTORY}\n{ROOT_SNAPSHOT}"),
            ),
            (
                "package_inventory+debug_snapshot",
                format!("{PACKAGE_INVENTORY}\n{DEBUG_SNAPSHOT}"),
            ),
            ("hook_counters", HOOK_COUNTERS.to_owned()),
            ("kernel_partitions", KERNEL_PARTITIONS.to_owned()),
            ("kernel_images", KERNEL_IMAGES.to_owned()),
            ("lsposed_db", LSPOSED_DB.to_owned()),
            ("logcat_lsposed_boot", LOGCAT_LSPOSED_BOOT.to_owned()),
        ] {
            let status = Command::new("sh")
                .args(["-n", "-c", &script])
                .status()
                .unwrap();
            assert!(status.success(), "{name} does not parse");
        }
    }

    #[test]
    fn root_snapshot_without_packages_skips_the_inventory_scan() {
        let request = DiagnosticsRequest {
            probes: vec![DiagnosticProbe::RootSnapshot { packages: false }],
            clear_dmesg: false,
        };
        let report = collect(&host_env(None), &request);
        let text = &report.outputs[0].output;
        assert!(
            !text.contains("__VPNHIDE_PM_USER_BEGIN__:"),
            "no per-user package scan"
        );
    }

    #[test]
    fn logcat_since_accepts_only_a_timestamp() {
        assert!(valid_logcat_time("10-19 21:00:00.123"));
        for bad in [
            "",
            "10-19 21:00:00",
            "10-19 21:00:00.123; rm -rf /",
            "aa-19 21:00:00.123",
        ] {
            assert!(!valid_logcat_time(bad), "{bad}");
        }
        let report = collect(
            &host_env(None),
            &DiagnosticsRequest {
                probes: vec![DiagnosticProbe::LogcatSince {
                    since: "$(id)".into(),
                }],
                clear_dmesg: false,
            },
        );
        assert_eq!(report.outputs[0].exit, -1);
    }

    #[test]
    fn large_output_goes_to_an_app_owned_export_file() {
        let dir = tempfile::tempdir().unwrap();
        let env = host_env(Some(dir.path().to_path_buf()));
        let big = Captured {
            exit: 0,
            stdout: vec![b'x'; INLINE_MAX + 1],
            ..Captured::default()
        };
        let mut budget = REPORT_INLINE_BUDGET;
        let out = place(&env, DiagnosticProbe::Dmesg, big, &mut budget);
        let file = out.file.expect("file");
        assert!(out.output.is_empty());
        assert!(file.starts_with(&dir.path().join(EXPORT_DIR_NAME).display().to_string()));
        assert_eq!(std::fs::read(&file).unwrap().len(), INLINE_MAX + 1);
        let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(budget, REPORT_INLINE_BUDGET);
    }

    #[test]
    fn without_an_export_dir_large_output_is_cut_and_flagged() {
        let big = Captured {
            exit: 0,
            stdout: vec![b'x'; INLINE_MAX * 2],
            ..Captured::default()
        };
        let mut budget = REPORT_INLINE_BUDGET;
        let out = place(&host_env(None), DiagnosticProbe::Dmesg, big, &mut budget);
        assert!(out.truncated);
        assert_eq!(out.output.len(), INLINE_MAX);
    }

    #[test]
    fn file_probes_write_into_their_own_export_subdir() {
        let dir = tempfile::tempdir().unwrap();
        let env = host_env(Some(dir.path().to_path_buf()));
        let out = with_out_dir(&env, "t", |d| {
            std::fs::write(d.join("a.bin"), b"1").unwrap();
            Captured::default()
        });
        assert_eq!(
            out.artifacts,
            vec![dir.path().join(EXPORT_DIR_NAME).join("t/a.bin")]
        );
    }

    #[test]
    fn a_hung_probe_is_killed_at_its_deadline() {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "echo start; sleep 30"]);
        let started = Instant::now();
        let c = run_bounded(cmd, Duration::from_millis(300));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(c.exit, -1);
        assert!(c.truncated);
    }
}
