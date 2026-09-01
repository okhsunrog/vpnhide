use std::error::Error;
use std::fs;
use std::io::{ErrorKind, Write};
use std::os::raw::{c_int, c_long};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::str;
use std::thread;
use std::time::Duration;

use vpnhide_protocol::{MAX_TARGET_UIDS, parse_config};

pub type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

pub const CANONICAL_CONFIG: &str = "/data/system/vpnhide_config.json";
pub const KMOD_CTL: &str = "/proc/vpnhide_ctl";
pub const KMOD_MODULE_DIR: &str = "/data/adb/modules/vpnhide_kmod";
/// The built-in backend lives in the kernel image (CONFIG_VPNHIDE=y), not in a
/// loadable module. Its companion module ships only the userspace glue (this
/// activator + boot scripts) that delivers config over the shared control node
/// and records liveness. The control node itself is KMOD_CTL — the built-in backend and kmod
/// speak the identical /proc/vpnhide_ctl wire and are mutually exclusive (one
/// kernel is either built with the driver or loads the .ko, never both).
pub const BUILTIN_MODULE_DIR: &str = "/data/adb/modules/vpnhide_builtin";
pub const ZYGISK_RUNTIME_CONFIG: &str = "/data/adb/modules/vpnhide_zygisk/targets.txt";
pub const KPM_MODULE_FILE: &str = "/data/adb/modules/vpnhide_kpm/vpnhide.kpm";
pub const SUPERKEY_FILE: &str = "/data/adb/vpnhide/superkey";
const APATCH_DIR: &str = "/data/adb/ap";
const APP_PACKAGE: &str = "dev.okhsunrog.vpnhide";
const KPM_NAME: &str = "vpnhide";
const PORTS_CHAIN4: &str = "vpnhide_out";
const PORTS_CHAIN6: &str = "vpnhide_out6";
const PORTS_STATUS_DIR: &str = "/data/adb/vpnhide_ports";
const PORTS_LOAD_STATUS: &str = "/data/adb/vpnhide_ports/load_status";
const PORTS_LOAD_LOG: &str = "/data/adb/vpnhide_ports/load_log";
const KPM_CTL_LOCK: &str = "/data/adb/vpnhide_kpm/ctl.lock";
const KPM_TRUNCATION_MARKER: &str = "# vpnhide truncated";
pub const KPM_SUPPORTED_KERNEL_FAMILIES: &str = "4.9, 4.14, 4.19, 5.4, 5.10, 5.15, 6.1, 6.6, 6.12";
const KPM_SUPPORTED_KERNEL_PAIRS: &[(u32, u32)] = &[
    (4, 9),
    (4, 14),
    (4, 19),
    (5, 4),
    (5, 10),
    (5, 15),
    (6, 1),
    (6, 6),
    (6, 12),
];
// The native-target cap is owned by the shared protocol crate (and mirrored by
// the C backends' `#define MAX_TARGET_UIDS`); alias it here so all three stay in
// lock-step instead of restating the literal capacity.
const MAX_NATIVE_TARGETS: usize = MAX_TARGET_UIDS;
// The control protocol carries a default hookmask for every uid NOT listed as a
// target, which is the mechanism a whitelist mode would ride on: non-zero flips
// `targets` from "the apps to act on" into "the apps to leave alone". Nothing
// emits a non-zero default yet — the shipped model is the blacklist — so the
// activator names the constant rather than spelling a bare 0 at each call site,
// and turning the mode on later is a change here, not in any parser.
const NO_DEFAULT_MASK: u32 = 0;
/// First uid Android hands to an ordinary app; everything below is a system AID
/// (`system_server` 1000, radio 1001, `network_stack` 1073, shell 2000, the OEM
/// 5000s). Mirrored by both kernel backends, which enforce the same floor.
const FIRST_APP_UID: u32 = 10_000;
const PER_USER_RANGE: u32 = 100_000;

/// Whether `uid` identifies an ordinary app rather than a platform AID.
///
/// Compared on the app-id, so a uid from a secondary profile — 1010234 in
/// profile 10 — classifies the same as 10234 in the owner profile. Note this is
/// **not** `FLAG_SYSTEM`: a vendor-preinstalled app keeps an ordinary 10xxx uid
/// and stays targetable; only packages sharing a platform AID fall below.
fn is_app_uid(uid: u32) -> bool {
    uid % PER_USER_RANGE >= FIRST_APP_UID
}
const PM_READY_ATTEMPTS: u32 = 60;
const APATCH_TRUSTED_SU_KEY: &str = "su";
const SUPERCALL_HELLO: c_long = 0x1000;
const SUPERCALL_HELLO_MAGIC: c_long = 0x11581158;
const SUPERCALL_KPM_LOAD: c_long = 0x1020;
const SUPERCALL_KPM_CONTROL: c_long = 0x1022;
const SUPERCALL_KPM_LIST: c_long = 0x1031;

unsafe extern "C" {
    fn syscall(num: c_long, ...) -> c_long;
    fn flock(fd: c_int, operation: c_int) -> c_int;
}

const LOCK_EX: c_int = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PmReadyWait {
    Bounded(u32),
    Forever,
}

mod kpm;
mod lifecycle;
mod model;
mod ports;

use kpm::*;
pub use lifecycle::*;
pub use model::*;
use ports::*;

pub fn read_canonical() -> Result<String> {
    match fs::read_to_string(CANONICAL_CONFIG) {
        Ok(raw) => Ok(raw),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(empty_canonical_json().to_owned()),
        Err(e) => Err(e.into()),
    }
}

/// Read one optional feature without waiting for PackageManager. Kernel boot
/// loaders query their relevant features before loading the backend, so a
/// disabled load-time feature never registers its probes at all.
pub(crate) fn optional_feature_enabled(feature: &str) -> Result<bool> {
    let config = parse_canonical(&read_canonical()?)?;
    Ok(config.settings.optional_features.contains(feature))
}

/// Optional boot hardening is never a prerequisite for the baseline backend.
/// A corrupt desired-state file therefore falls back to the disabled default;
/// late-start config delivery still reports the parse error and refuses a
/// partial snapshot.
pub(crate) fn optional_feature_enabled_or_default(feature: &str) -> bool {
    match optional_feature_enabled(feature) {
        Ok(enabled) => enabled,
        Err(err) => {
            eprintln!(
                "cannot read optional boot feature {feature}; loading with the safe default: {err}"
            );
            false
        }
    }
}

pub fn activate_kmod() -> Result<()> {
    activate_kmod_with_pm_wait(PmReadyWait::Bounded(PM_READY_ATTEMPTS))
}

pub(crate) fn activate_kmod_boot() -> Result<()> {
    wait_for_path(KMOD_CTL);
    activate_kmod_with_pm_wait(PmReadyWait::Forever)
}

fn activate_kmod_with_pm_wait(wait: PmReadyWait) -> Result<()> {
    let wire = project_native_with_pm_wait(&read_canonical()?, NativeHookFamily::Kmod, wait)?;
    // /proc/vpnhide_ctl replaces the entire config per write(), so keep this
    // bounded to MAX_NATIVE_TARGETS and deliver one complete snapshot.
    fs::write(KMOD_CTL, wire)?;
    Ok(())
}

/// Deliver config to the in-tree (built-in) backend. Identical to the kmod config
/// path — same hook family, same /proc/vpnhide_ctl wire — because it owns the
/// same kernel hooks; only the boot lifecycle (no insmod) and detection differ.
/// The wire's `backend` field is set by the kernel driver, not here.
pub fn activate_builtin() -> Result<()> {
    activate_kmod_with_pm_wait(PmReadyWait::Bounded(PM_READY_ATTEMPTS))
}

pub(crate) fn activate_builtin_boot() -> Result<()> {
    wait_for_path(KMOD_CTL);
    activate_kmod_with_pm_wait(PmReadyWait::Forever)
}

pub fn activate_zygisk() -> Result<()> {
    activate_zygisk_with_pm_wait(PmReadyWait::Bounded(PM_READY_ATTEMPTS))
}

pub(crate) fn activate_zygisk_boot() -> Result<()> {
    activate_zygisk_with_pm_wait(PmReadyWait::Forever)
}

fn activate_zygisk_with_pm_wait(wait: PmReadyWait) -> Result<()> {
    let wire = project_native_with_pm_wait(&read_canonical()?, NativeHookFamily::Zygisk, wait)?;
    validate_zygisk_config_wire(&wire)?;
    write_atomic(Path::new(ZYGISK_RUNTIME_CONFIG), wire.as_bytes(), 0o644)
}

/// Zygisk installs hooks only in processes whose UID is explicitly listed.
/// A non-zero default means the opposite — act on every UID not listed — which
/// would require injecting the module into every app process. Refuse that wire
/// at the delivery boundary so a future whitelist producer cannot silently turn
/// the exception list back into a blacklist.
fn validate_zygisk_config_wire(wire: &str) -> Result<()> {
    let config = parse_config(wire.as_bytes()).ok_or("invalid Zygisk control payload")?;
    if config.default_mask != 0 {
        return Err(
            "Zygisk cannot apply a non-zero default hookmask; whitelist mode requires kmod or KPM"
                .into(),
        );
    }
    Ok(())
}

/// Outcome of a KPM boot activation. A kmod conflict is a legitimate,
/// non-error result (the KPM deliberately stands down — see docs/storage.md
/// §4.3), so it must
/// be distinguishable from a successful configure: the lifecycle layer records
/// each as a different typed `load_status`, and reporting a deferral as
/// "configured" would lie to the diagnostics screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KpmBootOutcome {
    /// Wire snapshot was delivered to the KPM over ctl0.
    Configured,
    /// The .ko backend is present, so the KPM stood down without loading or
    /// configuring (co-residence freezes the kernel — docs/storage.md §4.3).
    DeferredConflict,
    /// APatch/FolkPatch is installed, but neither a saved superkey nor the
    /// trusted `su` supercall grant authenticated yet. Boot may legitimately
    /// reach this state before the user unlocks the device and supplies the
    /// key, so the service records it as deferred rather than parsing an error
    /// message to guess what happened.
    AwaitingAuthentication,
    /// The running kernel's major.minor family has no validated KPM offset
    /// table. The KPM itself performs the same authoritative check at init;
    /// this userspace preflight exists so boot diagnostics can name the cause.
    UnsupportedKernel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct KpmBootReport {
    pub(crate) outcome: KpmBootOutcome,
    /// Exact boot-only loader choice used by this activation attempt. It is
    /// absent when conflict/kernel preflight stopped before canonical parsing.
    pub(crate) filesystem_hiding: Option<bool>,
}

impl KpmBootReport {
    fn before_config(outcome: KpmBootOutcome) -> Self {
        Self {
            outcome,
            filesystem_hiding: None,
        }
    }

    fn with_feature(outcome: KpmBootOutcome, filesystem_hiding: bool) -> Self {
        Self {
            outcome,
            filesystem_hiding: Some(filesystem_hiding),
        }
    }
}

pub fn activate_kpm() -> Result<()> {
    // App / manual path: a conflict is a hard error (conflict_is_error=true),
    // so this only ever returns Configured on success.
    activate_kpm_with_pm_wait(PmReadyWait::Bounded(PM_READY_ATTEMPTS), true).map(|_| ())
}

pub(crate) fn activate_kpm_boot() -> Result<KpmBootReport> {
    activate_kpm_with_pm_wait(PmReadyWait::Forever, false)
}

/// Load the KPM without waiting for PackageManager or delivering config.
/// The early lifecycle phase calls this during post-fs-data; configuration happens
/// later through `activate_kpm_boot` once package UIDs can be resolved.
pub(crate) fn load_kpm_boot() -> Result<KpmBootReport> {
    if skip_kpm_for_kmod_conflict(false)? {
        return Ok(KpmBootReport::before_config(
            KpmBootOutcome::DeferredConflict,
        ));
    }
    if !running_kernel_supports_kpm()? {
        return Ok(KpmBootReport::before_config(
            KpmBootOutcome::UnsupportedKernel,
        ));
    }
    let filesystem_hiding =
        optional_feature_enabled_or_default(OPTIONAL_FEATURE_FILESYSTEM_IFACE_PATHS);
    let client = match KpmClient::detect_outcome()? {
        KpmClientDetection::Ready(client) => client,
        KpmClientDetection::AwaitingAuthentication(_) => {
            return Ok(KpmBootReport::with_feature(
                KpmBootOutcome::AwaitingAuthentication,
                filesystem_hiding,
            ));
        }
    };
    client.ensure_loaded(KpmLoadOptions { filesystem_hiding })?;
    Ok(KpmBootReport::with_feature(
        KpmBootOutcome::Configured,
        filesystem_hiding,
    ))
}

pub fn read_kpm_status() -> Result<String> {
    read_kpm_payload("vpnhide 1 status")
}

pub fn read_kpm_stats() -> Result<String> {
    let client = KpmClient::detect()?;
    client.ctl0_stats()
}

pub fn read_kpm_state() -> Result<String> {
    let client = KpmClient::detect()?;
    let mut out = client.ctl0_read("vpnhide 1 status")?;
    out.push_str(&client.ctl0_stats()?);
    Ok(out)
}

fn read_kpm_payload(wire: &str) -> Result<String> {
    let client = KpmClient::detect()?;
    client.ctl0_read(wire)
}

fn activate_kpm_with_pm_wait(wait: PmReadyWait, conflict_is_error: bool) -> Result<KpmBootReport> {
    if skip_kpm_for_kmod_conflict(conflict_is_error)? {
        return Ok(KpmBootReport::before_config(
            KpmBootOutcome::DeferredConflict,
        ));
    }
    if !running_kernel_supports_kpm()? {
        if conflict_is_error {
            let release = running_kernel_release()?;
            return Err(format!(
                "unsupported kernel {release}; KPM supports {KPM_SUPPORTED_KERNEL_FAMILIES}"
            )
            .into());
        }
        return Ok(KpmBootReport::before_config(
            KpmBootOutcome::UnsupportedKernel,
        ));
    }
    let canonical = read_canonical()?;
    let filesystem_hiding = parse_canonical(&canonical)?
        .settings
        .optional_features
        .contains(OPTIONAL_FEATURE_FILESYSTEM_IFACE_PATHS);
    let wire = project_native_with_pm_wait(&canonical, NativeHookFamily::Kpm, wait)?;
    // Re-check after the (possibly long) PackageManager wait: the .ko may have
    // been loaded meanwhile, in which case we must not configure the KPM.
    if skip_kpm_for_kmod_conflict(conflict_is_error)? {
        return Ok(KpmBootReport::with_feature(
            KpmBootOutcome::DeferredConflict,
            filesystem_hiding,
        ));
    }
    let client = match KpmClient::detect_outcome()? {
        KpmClientDetection::Ready(client) => client,
        KpmClientDetection::AwaitingAuthentication(_) if !conflict_is_error => {
            return Ok(KpmBootReport::with_feature(
                KpmBootOutcome::AwaitingAuthentication,
                filesystem_hiding,
            ));
        }
        KpmClientDetection::AwaitingAuthentication(detail) => return Err(detail.into()),
    };
    client.ensure_loaded(KpmLoadOptions { filesystem_hiding })?;
    client.ctl0_config(&wire)?;
    Ok(KpmBootReport::with_feature(
        KpmBootOutcome::Configured,
        filesystem_hiding,
    ))
}

fn running_kernel_release() -> Result<String> {
    let out = Command::new("uname").arg("-r").output()?;
    if !out.status.success() {
        return Err(format!("uname -r failed with status {}", out.status).into());
    }
    let release = String::from_utf8(out.stdout)?.trim().to_owned();
    if release.is_empty() {
        return Err("uname -r returned an empty kernel release".into());
    }
    Ok(release)
}

fn running_kernel_supports_kpm() -> Result<bool> {
    let release = running_kernel_release()?;
    Ok(kernel_release_supports_kpm(&release))
}

fn kernel_release_supports_kpm(release: &str) -> bool {
    parse_kernel_family(release).is_some_and(|family| KPM_SUPPORTED_KERNEL_PAIRS.contains(&family))
}

/// Parse only the leading Linux major.minor pair. Android kernels commonly
/// append patchlevels and arbitrary vendor/KMI suffixes, which do not affect
/// KPM offset-table selection. Requiring a boundary after minor avoids prefix
/// confusion such as treating 6.10 as 6.1.
fn parse_kernel_family(release: &str) -> Option<(u32, u32)> {
    let bytes = release.trim().as_bytes();
    let major_end = bytes.iter().position(|byte| *byte == b'.')?;
    if major_end == 0 {
        return None;
    }
    let major = str::from_utf8(&bytes[..major_end]).ok()?.parse().ok()?;
    let minor_start = major_end + 1;
    let minor_len = bytes[minor_start..]
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if minor_len == 0 {
        return None;
    }
    let minor_end = minor_start + minor_len;
    if bytes
        .get(minor_end)
        .is_some_and(|byte| !matches!(byte, b'.' | b'-' | b'+'))
    {
        return None;
    }
    let minor = str::from_utf8(&bytes[minor_start..minor_end])
        .ok()?
        .parse()
        .ok()?;
    Some((major, minor))
}

pub fn activate_ports() -> Result<PortsActivationReport> {
    activate_ports_with_pm_wait(PmReadyWait::Bounded(PM_READY_ATTEMPTS))
}

pub(crate) fn activate_ports_boot() -> Result<PortsActivationReport> {
    activate_ports_with_pm_wait(PmReadyWait::Forever)
}

fn activate_ports_with_pm_wait(wait: PmReadyWait) -> Result<PortsActivationReport> {
    let rules = project_ports_with_pm_wait(&read_canonical()?, wait)?;
    apply_ports_rules(&rules)
}

pub fn activate_ports_recorded(boot_wait: bool) -> Result<()> {
    let source = if boot_wait { "boot" } else { "app" };
    let result = if boot_wait {
        activate_ports_boot()
    } else {
        activate_ports()
    };
    match result {
        Ok(report) => {
            write_ports_load_status(
                source,
                true,
                Some(report.target_count),
                "configured",
                &report.log,
            );
            Ok(())
        }
        Err(err) => {
            let detail = err.to_string();
            write_ports_load_status(source, false, None, &detail, &detail);
            Err(err)
        }
    }
}

fn has_native_targets(cfg: &CanonicalConfig, family: NativeHookFamily) -> bool {
    cfg.apps
        .values()
        .any(|app| app.native.hooks(family).is_some())
}

fn has_ports_targets(cfg: &CanonicalConfig) -> bool {
    cfg.apps.values().any(|app| app.ports)
}

fn empty_canonical_json() -> &'static str {
    "{\"version\":1,\"debug\":false,\"apps\":{},\"settings\":{\"rememberSuperkey\":false}}\n"
}

fn wait_for_pm_ready(wait: PmReadyWait) -> Result<()> {
    let mut attempts = 0;
    loop {
        attempts += 1;
        if let Ok(stdout) = pm_list_packages(&["list", "packages", "-U"])
            && pm_output_has_package(&stdout, APP_PACKAGE)
        {
            return Ok(());
        }
        if matches!(wait, PmReadyWait::Bounded(max) if attempts >= max) {
            return Err(
                format!("PackageManager did not expose {APP_PACKAGE} within {attempts}s").into(),
            );
        }
        thread::sleep(Duration::from_secs(1));
    }
}

fn wait_for_path(path: &str) {
    while !Path::new(path).exists() {
        thread::sleep(Duration::from_secs(1));
    }
}

fn pm_list_packages(args: &[&str]) -> Result<String> {
    let out = Command::new("pm").args(args).output()?;
    if !out.status.success() {
        return Err(format!("pm list packages failed with status {}", out.status).into());
    }
    Ok(String::from_utf8(out.stdout)?)
}

fn pm_list_users() -> Result<String> {
    let out = Command::new("pm").args(["list", "users"]).output()?;
    if !out.status.success() {
        return Err(format!("pm list users failed with status {}", out.status).into());
    }
    Ok(String::from_utf8(out.stdout)?)
}

fn pm_output_has_package(output: &str, package: &str) -> bool {
    let expected = format!("package:{package}");
    output
        .lines()
        .any(|line| line.split_whitespace().next() == Some(expected.as_str()))
}

/// True when the .ko backend is present and not disabled. This is the most
/// complete of the project's "is the kmod here?" checks: it catches both an
/// installed-and-enabled module directory *and* a live `/proc/vpnhide_ctl`
/// (e.g. a manually-loaded .ko whose module dir is gone). Both lifecycle phases
/// use this authoritative, ordering-independent gate. See docs/storage.md §4.3.
pub(crate) fn kmod_backend_present() -> bool {
    Path::new(KMOD_CTL).exists()
        || (Path::new(KMOD_MODULE_DIR).is_dir()
            && !Path::new(KMOD_MODULE_DIR).join("disable").exists())
}

fn skip_kpm_for_kmod_conflict(conflict_is_error: bool) -> Result<bool> {
    if !kmod_backend_present() {
        return Ok(false);
    }
    // .ko present. App/manual path treats this as a hard error; boot path
    // signals a (non-error) deferral to the caller.
    if conflict_is_error {
        return Err("kmod backend present; refusing to load/configure KPM".into());
    }
    Ok(true)
}

pub fn write_atomic(path: &Path, content: &[u8], mode: u32) -> Result<()> {
    let parent = path.parent().ok_or("path has no parent")?;
    fs::create_dir_all(parent)?;
    // Per-process temp name: the boot service and an app-triggered activation run
    // the same binary and both write e.g. load_status; a shared `.tmp` would let
    // them truncate/interleave each other. Each process gets its own temp and the
    // final rename stays atomic, so a concurrent run can only ever leave one
    // complete file, never a partial one.
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(content)?;
        file.sync_all()?;
    }
    fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))?;
    fs::rename(&tmp, path)?;
    Ok(())
}

// Locate the KPatch-Next `kpatch` CLI. This is the KPatch-Next-Module path only
// (Magisk / KSU / KSU-Next all install the same module — bin path confirmed on a
// Pixel 8 Pro). APatch is NOT covered here: it has no kpatch CLI on disk (the
// binary lives in the manager app's private libs) and loads KPMs via the
// supercall instead, so KpmClient::detect() routes APatch through that path.

#[cfg(test)]
mod tests;
