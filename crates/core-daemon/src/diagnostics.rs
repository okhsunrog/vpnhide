//! `collect_diagnostics`: the fixed set of root probes the app may ask for (protocol v5).
//!
//! The app runs no root process. Every root read it needs is a [`DiagnosticProbe`]; each maps
//! to a `vpnhide-core probe …|observe …` child (Rust, see `device_probes`), a fixed `logcat -d`
//! invocation, or a parent-owned kernel/KPM observation. KPM credentials remain in the
//! daemon; isolated snapshot children consume typed nonsecret telemetry over stdin.
//! No caller-supplied command, script or path exists. Probes run concurrently with
//! bounded child runtimes and captured output.
//!
//! Output up to [`INLINE_MAX`] travels inline; anything larger (debug snapshot, logcat dumps) and
//! every artifact (kernel images, LSPosed db copies) is written to the export dir inside the
//! app's own data dir, owned by the app, so the reply stays far below the transport frame.

use std::collections::BTreeMap;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
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
const MAX_PROBES: usize = 32;
const MAX_PROBE_WORKERS: usize = 4;
const REQUEST_OUTPUT_MAX: usize = 64 * 1024 * 1024;
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

    pub(crate) fn export_dir(&self) -> Option<PathBuf> {
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
    if request.probes.len() > MAX_PROBES {
        return DiagnosticsReport {
            outputs: request
                .probes
                .first()
                .map(|probe| ProbeOutput {
                    probe: probe.clone(),
                    exit: -1,
                    output: "too many probes in one request".into(),
                    file: None,
                    artifacts: Vec::new(),
                    truncated: false,
                })
                .into_iter()
                .collect(),
            dmesg_cleared: false,
        };
    }
    let mut probes = Vec::new();
    for probe in &request.probes {
        if !probes.contains(probe) {
            probes.push(probe.clone());
        }
    }
    let dmesg_cleared = request.clear_dmesg && clear_dmesg();
    // A queue of individually bounded probes must also finish inside the RPC
    // budget, rather than retaining workers long after its caller timed out.
    let deadline = Instant::now()
        + vpnhide_core_rpc::DIAGNOSTICS_DEADLINE.saturating_sub(Duration::from_secs(10));
    // Credentials stay in this process. Children receive only the already-public
    // telemetry, so a session-only KPM key remains usable without crossing exec.
    let inputs = probes
        .iter()
        .any(|probe| {
            matches!(
                probe,
                DiagnosticProbe::RootSnapshot { .. }
                    | DiagnosticProbe::DebugSnapshot
                    | DiagnosticProbe::HookCounters
            )
        })
        .then(|| {
            use crate::device_probes::{PROBE_INPUT_MAX, ProbeInputs, Ran};
            let mut inputs = ProbeInputs {
                kpm: crate::probe_cli::observations::collect_kpm_state(),
            };
            let encoded = serde_json::to_vec(&inputs).expect("fixed probe observations serialize");
            if encoded.len() <= PROBE_INPUT_MAX {
                encoded
            } else {
                inputs.kpm = Ran {
                    exit: 1,
                    stdout: String::new(),
                    stderr: "KPM telemetry exceeds the probe input limit\n".into(),
                };
                serde_json::to_vec(&inputs).expect("fixed probe observations serialize")
            }
        });
    let limit = OUTPUT_MAX.min(REQUEST_OUTPUT_MAX / probes.len().max(1));
    let next = std::sync::atomic::AtomicUsize::new(0);
    let captured = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..MAX_PROBE_WORKERS.min(probes.len()) {
            let (next, captured, probes, inputs) = (&next, &captured, &probes, inputs.as_deref());
            scope.spawn(move || {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(probe) = probes.get(index) else {
                        break;
                    };
                    let result =
                        std::panic::catch_unwind(|| run_probe(env, probe, limit, deadline, inputs))
                            .unwrap_or(Captured {
                                exit: -1,
                                ..Default::default()
                            });
                    captured
                        .lock()
                        .expect("probe results")
                        .push((index, probe.clone(), result));
                }
            });
        }
    });
    let mut captured = captured.into_inner().expect("probe results");
    captured.sort_by_key(|(index, _, _)| *index);
    let mut budget = REPORT_INLINE_BUDGET;
    let outputs = captured
        .into_iter()
        .map(|(_, probe, result)| place(env, probe, result, &mut budget))
        .collect();
    DiagnosticsReport {
        outputs,
        dmesg_cleared,
    }
}

fn clear_dmesg() -> bool {
    vpnhide_activator::sys::clear_kernel_log().is_ok()
}

fn run_probe(
    env: &ProbeEnv,
    probe: &DiagnosticProbe,
    limit: usize,
    deadline: Instant,
    inputs: Option<&[u8]>,
) -> Captured {
    let expired = || Captured {
        exit: -1,
        stdout: b"diagnostic collection deadline exceeded".to_vec(),
        truncated: true,
        ..Default::default()
    };
    if Instant::now() >= deadline {
        return expired();
    }
    // Every probe is core's own bounded child (`vpnhide-core probe …`, Rust in
    // `device_probes`) or a fixed system tool invoked directly. Never a shell.
    let core = |args: &[&str], timeout: Duration| {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return expired();
        }
        let mut cmd = Command::new(&env.core_exe);
        cmd.args(args);
        let inputs = inputs.filter(|_| {
            args.get(1).is_some_and(|mode| {
                matches!(*mode, "root-snapshot" | "debug-snapshot" | "hook-counters")
            })
        });
        if inputs.is_some() {
            cmd.arg(crate::device_probes::KPM_STDIN_FLAG);
        }
        run_bounded_limit_with_input(cmd, timeout.min(left), limit, inputs)
    };
    let mut result = match probe {
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
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                expired()
            } else {
                run_bounded_limit(cmd, Duration::from_secs(30).min(left), limit)
            }
        }
        DiagnosticProbe::LogcatLsposedBoot => {
            core(&["probe", "logcat-lsposed"], Duration::from_secs(20))
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
    };
    if result.stdout.len() > limit {
        result.stdout.truncate(limit);
        result.truncated = true;
    }
    result
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
    let sequence = FILE_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = export.join(format!("{name}-{}-{sequence}", export_instance()));
    if let Err(e) = prepare_dir(env, &export).and_then(|()| std::fs::create_dir(&dir)) {
        return Captured {
            exit: -1,
            stdout: format!("export dir: {e}").into_bytes(),
            ..Captured::default()
        };
    }
    hand_to_app(env, &dir, 0o700);
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

pub(crate) fn prepare_dir(env: &ProbeEnv, dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    if !std::fs::symlink_metadata(dir)?.is_dir() {
        return Err(std::io::Error::other("export path is not a directory"));
    }
    hand_to_app(env, dir, 0o700);
    Ok(())
}

/// Owner = the app data dir's owner, `mode`, and the dir's SELinux label — the same
/// treatment as the socket, so only the app can read what core exported for it.
pub(crate) fn hand_to_app(env: &ProbeEnv, path: &Path, mode: u32) {
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
    if let Some(app_dir) = &env.app_dir {
        if let Err(e) = crate::auth::hand_to_owner_of(path, app_dir) {
            eprintln!("core: export ownership {}: {e}", path.display());
        }
    }
}

/// Spawn, drain stdout up to [`OUTPUT_MAX`], and kill on `timeout`. stderr is discarded.
#[cfg(test)]
fn run_bounded(cmd: Command, timeout: Duration) -> Captured {
    run_bounded_limit(cmd, timeout, OUTPUT_MAX)
}

fn run_bounded_limit(cmd: Command, timeout: Duration, limit: usize) -> Captured {
    run_bounded_limit_with_input(cmd, timeout, limit, None)
}

fn run_bounded_limit_with_input(
    mut cmd: Command,
    timeout: Duration,
    limit: usize,
    input: Option<&[u8]>,
) -> Captured {
    match vpnhide_activator::capture_command(&mut cmd, input, timeout, limit) {
        Ok(captured) => Captured {
            exit: if captured.timed_out {
                -1
            } else {
                captured.output.status.code().unwrap_or(-1)
            },
            stdout: captured.output.stdout,
            truncated: captured.truncated || captured.timed_out,
            artifacts: Vec::new(),
        },
        Err(_) => Captured {
            exit: -1,
            ..Captured::default()
        },
    }
}

static FILE_SEQ: AtomicU64 = AtomicU64::new(0);

fn export_instance() -> &'static str {
    static INSTANCE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    INSTANCE.get_or_init(|| {
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        format!("{}-{time:x}", std::process::id())
    })
}

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
        let path = export.join(format!(
            "{}-{}-{seq}.txt",
            probe_slug(&probe),
            export_instance()
        ));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .ok()?;
        file.write_all(text.as_bytes()).ok()?;
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
        let collect = |contents: &[u8]| {
            with_out_dir(&env, "t", |d| {
                std::fs::write(d.join("a.bin"), contents).unwrap();
                Captured::default()
            })
        };
        let first = collect(b"1");
        let second = collect(b"2");
        assert_eq!(first.artifacts.len(), 1);
        assert_eq!(second.artifacts.len(), 1);
        assert_ne!(first.artifacts, second.artifacts);
        for (artifact, expected) in [(&first.artifacts[0], b"1"), (&second.artifacts[0], b"2")] {
            assert!(artifact.starts_with(dir.path().join(EXPORT_DIR_NAME)));
            assert!(
                artifact
                    .parent()
                    .unwrap()
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with("t-")
            );
            assert_eq!(std::fs::read(artifact).unwrap(), expected);
        }
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

    #[test]
    fn queued_artifact_probe_past_collection_deadline_does_not_create_files() {
        let dir = tempfile::tempdir().unwrap();
        let env = host_env(Some(dir.path().to_path_buf()));
        let result = run_probe(
            &env,
            &DiagnosticProbe::KernelImages,
            OUTPUT_MAX,
            Instant::now() - Duration::from_secs(1),
            None,
        );
        assert_eq!(result.exit, -1);
        assert!(result.truncated);
        assert!(result.artifacts.is_empty());
        assert!(!dir.path().join(EXPORT_DIR_NAME).exists());
    }
}
