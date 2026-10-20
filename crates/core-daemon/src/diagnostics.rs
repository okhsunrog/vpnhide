//! `collect_diagnostics`: the fixed set of root probes the app may ask for (protocol v5).
//!
//! The app runs no root process. Every root read it needs is a [`DiagnosticProbe`]; each maps
//! to a `vpnhide-core probe …|observe …` child (Rust, see `device_probes`), a fixed `logcat`
//! invocation, or an in-process kernel-log read. No shell runs, and no caller-supplied
//! command, script or path exists. Probes run concurrently, each bounded in time and output.
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
    /// This binary, for `vpnhide-core probe …` children.
    pub core_exe: PathBuf,
    /// The app data dir (export dir parent, artifact owner). `None`: no file outputs.
    pub app_dir: Option<PathBuf>,
}

impl ProbeEnv {
    pub fn from_env() -> Self {
        ProbeEnv {
            core_exe: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("vpnhide-core")),
            app_dir: crate::app_socket::app_data_dir_from_env(),
        }
    }

    fn export_dir(&self) -> Option<PathBuf> {
        self.app_dir.as_ref().map(|d| d.join(EXPORT_DIR_NAME))
    }
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
    vpnhide_activator::sys::clear_kernel_log().is_ok()
}

fn run_probe(env: &ProbeEnv, probe: &DiagnosticProbe) -> Captured {
    // Every probe is core's own bounded child (`vpnhide-core probe …`, Rust in
    // `device_probes`) or a fixed system tool invoked directly. Never a shell.
    let core = |args: &[&str], timeout: Duration| {
        let mut cmd = Command::new(&env.core_exe);
        cmd.args(args);
        run_bounded(cmd, timeout)
    };
    match probe {
        DiagnosticProbe::RootSnapshot { packages } => {
            let mut args = vec!["probe", "root-snapshot"];
            if *packages {
                args.push("--packages");
            }
            core(&args, Duration::from_secs(35))
        }
        DiagnosticProbe::DebugSnapshot => {
            core(&["probe", "debug-snapshot"], Duration::from_secs(125))
        }
        DiagnosticProbe::HookCounters => core(&["probe", "hook-counters"], Duration::from_secs(10)),
        DiagnosticProbe::Dmesg => match vpnhide_activator::sys::kernel_log() {
            Ok(text) => Captured {
                exit: 0,
                stdout: text.into_bytes(),
                ..Captured::default()
            },
            Err(e) => Captured {
                exit: 1,
                stdout: format!("klogctl: {e}").into_bytes(),
                ..Captured::default()
            },
        },
        DiagnosticProbe::LogcatAppTags => {
            let mut cmd = Command::new("logcat");
            cmd.args(["-d", "-b", "all", "-v", "threadtime", "-s"])
                .args(APP_LOG_TAGS);
            run_bounded(cmd, Duration::from_secs(30))
        }
        DiagnosticProbe::LogcatLsposedBoot => {
            core(&["probe", "logcat-lsposed"], Duration::from_secs(20))
        }
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
        DiagnosticProbe::KernelPartitions => {
            core(&["probe", "kernel-partitions"], Duration::from_secs(30))
        }
        DiagnosticProbe::KernelImages => with_out_dir(env, "kernel-images", |dir| {
            let dir = dir.display().to_string();
            core(&["probe", "kernel-images", &dir], PROBE_MAX_RUNTIME)
        }),
        DiagnosticProbe::LsposedModulesDb => with_out_dir(env, "lsposed-db", |dir| {
            let dir = dir.display().to_string();
            core(&["probe", "lsposed-db", &dir], Duration::from_secs(15))
        }),
        DiagnosticProbe::GroundTruthChecks => core(&["probe", "checks"], Duration::from_secs(20)),
        DiagnosticProbe::AppVpnState { uid } => {
            let uid = uid.to_string();
            core(
                &["observe", "app-vpn-state", "--uid", &uid],
                Duration::from_secs(20),
            )
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
            core_exe: PathBuf::from("/nonexistent/vpnhide-core"),
            app_dir,
        }
    }

    #[test]
    fn a_missing_core_child_is_a_failed_probe_not_a_shell_fallback() {
        let report = collect(
            &host_env(None),
            &DiagnosticsRequest {
                probes: vec![DiagnosticProbe::RootSnapshot { packages: false }],
                clear_dmesg: false,
            },
        );
        assert_eq!(report.outputs[0].exit, -1);
        assert!(!report.dmesg_cleared);
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
        let mut cmd = Command::new("sleep");
        cmd.arg("30");
        let started = Instant::now();
        let c = run_bounded(cmd, Duration::from_millis(300));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(c.exit, -1);
        assert!(c.truncated);
    }
}
