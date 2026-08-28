use std::fs;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::{
    APATCH_DIR, KMOD_CTL, KPM_CTL_LOCK, KpmBootOutcome, KpmBootReport,
    OPTIONAL_FEATURE_FILESYSTEM_IFACE_PATHS, PORTS_CHAIN4, PORTS_CHAIN6, PORTS_STATUS_DIR, Result,
    activate_kmod_boot, activate_kpatch_boot, activate_kpm_boot, activate_ports_recorded,
    activate_zygisk_boot, kmod_backend_present, load_kpm_boot, optional_feature_enabled,
    optional_feature_enabled_or_default, write_atomic,
};

const KMOD_STATUS_DIR: &str = "/data/adb/vpnhide_kmod";
const KPATCH_STATUS_DIR: &str = "/data/adb/vpnhide_kpatch";
const KPATCH_LOAD_STATUS: &str = "/data/adb/vpnhide_kpatch/load_status";
const KMOD_LOAD_STATUS: &str = "/data/adb/vpnhide_kmod/load_status";
const KMOD_LOAD_DMESG: &str = "/data/adb/vpnhide_kmod/load_dmesg";
const KMOD_NAME: &str = "vpnhide_kmod";
const KPM_STATUS_DIR: &str = "/data/adb/vpnhide_kpm";
const KPM_LOAD_STATUS: &str = "/data/adb/vpnhide_kpm/load_status";
pub(crate) const CHILD_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RootManager {
    Magisk,
    KernelSu,
    Apatch,
    Unknown,
}

impl RootManager {
    fn detect() -> Self {
        // APatch and KernelSU may expose Magisk-compatible directories, so
        // exclusive manager directories override the compatibility marker.
        if Path::new("/data/adb/ap").is_dir() {
            Self::Apatch
        } else if Path::new("/data/adb/ksu").is_dir() {
            Self::KernelSu
        } else if Path::new("/data/adb/magisk").is_dir() {
            Self::Magisk
        } else {
            Self::Unknown
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Magisk => "magisk",
            Self::KernelSu => "kernelsu",
            Self::Apatch => "apatch",
            Self::Unknown => "unknown",
        }
    }
}

struct KmodLoadStatus {
    timestamp: u64,
    boot_id: String,
    uname_r: String,
    gki_variant: String,
    kmod_version: String,
    root_manager: RootManager,
    kprobes: String,
    kretprobes: String,
    filesystem_hiding: bool,
    filesystem_config_exit: i32,
    filesystem_config_error: String,
    insmod_exit: i32,
    loaded: bool,
    insmod_stderr: String,
}

impl KmodLoadStatus {
    fn render(&self) -> String {
        format!(
            "timestamp={}\n\
             boot_id={}\n\
             uname_r={}\n\
             gki_variant={}\n\
             kmod_version={}\n\
             root_manager={}\n\
             kprobes={}\n\
             kretprobes={}\n\
             filesystem_hiding={}\n\
             filesystem_config_exit={}\n\
             filesystem_config_error={}\n\
             insmod_exit={}\n\
             loaded={}\n\
             insmod_stderr={}\n",
            self.timestamp,
            sanitize_line(&self.boot_id, None),
            sanitize_line(&self.uname_r, None),
            sanitize_line(&self.gki_variant, None),
            sanitize_line(&self.kmod_version, None),
            self.root_manager.as_str(),
            sanitize_line(&self.kprobes, None),
            sanitize_line(&self.kretprobes, None),
            u8::from(self.filesystem_hiding),
            self.filesystem_config_exit,
            sanitize_line(&self.filesystem_config_error, None),
            self.insmod_exit,
            u8::from(self.loaded),
            sanitize_line(&self.insmod_stderr, None),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KpmStatusRuntime {
    Activator,
    KpatchNext,
    Apatch,
    Conflict,
    Unknown,
}

impl KpmStatusRuntime {
    fn as_str(self) -> &'static str {
        match self {
            Self::Activator => "activator",
            Self::KpatchNext => "kpatch-next",
            Self::Apatch => "apatch",
            Self::Conflict => "conflict",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KpmStatusReason {
    Ok,
    ConflictingBackend,
    MissingKpm,
    AwaitingSuperkey,
    UnsupportedKernel,
    ActivationFailed,
    LoadFailed,
}

impl KpmStatusReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::ConflictingBackend => "conflicting_backend",
            Self::MissingKpm => "missing_kpm",
            Self::AwaitingSuperkey => "awaiting_superkey",
            Self::UnsupportedKernel => "unsupported_kernel",
            Self::ActivationFailed => "activation_failed",
            Self::LoadFailed => "load_failed",
        }
    }
}

struct KpmLoadStatus {
    runtime: KpmStatusRuntime,
    loaded: bool,
    filesystem_hiding: Option<bool>,
    reason: KpmStatusReason,
    detail: String,
}

impl KpmLoadStatus {
    fn render(&self) -> String {
        let filesystem_hiding = self
            .filesystem_hiding
            .map(|enabled| u8::from(enabled).to_string())
            .unwrap_or_default();
        format!(
            "timestamp={}\n\
             boot_id={}\n\
             uname_r={}\n\
             runtime={}\n\
             loaded={}\n\
             filesystem_hiding={}\n\
             reason={}\n\
             detail={}\n",
            unix_timestamp(),
            sanitize_line(&read_trimmed("/proc/sys/kernel/random/boot_id"), None),
            sanitize_line(&uname_release(), None),
            self.runtime.as_str(),
            u8::from(self.loaded),
            filesystem_hiding,
            self.reason.as_str(),
            sanitize_line(&self.detail, Some(240)),
        )
    }
}

pub fn boot_load_kmod() -> Result<()> {
    if module_is_loaded(KMOD_NAME)? {
        log_android("vpnhide", "kernel module already loaded");
        return Ok(());
    }

    fs::create_dir_all(KMOD_STATUS_DIR)?;
    let module_dir = current_module_dir()?;
    let module_prop = read_module_properties(&module_dir.join("module.prop"));
    let (kprobes, kretprobes) = read_kprobe_config();
    let (filesystem_hiding, filesystem_config_exit, filesystem_config_error) =
        match optional_feature_enabled(OPTIONAL_FEATURE_FILESYSTEM_IFACE_PATHS) {
            Ok(enabled) => (enabled, i32::from(!enabled), String::new()),
            Err(err) => (false, 2, err.to_string()),
        };

    let ko = module_dir.join("vpnhide_kmod.ko");
    let (insmod_exit, insmod_stderr) = if ko.is_file() {
        let mut command = Command::new("insmod");
        command
            .arg(&ko)
            .arg(format!("filesystem_hiding={}", u8::from(filesystem_hiding)));
        match output_with_timeout(&mut command, CHILD_COMMAND_TIMEOUT) {
            Ok(output) => (
                exit_code(&output.status),
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ),
            Err(err) if err.kind() == ErrorKind::TimedOut => (
                124,
                format!(
                    "insmod timed out after {}s",
                    CHILD_COMMAND_TIMEOUT.as_secs()
                ),
            ),
            Err(err) => (126, err.to_string()),
        }
    } else {
        (
            127,
            format!("vpnhide_kmod.ko not found at {}", ko.display()),
        )
    };

    let loaded = module_is_loaded(KMOD_NAME)?;
    write_kmod_dmesg()?;
    let status = KmodLoadStatus {
        timestamp: unix_timestamp(),
        boot_id: read_trimmed("/proc/sys/kernel/random/boot_id"),
        uname_r: uname_release(),
        gki_variant: module_prop
            .iter()
            .find_map(|(key, value)| (key == "gkiVariant").then_some(value.clone()))
            .unwrap_or_default(),
        kmod_version: module_prop
            .iter()
            .find_map(|(key, value)| (key == "version").then_some(value.clone()))
            .unwrap_or_default(),
        root_manager: RootManager::detect(),
        kprobes,
        kretprobes,
        filesystem_hiding,
        filesystem_config_exit,
        filesystem_config_error,
        insmod_exit,
        loaded,
        insmod_stderr,
    };
    write_atomic(
        Path::new(KMOD_LOAD_STATUS),
        status.render().as_bytes(),
        0o644,
    )?;

    if loaded {
        log_android(
            "vpnhide",
            &format!(
                "kernel module loaded (gki={} kernel={})",
                status.gki_variant, status.uname_r
            ),
        );
        Ok(())
    } else {
        let detail = sanitize_line(&status.insmod_stderr, Some(240));
        log_android(
            "vpnhide",
            &format!(
                "kernel module NOT loaded (exit={} gki={} kernel={}): {detail}",
                status.insmod_exit, status.gki_variant, status.uname_r
            ),
        );
        Err(format!("kernel module did not load: {detail}").into())
    }
}

pub fn uninstall_kmod() -> Result<()> {
    remove_if_present(KMOD_LOAD_STATUS)?;
    remove_if_present(KMOD_LOAD_DMESG)?;
    remove_empty_dir(KMOD_STATUS_DIR)?;
    log_android("vpnhide", "kmod: persistent module state removed");
    Ok(())
}

pub fn boot_service_kmod() -> Result<()> {
    match activate_kmod_boot() {
        Ok(()) => {
            log_android("vpnhide", "kmod: activator finished boot config");
            Ok(())
        }
        Err(err) => {
            log_android("vpnhide", &format!("kmod: activator failed: {err}"));
            Err(err)
        }
    }
}

/// Backend id the in-tree driver reports in its /proc/vpnhide_ctl `status` line
/// (data/hooks.toml -> VPNHIDE_BACKEND_KPATCH). The .ko reports 0; the two share
/// the node and are mutually exclusive, so this is how we tell which is live.
const KPATCH_BACKEND_ID: u32 = 4;

/// Parse the `backend 0x<n>` field from a /proc/vpnhide_ctl status read.
fn observed_ctl_backend() -> Option<u32> {
    let text = fs::read_to_string(KMOD_CTL).ok()?;
    let field = text
        .lines()
        .find_map(|line| line.strip_prefix("backend "))?
        .trim();
    let hex = field.strip_prefix("0x").unwrap_or(field);
    u32::from_str_radix(hex, 16).ok()
}

struct KpatchLoadStatus {
    loaded: bool,
    detail: String,
}

impl KpatchLoadStatus {
    fn render(&self) -> String {
        format!(
            "timestamp={}\n\
             boot_id={}\n\
             uname_r={}\n\
             runtime=kpatch\n\
             loaded={}\n\
             detail={}\n",
            unix_timestamp(),
            sanitize_line(&read_trimmed("/proc/sys/kernel/random/boot_id"), None),
            sanitize_line(&uname_release(), None),
            u8::from(self.loaded),
            sanitize_line(&self.detail, Some(240)),
        )
    }
}

fn write_kpatch_status(status: KpatchLoadStatus) -> Result<()> {
    fs::create_dir_all(KPATCH_STATUS_DIR)?;
    write_atomic(
        Path::new(KPATCH_LOAD_STATUS),
        status.render().as_bytes(),
        0o644,
    )
}

/// Boot config delivery for the in-tree backend. Unlike the .ko there is nothing
/// to insmod: the driver is already live if the kernel was built with
/// CONFIG_VPNHIDE. Verify that (control node present, backend id == kpatch),
/// deliver the config snapshot, and record liveness for the dashboard.
pub fn boot_service_kpatch() -> Result<()> {
    fs::create_dir_all(KPATCH_STATUS_DIR)?;

    match observed_ctl_backend() {
        Some(KPATCH_BACKEND_ID) => {}
        Some(other) => {
            let detail = format!(
                "/proc/vpnhide_ctl reports backend 0x{other:x}, not kpatch — kernel not built with CONFIG_VPNHIDE, or the .ko is loaded"
            );
            log_android("vpnhide", &format!("kpatch: {detail}"));
            return write_kpatch_status(KpatchLoadStatus {
                loaded: false,
                detail,
            });
        }
        None => {
            let detail =
                "/proc/vpnhide_ctl absent — kernel not built with CONFIG_VPNHIDE".to_owned();
            log_android("vpnhide", &format!("kpatch: {detail}"));
            return write_kpatch_status(KpatchLoadStatus {
                loaded: false,
                detail,
            });
        }
    }

    match activate_kpatch_boot() {
        Ok(()) => {
            log_android("vpnhide", "kpatch: activator finished boot config");
            write_kpatch_status(KpatchLoadStatus {
                loaded: true,
                detail: "in-tree backend live".to_owned(),
            })
        }
        Err(err) => {
            log_android("vpnhide", &format!("kpatch: activator failed: {err}"));
            write_kpatch_status(KpatchLoadStatus {
                loaded: false,
                detail: format!("config delivery failed: {err}"),
            })?;
            Err(err)
        }
    }
}

pub fn uninstall_kpatch() -> Result<()> {
    remove_if_present(KPATCH_LOAD_STATUS)?;
    remove_empty_dir(KPATCH_STATUS_DIR)?;
    log_android("vpnhide", "kpatch: persistent module state removed");
    Ok(())
}

pub fn boot_load_kpm() -> Result<()> {
    fs::create_dir_all(KPM_STATUS_DIR)?;
    let kpm_file = current_module_dir()?.join("vpnhide.kpm");
    if kmod_backend_present() {
        log_android(
            "vpnhide",
            "kpm: .ko backend present — not loading KPM (single-active)",
        );
        return write_kpm_status(KpmLoadStatus {
            runtime: KpmStatusRuntime::Conflict,
            loaded: false,
            filesystem_hiding: None,
            reason: KpmStatusReason::ConflictingBackend,
            detail: "vpnhide_kmod present".to_owned(),
        });
    }
    if !kpm_file.is_file() {
        let detail = format!("vpnhide.kpm not found at {}", kpm_file.display());
        write_kpm_status(KpmLoadStatus {
            runtime: KpmStatusRuntime::Unknown,
            loaded: false,
            filesystem_hiding: None,
            reason: KpmStatusReason::MissingKpm,
            detail: detail.clone(),
        })?;
        return Err(detail.into());
    }

    if Path::new(APATCH_DIR).is_dir() {
        let filesystem_hiding = Some(optional_feature_enabled_or_default(
            OPTIONAL_FEATURE_FILESYSTEM_IFACE_PATHS,
        ));
        log_android(
            "vpnhide",
            "kpm: APatch/FolkPatch runtime — deferring load to service activator",
        );
        return write_kpm_status(KpmLoadStatus {
            runtime: KpmStatusRuntime::Apatch,
            loaded: false,
            filesystem_hiding,
            reason: KpmStatusReason::AwaitingSuperkey,
            detail: "awaiting_superkey".to_owned(),
        });
    }

    match load_kpm_boot() {
        Ok(KpmBootReport {
            outcome: KpmBootOutcome::Configured,
            filesystem_hiding,
        }) => {
            log_android("vpnhide", "kpm: loaded (kpatch-next)");
            write_kpm_status(KpmLoadStatus {
                runtime: KpmStatusRuntime::KpatchNext,
                loaded: true,
                filesystem_hiding,
                reason: KpmStatusReason::Ok,
                detail: String::new(),
            })
        }
        Ok(KpmBootReport {
            outcome: KpmBootOutcome::DeferredConflict,
            ..
        }) => write_kpm_status(KpmLoadStatus {
            runtime: KpmStatusRuntime::Conflict,
            loaded: false,
            filesystem_hiding: None,
            reason: KpmStatusReason::ConflictingBackend,
            detail: "vpnhide_kmod present".to_owned(),
        }),
        Ok(KpmBootReport {
            outcome: KpmBootOutcome::AwaitingAuthentication,
            filesystem_hiding,
        }) => write_kpm_status(KpmLoadStatus {
            runtime: KpmStatusRuntime::Apatch,
            loaded: false,
            filesystem_hiding,
            reason: KpmStatusReason::AwaitingSuperkey,
            detail: "awaiting_superkey".to_owned(),
        }),
        Ok(KpmBootReport {
            outcome: KpmBootOutcome::UnsupportedKernel,
            filesystem_hiding: reported_feature,
        }) => {
            let detail = format!("unsupported kernel {}", uname_release());
            log_android("vpnhide", &format!("kpm: {detail}"));
            write_kpm_status(KpmLoadStatus {
                runtime: KpmStatusRuntime::KpatchNext,
                loaded: false,
                filesystem_hiding: reported_feature,
                reason: KpmStatusReason::UnsupportedKernel,
                detail,
            })
        }
        Err(err) => {
            let detail = err.to_string();
            log_android("vpnhide", &format!("kpm: load failed: {detail}"));
            write_kpm_status(KpmLoadStatus {
                runtime: KpmStatusRuntime::KpatchNext,
                loaded: false,
                filesystem_hiding: None,
                reason: KpmStatusReason::LoadFailed,
                detail: detail.clone(),
            })?;
            Err(err)
        }
    }
}

pub fn boot_service_kpm() -> Result<()> {
    fs::create_dir_all(KPM_STATUS_DIR)?;
    if kmod_backend_present() {
        log_android(
            "vpnhide",
            "kpm: .ko backend present — not configuring KPM (single-active)",
        );
        return write_kpm_status(KpmLoadStatus {
            runtime: KpmStatusRuntime::Conflict,
            loaded: false,
            filesystem_hiding: None,
            reason: KpmStatusReason::ConflictingBackend,
            detail: "vpnhide_kmod present".to_owned(),
        });
    }
    let kpm_file = current_module_dir()?.join("vpnhide.kpm");
    if !kpm_file.is_file() {
        let detail = format!("vpnhide.kpm not found at {}", kpm_file.display());
        write_kpm_status(KpmLoadStatus {
            runtime: KpmStatusRuntime::Activator,
            loaded: false,
            filesystem_hiding: None,
            reason: KpmStatusReason::MissingKpm,
            detail: detail.clone(),
        })?;
        return Err(detail.into());
    }

    let loaded_feature = current_kpm_loaded_feature();
    let status = match activate_kpm_boot() {
        Ok(KpmBootReport {
            outcome: KpmBootOutcome::Configured,
            filesystem_hiding,
        }) => {
            log_android("vpnhide", "kpm: activator finished boot config");
            KpmLoadStatus {
                runtime: KpmStatusRuntime::Activator,
                loaded: true,
                filesystem_hiding: loaded_feature.or(filesystem_hiding),
                reason: KpmStatusReason::Ok,
                detail: "configured".to_owned(),
            }
        }
        Ok(KpmBootReport {
            outcome: KpmBootOutcome::DeferredConflict,
            ..
        }) => {
            log_android("vpnhide", "kpm: activator deferred to .ko (single-active)");
            KpmLoadStatus {
                runtime: KpmStatusRuntime::Conflict,
                loaded: false,
                filesystem_hiding: None,
                reason: KpmStatusReason::ConflictingBackend,
                detail: "vpnhide_kmod present".to_owned(),
            }
        }
        Ok(KpmBootReport {
            outcome: KpmBootOutcome::AwaitingAuthentication,
            filesystem_hiding,
        }) => {
            log_android("vpnhide", "kpm: awaiting APatch authentication");
            KpmLoadStatus {
                runtime: KpmStatusRuntime::Apatch,
                loaded: false,
                filesystem_hiding,
                reason: KpmStatusReason::AwaitingSuperkey,
                detail: "awaiting_superkey".to_owned(),
            }
        }
        Ok(KpmBootReport {
            outcome: KpmBootOutcome::UnsupportedKernel,
            filesystem_hiding,
        }) => {
            let detail = format!("unsupported kernel {}", uname_release());
            log_android("vpnhide", &format!("kpm: {detail}"));
            KpmLoadStatus {
                runtime: KpmStatusRuntime::Activator,
                loaded: false,
                filesystem_hiding,
                reason: KpmStatusReason::UnsupportedKernel,
                detail,
            }
        }
        Err(err) => {
            let detail = err.to_string();
            log_android("vpnhide", &format!("kpm: activator failed: {detail}"));
            write_kpm_status(KpmLoadStatus {
                runtime: KpmStatusRuntime::Activator,
                loaded: false,
                filesystem_hiding: loaded_feature,
                reason: KpmStatusReason::ActivationFailed,
                detail: detail.clone(),
            })?;
            return Err(err);
        }
    };
    write_kpm_status(status)
}

pub fn uninstall_kpm() -> Result<()> {
    remove_if_present(KPM_LOAD_STATUS)?;
    remove_if_present(KPM_CTL_LOCK)?;
    remove_empty_dir(KPM_STATUS_DIR)?;
    log_android("vpnhide", "kpm: persistent module state removed");
    Ok(())
}

pub fn boot_service_zygisk() -> Result<()> {
    match activate_zygisk_boot() {
        Ok(()) => {
            log_android("vpnhide", "zygisk: activator finished boot config");
            Ok(())
        }
        Err(err) => {
            log_android("vpnhide", &format!("zygisk: activator failed: {err}"));
            Err(err)
        }
    }
}

pub fn uninstall_zygisk() {
    log_android("vpnhide", "zygisk: module removed");
}

pub fn boot_service_ports() -> Result<()> {
    // netd may replace its baseline chains during boot. Wait for the first
    // complete ruleset, apply once, then repeat after the observed rebuild
    // window. This runs in the non-blocking late_start service stage.
    for _ in 0..60 {
        if command_succeeds("iptables", &["-L", "bw_OUTPUT", "-n"]) {
            break;
        }
        thread::sleep(Duration::from_secs(1));
    }

    match activate_ports_recorded(true) {
        Ok(()) => log_android("vpnhide_ports", "applied iptables rules at boot"),
        Err(err) => log_android(
            "vpnhide_ports",
            &format!("initial activator pass failed: {err}"),
        ),
    }

    thread::sleep(Duration::from_secs(30));
    match activate_ports_recorded(true) {
        Ok(()) => {
            log_android(
                "vpnhide_ports",
                "re-applied iptables rules (T+30s safety pass)",
            );
            Ok(())
        }
        Err(err) => {
            log_android("vpnhide_ports", &format!("activator failed: {err}"));
            Err(err)
        }
    }
}

pub fn uninstall_ports() -> Result<()> {
    while command_succeeds("iptables", &["-D", "OUTPUT", "-j", PORTS_CHAIN4]) {}
    let _ = command_succeeds("iptables", &["-F", PORTS_CHAIN4]);
    let _ = command_succeeds("iptables", &["-X", PORTS_CHAIN4]);

    while command_succeeds("ip6tables", &["-D", "OUTPUT", "-j", PORTS_CHAIN6]) {}
    let _ = command_succeeds("ip6tables", &["-F", PORTS_CHAIN6]);
    let _ = command_succeeds("ip6tables", &["-X", PORTS_CHAIN6]);

    remove_if_present(Path::new(PORTS_STATUS_DIR).join("load_status"))?;
    remove_if_present(Path::new(PORTS_STATUS_DIR).join("load_log"))?;
    remove_empty_dir(PORTS_STATUS_DIR)?;
    log_android("vpnhide_ports", "uninstalled, iptables chains removed");
    Ok(())
}

pub(crate) fn current_module_dir() -> Result<PathBuf> {
    std::env::current_exe()?
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "activator path has no parent directory".into())
}

pub(crate) fn log_android(tag: &str, message: &str) {
    let _ = Command::new("log").args(["-t", tag, message]).status();
}

pub(crate) fn command_succeeds(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub(crate) fn remove_if_present(path: impl AsRef<Path>) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

pub(crate) fn remove_empty_dir(path: impl AsRef<Path>) -> Result<()> {
    match fs::remove_dir(path) {
        Ok(()) => Ok(()),
        Err(err)
            if matches!(
                err.kind(),
                ErrorKind::NotFound | ErrorKind::DirectoryNotEmpty
            ) =>
        {
            Ok(())
        }
        Err(err) => Err(err.into()),
    }
}

fn read_module_properties(path: &Path) -> Vec<(String, String)> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

fn write_kpm_status(status: KpmLoadStatus) -> Result<()> {
    write_atomic(
        Path::new(KPM_LOAD_STATUS),
        status.render().as_bytes(),
        0o644,
    )
}

fn current_kpm_loaded_feature() -> Option<bool> {
    let raw = fs::read_to_string(KPM_LOAD_STATUS).unwrap_or_default();
    let values = raw
        .lines()
        .filter_map(|line| line.split_once('='))
        .collect::<Vec<_>>();
    let value = |key: &str| {
        values
            .iter()
            .find_map(|(name, value)| (*name == key).then_some(*value))
    };
    let boot_id = read_trimmed("/proc/sys/kernel/random/boot_id");
    if value("boot_id") == Some(boot_id.as_str()) && value("loaded") == Some("1") {
        match value("filesystem_hiding") {
            Some("1") => return Some(true),
            Some("0") => return Some(false),
            _ => {}
        }
    }
    None
}

fn read_kprobe_config() -> (String, String) {
    let config = read_gzip_text("/proc/config.gz");
    let option = |name: &str| {
        config
            .as_deref()
            .and_then(|raw| {
                raw.lines()
                    .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
            })
            .unwrap_or(if config.is_some() { "n" } else { "unknown" })
            .to_owned()
    };
    (option("CONFIG_KPROBES"), option("CONFIG_KRETPROBES"))
}

fn read_gzip_text(path: &str) -> Option<String> {
    for (program, args) in [("zcat", vec![path]), ("gunzip", vec!["-c", path])] {
        let Ok(output) = Command::new(program).args(args).output() else {
            continue;
        };
        if output.status.success() {
            return String::from_utf8(output.stdout).ok();
        }
    }
    None
}

fn write_kmod_dmesg() -> Result<()> {
    let raw = Command::new("dmesg")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default();
    let mut matching = raw
        .lines()
        .filter(|line| {
            let line = line.to_ascii_lowercase();
            ["vpnhide", "kretprobe", "modules.verify", "version magic"]
                .iter()
                .any(|needle| line.contains(needle))
        })
        .collect::<Vec<_>>();
    if matching.len() > 40 {
        matching.drain(..matching.len() - 40);
    }
    let mut text = matching.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    write_atomic(Path::new(KMOD_LOAD_DMESG), text.as_bytes(), 0o644)
}

fn module_is_loaded(name: &str) -> Result<bool> {
    Ok(fs::read_to_string("/proc/modules")?
        .lines()
        .any(|line| line.split_whitespace().next() == Some(name)))
}

pub(crate) fn output_with_timeout(
    command: &mut Command,
    timeout: Duration,
) -> std::io::Result<Output> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("child stdout was not piped despite Stdio::piped"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| std::io::Error::other("child stderr was not piped despite Stdio::piped"))?;
    // Drain both streams while the child runs. Waiting first can deadlock when
    // a command fills either pipe buffer before it exits.
    let stdout_reader = thread::spawn(move || read_all(stdout));
    let stderr_reader = thread::spawn(move || read_all(stderr));
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Output {
                status,
                stdout: join_reader(stdout_reader)?,
                stderr: join_reader(stderr_reader)?,
            });
        }
        if Instant::now() >= deadline {
            child.kill()?;
            let _ = child.wait();
            let _ = join_reader(stdout_reader);
            let _ = join_reader(stderr_reader);
            return Err(std::io::Error::new(
                ErrorKind::TimedOut,
                "command timed out",
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn read_all(mut reader: impl Read) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn join_reader(reader: thread::JoinHandle<std::io::Result<Vec<u8>>>) -> std::io::Result<Vec<u8>> {
    reader
        .join()
        .map_err(|_| std::io::Error::other("child output reader panicked"))?
}

fn exit_code(status: &ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}

fn sanitize_line(value: &str, max_chars: Option<usize>) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    match max_chars {
        Some(max_chars) => collapsed.chars().take(max_chars).collect(),
        None => collapsed,
    }
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn read_trimmed(path: &str) -> String {
    fs::read_to_string(path)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

fn uname_release() -> String {
    Command::new("uname")
        .arg("-r")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_values_stay_on_one_line() {
        assert_eq!(sanitize_line(" one\n\ttwo  three ", None), "one two three");
        assert_eq!(sanitize_line("абвгд", Some(3)), "абв");
    }

    #[test]
    fn root_manager_tokens_match_dashboard_contract() {
        assert_eq!(RootManager::Magisk.as_str(), "magisk");
        assert_eq!(RootManager::KernelSu.as_str(), "kernelsu");
        assert_eq!(RootManager::Apatch.as_str(), "apatch");
        assert_eq!(RootManager::Unknown.as_str(), "unknown");
    }

    #[test]
    fn kpm_status_uses_stable_dashboard_tokens() {
        let status = KpmLoadStatus {
            runtime: KpmStatusRuntime::Apatch,
            loaded: false,
            filesystem_hiding: Some(true),
            reason: KpmStatusReason::AwaitingSuperkey,
            detail: "awaiting_superkey".to_owned(),
        }
        .render();

        assert!(status.contains("runtime=apatch\n"));
        assert!(status.contains("loaded=0\n"));
        assert!(status.contains("filesystem_hiding=1\n"));
        assert!(status.contains("reason=awaiting_superkey\n"));
        assert!(status.contains("detail=awaiting_superkey\n"));
    }

    #[test]
    fn timed_command_drains_chatty_stdout_and_stderr() {
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "head -c 131072 /dev/zero; head -c 131072 /dev/zero >&2",
        ]);

        let output = output_with_timeout(&mut command, Duration::from_secs(2)).unwrap();

        assert!(output.status.success());
        assert_eq!(output.stdout.len(), 131_072);
        assert_eq!(output.stderr.len(), 131_072);
    }

    #[test]
    fn timed_command_kills_a_stalled_child() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 1"]);

        let error = output_with_timeout(&mut command, Duration::from_millis(10)).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::TimedOut);
    }
}
