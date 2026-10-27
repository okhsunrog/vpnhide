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
/// Historical Magisk id for the removed companion module. Builtin is detected
/// by control-node backend id; core delivers wire.
pub const BUILTIN_MODULE_DIR: &str = "/data/adb/modules/vpnhide_builtin";
pub const ZYGISK_RUNTIME_CONFIG: &str = "/data/adb/modules/vpnhide_zygisk/targets.txt";
pub const KPM_MODULE_FILE: &str = "/data/adb/modules/vpnhide_kpm/vpnhide.kpm";
pub const SUPERKEY_FILE: &str = "/data/adb/vpnhide/superkey";
const APATCH_DIR: &str = "/data/adb/ap";
const APP_PACKAGE: &str = "dev.okhsunrog.vpnhide";
const KPM_NAME: &str = "vpnhide";
pub const PORTS_CHAIN4: &str = "vpnhide_out";
pub const PORTS_CHAIN6: &str = "vpnhide_out6";
/// Marker file written by core when it has applied Ports rules (diagnostics).
pub const PORTS_CORE_OWNER_MARKER: &str = "/data/adb/vpnhide/core/ports_owned";
/// Marker: core owns the durable canonical config file writes.
pub const CONFIG_CORE_OWNER_MARKER: &str = "/data/adb/vpnhide/core/config_owned";
/// Marker: core owns Native (kmod/builtin/KPM) activation delivery.
pub const NATIVE_CORE_OWNER_MARKER: &str = "/data/adb/vpnhide/core/native_owned";
/// Marker: core owns Zygisk `targets.txt` publication.
pub const ZYGISK_CORE_OWNER_MARKER: &str = "/data/adb/vpnhide/core/zygisk_owned";
pub const CORE_OWNER_DIR_ENV: &str = "VPNHIDE_CORE_OWNER_DIR";
const KPM_CTL_LOCK: &str = "/data/adb/vpnhide_kpm/ctl.lock";
const KPM_TRUNCATION_MARKER: &str = "# vpnhide truncated";
pub const KPM_SUPPORTED_KERNEL_FAMILIES: &str =
    "4.4, 4.9, 4.14, 4.19, 5.4, 5.10, 5.15, 6.1, 6.6, 6.12";
const KPM_SUPPORTED_KERNEL_PAIRS: &[(u32, u32)] = &[
    (4, 4),
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
// i64, not c_long: these command words are fed to `supercall_cmd`
// (vpnhide_apatch_abi::encode_command), which encodes them as 64-bit
// (version << 32 | magic | cmd). c_long is 32-bit on armv7 (the zygisk activator
// ships 32-bit too), which would both fail to typecheck against the i64 API and
// truncate the encoding; on arm64 c_long is already i64, so nothing changes.
// SUPERCALL_HELLO_MAGIC stays c_long: it is only compared against the raw
// syscall return (also c_long), never encoded.
const SUPERCALL_HELLO: i64 = 0x1000;
const SUPERCALL_HELLO_MAGIC: c_long = 0x11581158;
const SUPERCALL_KPM_LOAD: i64 = 0x1020;
const SUPERCALL_KPM_CONTROL: i64 = 0x1022;
const SUPERCALL_KPM_LIST: i64 = 0x1031;
const SUPERCALL_KPM_INFO: i64 = 0x1032;

unsafe extern "C" {
    fn syscall(num: c_long, ...) -> c_long;
    fn flock(fd: c_int, operation: c_int) -> c_int;
}

const LOCK_EX: c_int = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PmReadyWait {
    Bounded(u32),
}

mod kernel_control;
mod kpm;
mod load_kmod;
pub(crate) use load_kmod::CHILD_COMMAND_TIMEOUT;
pub use process::{
    CapturedCommand, capture_command, output_with_input_timeout, output_with_timeout,
};
mod model;
mod ownership;
mod ports;
mod process;
pub mod sys;

use kpm::*;
pub use kpm::{
    CoreKpm, KpmConfigError, KpmInfo, KpmLiveCheck, KpmRuntimeProbe, MIN_KPM_VERSION,
    kpm_version_compatible, set_session_superkey,
};
pub use model::*;
pub use ownership::{
    config_owned_by_core, core_marker_path, native_owned_by_core, zygisk_owned_by_core,
};
pub use ports::{
    PORTS_OWNER_MARKER_ENV, PortsApplyResult, PortsChainPresence, PortsFamilyResult,
    apply_ports_rules_families, inspect_ports_chains, ports_owned_by_core, ports_owner_marker_path,
    write_ports_core_owner_marker,
};

// --- core-2.0 (§4): the persistent root core absorbs the activator, linking these instead
// of shelling out to the per-backend binaries. Additive pub surface over the pieces the
// binaries already compose (project a config to the native wire; deliver it to the control
// node, refusing a foreign owner). See core-daemon's activation phase.

/// The backend id the built-in / kmod control node reports when it is the in-tree backend.
pub const BUILTIN_BACKEND_ID: u32 = kernel_control::BUILTIN_BACKEND_ID;
/// The backend id the loadable `.ko` reports.
pub const KMOD_BACKEND_ID: u32 = kernel_control::KMOD_BACKEND_ID;

/// Project a canonical-config JSON string into the native control-v2 wire for one backend
/// family, resolving package uids from the live PackageManager (bounded wait). This is the
/// same projection the per-backend activator binaries run.
pub fn project_native_for_family(json: &str, family: NativeHookFamily) -> Result<String> {
    project_native_with_pm_wait(json, family, PmReadyWait::Bounded(PM_READY_ATTEMPTS))
}

/// Deliver a native control-v2 `wire` to the control node at `path`, first reading the node
/// and refusing unless it belongs to `expected_backend` (never configure a foreign owner).
/// `path` is explicit so the dev sandbox can target an isolated node, never the real
/// `/proc/vpnhide_ctl`.
pub fn deliver_kernel_config(path: &Path, expected_backend: u32, wire: &str) -> Result<()> {
    kernel_control::write_config(path, expected_backend, wire)
}

/// The backend id the control node at `path` currently reports, or `None` if the node is
/// absent. Read-only — safe against the live node.
pub fn observed_kernel_backend(path: &Path) -> Result<Option<u32>> {
    kernel_control::observed_backend(path)
}

/// Why a core-selected kmod load did not produce a loaded module. On device `exit` is the
/// `finit_module(2)` errno (e.g. 129 `EKEYREJECTED`, 126 `ENOKEY`, 8 `ENOEXEC`); 127 means
/// the artifact is missing and -1 a refusal before dispatch (control node already owned).
/// Host test builds load through an `insmod` test double and report its exit status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KmodLoadFailure {
    pub exit: i32,
    pub stderr: String,
}

impl std::fmt::Display for KmodLoadFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "kmod load exit {}: {}", self.exit, self.stderr.trim())
    }
}

/// Load the kmod artifact core selected for this boot. Core has already ruled out every
/// other native backend (including a live KPM), so unlike [`load_kernel_module`] this does
/// not re-probe KernelPatch; it still refuses a control node that is already owned.
pub fn insmod_selected_kmod(
    ctl: &Path,
    artifact: &Path,
    filesystem_hiding: bool,
) -> std::result::Result<(), KmodLoadFailure> {
    let refused = |e: Box<dyn Error + Send + Sync>| KmodLoadFailure {
        exit: -1,
        stderr: format!("kernel backend preflight refused loading: {e}"),
    };
    match kernel_control::observed_backend(ctl) {
        Ok(None) => {}
        Ok(Some(id)) => {
            return Err(refused(
                kernel_control::load_kmod_if_absent::<()>(Ok(Some(id)), || Ok(()))
                    .err()
                    .unwrap_or_else(|| "control node owned".into()),
            ));
        }
        Err(e) => return Err(refused(e)),
    }
    if !artifact.is_file() {
        return Err(KmodLoadFailure {
            exit: 127,
            stderr: format!("vpnhide_kmod.ko not found at {}", artifact.display()),
        });
    }
    load_kmod::load_kmod_artifact(artifact, filesystem_hiding)
        .map_err(|(exit, stderr)| KmodLoadFailure { exit, stderr })
}

/// The running kernel release and whether the KPM has an offset table for its family.
pub fn running_kernel_kpm_support() -> Result<(String, bool)> {
    let release = running_kernel_release()?;
    let supported = kernel_release_supports_kpm(&release);
    Ok((release, supported))
}

/// Block until PackageManager lists this app — however long boot takes. Core calls this
/// on its own boot thread, never on a request path.
pub fn wait_for_package_manager() {
    // Ready = PackageManager answers with its own platform package. Waiting for this
    // app's package instead would block forever when the app is not installed.
    loop {
        if let Ok(stdout) = pm_list_packages(&["list", "packages", "-U"])
            && (pm_output_has_package(&stdout, "android")
                || pm_output_has_package(&stdout, APP_PACKAGE))
        {
            return;
        }
        thread::sleep(Duration::from_secs(1));
    }
}

pub fn read_canonical() -> Result<String> {
    match fs::read_to_string(CANONICAL_CONFIG) {
        Ok(raw) => Ok(raw),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(empty_canonical_json().to_owned()),
        Err(e) => Err(e.into()),
    }
}

/// Zygisk installs hooks only in processes whose UID is explicitly listed.
/// A non-zero default means the opposite — act on every UID not listed — which
/// would require injecting the module into every app process. Refuse that wire
/// at the delivery boundary so a future whitelist producer cannot silently turn
/// the exception list back into a blacklist.
pub fn validate_zygisk_config_wire(wire: &str) -> Result<()> {
    let config = parse_config(wire.as_bytes()).ok_or("invalid Zygisk control payload")?;
    if config.default_mask != 0 {
        return Err(
            "Zygisk cannot apply a non-zero default hookmask; whitelist mode requires kmod or KPM"
                .into(),
        );
    }
    Ok(())
}

/// Read-only safety probe for core's central selector. Runtime/authentication
/// failures remain errors, rather than permission to select Zygisk.
pub fn observed_kpm_loaded() -> Result<bool> {
    kpm::KpmClient::observe_loaded()
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

fn running_kernel_release() -> Result<String> {
    // Host test builds only: the integration tests pin an Android kernel family instead
    // of inheriting the CI runner's. Device builds always read uname(2).
    #[cfg(not(target_os = "android"))]
    if let Some(release) = std::env::var_os("VPNHIDE_TEST_KERNEL_RELEASE") {
        return Ok(release.to_string_lossy().into_owned());
    }
    let release = sys::uname()?.release;
    if release.is_empty() {
        return Err("uname(2) returned an empty kernel release".into());
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

fn has_native_targets(cfg: &CanonicalConfig, family: NativeHookFamily) -> bool {
    cfg.apps
        .values()
        .any(|app| app.native.hooks(family).is_some())
}

pub fn has_ports_targets(cfg: &CanonicalConfig) -> bool {
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

/// PackageManager's shell command, reached through the `cmd` binder client. Never
/// `pm`: on AOSP `/system/bin/pm` is a `#!/system/bin/sh` wrapper around exactly this
/// call, and core spawns no shell.
pub const PACKAGE_SERVICE_CMD: &str = "cmd";
pub const PACKAGE_SERVICE_ARG: &str = "package";

fn package_command() -> Command {
    let mut command = Command::new(PACKAGE_SERVICE_CMD);
    command.arg(PACKAGE_SERVICE_ARG);
    command
}

fn pm_list_packages(args: &[&str]) -> Result<String> {
    let out = output_with_timeout(package_command().args(args), CHILD_COMMAND_TIMEOUT)?;
    if !out.status.success() {
        return Err(format!(
            "cmd package list packages failed with status {}",
            out.status
        )
        .into());
    }
    Ok(String::from_utf8(out.stdout)?)
}

fn pm_list_users() -> Result<String> {
    let out = output_with_timeout(
        package_command().args(["list", "users"]),
        CHILD_COMMAND_TIMEOUT,
    )?;
    if !out.status.success() {
        return Err(format!("cmd package list users failed with status {}", out.status).into());
    }
    Ok(String::from_utf8(out.stdout)?)
}

fn pm_output_has_package(output: &str, package: &str) -> bool {
    let expected = format!("package:{package}");
    output
        .lines()
        .any(|line| line.split_whitespace().next() == Some(expected.as_str()))
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
