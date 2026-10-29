use std::ffi::CString;
use std::fs::{self, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::raw::{c_char, c_long, c_void};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::Mutex;
use std::{env, ptr};

use vpnhide_apatch_abi::{
    APATCH_SUPERCALL_NR, CommandStyle as ApatchCommandStyle,
    command_candidates as apatch_command_candidates_for_hint, encode_command as supercall_cmd,
    parse_kernel_version_hint as parse_apatch_kernel_version_hint,
};
use vpnhide_protocol::{KPM_ARGS_LEN, Kind, TELEMETRY_VERSION, parse_config, peek_kind};

use crate::{
    APATCH_DIR, APATCH_TRUSTED_SU_KEY, CHILD_COMMAND_TIMEOUT, KPM_CTL_LOCK, KPM_NAME,
    KPM_TRUNCATION_MARKER, LOCK_EX, Result, SUPERCALL_HELLO, SUPERCALL_HELLO_MAGIC,
    SUPERCALL_KPM_CONTROL, SUPERCALL_KPM_INFO, SUPERCALL_KPM_LIST, SUPERCALL_KPM_LOAD,
    SUPERKEY_FILE, flock, output_with_timeout, syscall,
};

// A non-persistent key belongs to this core process. It is never included in config,
// journals, diagnostics or command arguments; a restart requires authentication again.
static SESSION_SUPERKEY: Mutex<Option<String>> = Mutex::new(None);

pub fn set_session_superkey(key: Option<String>) {
    *SESSION_SUPERKEY.lock().expect("session superkey") = key;
}

fn find_kpatch() -> Option<PathBuf> {
    [
        "kpatch",
        "/data/adb/modules/KPatch-Next/bin/kpatch",
        "/data/adb/modules/kpatch-next/bin/kpatch",
    ]
    .into_iter()
    .find_map(|candidate| {
        if candidate.contains('/') {
            let p = PathBuf::from(candidate);
            p.is_file().then_some(p)
        } else {
            env::var_os("PATH").and_then(|paths| {
                env::split_paths(&paths)
                    .map(|dir| dir.join(candidate))
                    .find(|path| path.is_file())
            })
        }
    })
}

fn read_superkey() -> Result<String> {
    let key = fs::read_to_string(SUPERKEY_FILE)?;
    let key = key.trim().to_owned();
    if key.is_empty() {
        Err("superkey file is empty".into())
    } else {
        Ok(key)
    }
}

fn apatch_auth_candidates() -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(key) = SESSION_SUPERKEY.lock().expect("session superkey").clone() {
        keys.push(key);
    }
    if let Ok(key) = read_superkey() {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    if !keys.iter().any(|key| key == APATCH_TRUSTED_SU_KEY) {
        keys.push(APATCH_TRUSTED_SU_KEY.to_owned());
    }
    keys
}

pub(crate) enum KpmClient {
    KpatchCli {
        path: PathBuf,
    },
    ApatchSupercall {
        key: String,
        style: ApatchCommandStyle,
    },
}

pub(crate) enum KpmClientDetection {
    Ready(KpmClient),
    AwaitingAuthentication(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct KpmLoadOptions {
    pub(crate) filesystem_hiding: bool,
}

impl KpmLoadOptions {
    fn args(self) -> Option<&'static str> {
        // When off, omit the arg rather than passing `=0`: the KPM parser defaults
        // filesystem_hiding to off, so absence == disabled. (The .ko path in
        // core load path instead passes an explicit `filesystem_hiding=0` — same
        // intent, different loader ABIs.)
        self.filesystem_hiding.then_some("filesystem_hiding=1")
    }
}

#[cfg(test)]
mod load_options_tests {
    use super::KpmLoadOptions;

    #[test]
    fn filesystem_hiding_is_an_opt_in_load_argument() {
        assert_eq!(KpmLoadOptions::default().args(), None);
        assert_eq!(
            KpmLoadOptions {
                filesystem_hiding: true,
            }
            .args(),
            Some("filesystem_hiding=1")
        );
    }
}

/// Cross-process serialization for this project's KPM ctl0 callers. The
/// KernelPatch runtime stores ctl args in one module-owned buffer before
/// dispatching the handler, so boot activation, app reconciliation, and stats
/// reads must not enter ctl0 concurrently even though the handler also guards
/// its own live config snapshot.
struct KpmCtlLock {
    _file: fs::File,
}

impl KpmCtlLock {
    fn acquire() -> Result<Self> {
        Self::acquire_at(Path::new(KPM_CTL_LOCK), false)
    }

    fn acquire_at(path: &Path, nonblocking: bool) -> Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        // LOCK_NB is 4 on the supported Linux/Android targets. Core reports busy
        // rather than blocking its mutation owner behind a legacy process.
        let flags = LOCK_EX | if nonblocking { 4 } else { 0 };
        if unsafe { flock(file.as_raw_fd(), flags) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(Self { _file: file })
    }
}

/// One authenticated transport selected by core. It owns no policy or canonical
/// file: artifact/lock/control paths and projected wire come from the caller.
pub struct CoreKpm {
    client: KpmClient,
    lock_path: PathBuf,
}

#[derive(Debug)]
pub enum KpmConfigError {
    BeforeDispatch(String),
    /// The command was dispatched, but completion is not established. No retry.
    Unknown(String),
}

impl std::fmt::Display for KpmConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BeforeDispatch(message) | Self::Unknown(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for KpmConfigError {}

/// What core learns about KernelPatch before deciding the boot's native backend.
pub enum KpmRuntimeProbe {
    /// No APatch/FolkPatch directory and no KPatch-Next CLI.
    NoRuntime,
    /// An authenticated management channel.
    Ready(CoreKpm),
    /// APatch/FolkPatch is present but neither the saved superkey nor a trusted `su`
    /// grant authenticated.
    AwaitingAuthentication(String),
}

/// The oldest VPN Hide KPM core drives. 1.2.0 is the first release whose KPM speaks
/// the control-v2 config snapshot and telemetry-v1 status core uses (the decimal
/// load-argument format was removed then); every later release kept that wire.
pub const MIN_KPM_VERSION: &str = "1.2.0";

/// KernelPatch's `get_module_info` for a loaded module (`name=`, `version=`, `args=`
/// lines). `args` are the load arguments, so they carry `filesystem_hiding=1` when the
/// module was loaded with it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KpmInfo {
    pub name: String,
    pub version: String,
    pub args: String,
}

impl KpmInfo {
    pub fn parse(text: &str) -> Option<Self> {
        let mut info = KpmInfo::default();
        let mut named = false;
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim().to_owned();
            match key.trim() {
                "name" => {
                    info.name = value;
                    named = true;
                }
                "version" => info.version = value,
                "args" => info.args = value,
                _ => {}
            }
        }
        named.then_some(info)
    }

    /// The load-time filesystem-hiding choice, read back from the load arguments.
    /// Absence means off: core omits the argument when it is disabled.
    pub fn filesystem_hiding(&self) -> bool {
        self.args
            .split(|c: char| c.is_whitespace() || c == ',')
            .any(|arg| arg == "filesystem_hiding=1")
    }
}

/// Whether a KPM build version (`X.Y.Z`, optionally `-N-gSHA[-dirty]`) is at least
/// [`MIN_KPM_VERSION`]. `None` when the version is not of that shape.
pub fn kpm_version_compatible(version: &str) -> Option<bool> {
    fn triple(v: &str) -> Option<(u32, u32, u32)> {
        let core = v.trim().trim_start_matches('v');
        let core = core.split(['-', '+']).next()?;
        let mut parts = core.split('.');
        let parsed = (
            parts.next()?.parse().ok()?,
            parts.next()?.parse().ok()?,
            parts.next()?.parse().ok()?,
        );
        parts.next().is_none().then_some(parsed)
    }
    Some(triple(version)? >= triple(MIN_KPM_VERSION)?)
}

/// What core learns about the VPN Hide KPM that is live in KernelPatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KpmLiveCheck {
    /// No `vpnhide` module is loaded.
    NotLoaded,
    /// Loaded, answers the telemetry-v1 status handshake as the KPM backend, and its
    /// version (when KernelPatch reports one) is supported. `info` is `None` when the
    /// runtime cannot report module info (a CLI without `kpm info`).
    Compatible { info: Option<KpmInfo> },
    /// Loaded, but core cannot drive it: too old, an unparseable version, or no valid
    /// status handshake. `found` is the version KernelPatch reported, if any.
    Incompatible {
        found: Option<String>,
        detail: String,
    },
}

impl CoreKpm {
    /// Like [`Self::connect`], but keeps an authentication refusal distinct from other
    /// probe failures so core can tell "awaiting the superkey" from "broken runtime".
    pub fn probe(lock_path: &Path) -> Result<KpmRuntimeProbe> {
        if !Path::new(APATCH_DIR).is_dir() && find_kpatch().is_none() {
            return Ok(KpmRuntimeProbe::NoRuntime);
        }
        Ok(match KpmClient::detect_outcome()? {
            KpmClientDetection::Ready(client) => KpmRuntimeProbe::Ready(Self {
                client,
                lock_path: lock_path.to_owned(),
            }),
            KpmClientDetection::AwaitingAuthentication(detail) => {
                KpmRuntimeProbe::AwaitingAuthentication(detail)
            }
        })
    }

    /// None is definite runtime absence. Authentication/probe errors stay errors.
    pub fn connect(lock_path: &Path) -> Result<Option<Self>> {
        if !Path::new(APATCH_DIR).is_dir() && find_kpatch().is_none() {
            return Ok(None);
        }
        Ok(Some(Self {
            client: KpmClient::detect()?,
            lock_path: lock_path.to_owned(),
        }))
    }

    /// Read-only `kpm_state` text for diagnostics: the status reply followed by the
    /// stats pages, as the app's parsers read them. `None` when vpnhide is not loaded.
    /// Each read takes the shared KPM lock, so this never races a config dispatch.
    /// The whole snapshot shares one child-call budget; one already-running
    /// bounded transport call may overrun it. No detached reader survives timeout.
    pub fn read_state(&self) -> Result<Option<String>> {
        self.read_state_with_budget(CHILD_COMMAND_TIMEOUT)
    }

    fn read_state_with_budget(&self, budget: std::time::Duration) -> Result<Option<String>> {
        let deadline = std::time::Instant::now() + budget;
        let check = || -> Result<()> {
            if std::time::Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "KPM state snapshot deadline exceeded",
                )
                .into());
            }
            Ok(())
        };
        check()?;
        let _lock = KpmCtlLock::acquire_at(&self.lock_path, true)?;
        check()?;
        let loaded = self.client.list_contains()?;
        check()?;
        if !loaded {
            return Ok(None);
        }
        // Keep the configured lock across the complete snapshot. Client read
        // helpers take the legacy default lock and must not nest inside it.
        let read = |wire: &str| -> Result<String> {
            check()?;
            let reply = self.client.ctl0_read_raw(wire)?;
            check()?;
            Ok(reply)
        };
        let mut out = normalize_kpm_reply("vpnhide 1 status", read("vpnhide 1 status")?)?;
        out.push_str(&collect_kpm_stats_pages(read)?);
        Ok(Some(out))
    }

    /// Runtime evidence, not module-directory presence. No canonical read or load.
    pub fn status(&self) -> Result<Option<vpnhide_protocol::Status>> {
        let _lock = KpmCtlLock::acquire_at(&self.lock_path, true)?;
        self.status_locked()
    }

    /// Identify the live `vpnhide` KPM — loaded by core, by APatch autoload or by hand —
    /// before core adopts it: the status handshake must answer as the KPM backend, and
    /// a reported version must be at least [`MIN_KPM_VERSION`]. Transport failures stay
    /// errors (the runtime could not be queried).
    pub fn verify_live(&self) -> Result<KpmLiveCheck> {
        let _lock = KpmCtlLock::acquire_at(&self.lock_path, true)?;
        if !self.client.list_contains()? {
            return Ok(KpmLiveCheck::NotLoaded);
        }
        let info = self.client.info().ok().flatten();
        let found = info.as_ref().map(|i| i.version.clone());
        if let Some(version) = found.clone() {
            match kpm_version_compatible(&version) {
                Some(true) => {}
                Some(false) => {
                    return Ok(KpmLiveCheck::Incompatible {
                        found,
                        detail: format!(
                            "the loaded vpnhide KPM is version {version}; core needs {MIN_KPM_VERSION} or newer"
                        ),
                    });
                }
                None => {
                    return Ok(KpmLiveCheck::Incompatible {
                        found,
                        detail: format!(
                            "the loaded vpnhide KPM reports an unrecognised version '{version}'"
                        ),
                    });
                }
            }
        }
        match self.status_locked() {
            Ok(Some(_)) => Ok(KpmLiveCheck::Compatible { info }),
            Ok(None) => Ok(KpmLiveCheck::NotLoaded),
            Err(e) => Ok(KpmLiveCheck::Incompatible {
                found,
                detail: format!("the loaded vpnhide KPM failed the status handshake: {e}"),
            }),
        }
    }

    fn status_locked(&self) -> Result<Option<vpnhide_protocol::Status>> {
        if !self.client.list_contains()? {
            return Ok(None);
        }
        let reply = self.client.ctl0_read_raw("vpnhide 1 status")?;
        if !reply.ends_with('\n') {
            return Err("truncated KPM status".into());
        }
        let status = vpnhide_protocol::parse_status_fields(reply.as_bytes())
            .and_then(|fields| fields.complete())
            .ok_or("invalid KPM status")?;
        if status.backend != vpnhide_protocol::hook_ids::Backend::Kpm as u32 {
            return Err("foreign KPM control owner".into());
        }
        Ok(Some(status))
    }

    /// Load only the artifact selected by core; never select neighbors or fallback.
    pub fn load_selected(
        &self,
        ctl: &Path,
        artifact: &Path,
        filesystem_hiding: bool,
    ) -> Result<()> {
        let _lock = KpmCtlLock::acquire_at(&self.lock_path, true)?;
        require_no_kernel_owner(ctl)?;
        if self.status_locked()?.is_some() {
            return Ok(());
        }
        if !crate::running_kernel_supports_kpm()? {
            return Err(format!(
                "unsupported KPM kernel; supported families: {}",
                crate::KPM_SUPPORTED_KERNEL_FAMILIES
            )
            .into());
        }
        if !artifact.is_file() {
            return Err("selected KPM artifact is missing".into());
        }
        require_no_kernel_owner(ctl)?;
        self.client
            .load_from(artifact, KpmLoadOptions { filesystem_hiding })?;
        self.status_locked()?
            .ok_or("KPM load did not produce a compatible runtime identity")?;
        Ok(())
    }

    /// Identity validation and dispatch share the KPM lock. Unlike the legacy
    /// activator this sends at most one config command; uncertain errors stay unknown.
    pub fn configure_once(
        &self,
        ctl: &Path,
        wire: &str,
    ) -> std::result::Result<(), KpmConfigError> {
        let preflight = || -> Result<KpmCtlLock> {
            validate_kpm_config_wire(wire)?;
            if parse_config(wire.as_bytes()).is_none() {
                return Err("invalid KPM config snapshot".into());
            }
            let lock = KpmCtlLock::acquire_at(&self.lock_path, true)?;
            require_no_kernel_owner(ctl)?;
            let status = self.status_locked()?.ok_or("KPM is not loaded")?;
            use vpnhide_protocol::hook_ids::StatusError;
            if status.error != StatusError::Ok as u32
                && status.error != StatusError::PartialHooks as u32
            {
                return Err(format!("KPM is not ready: status error {}", status.error).into());
            }
            require_no_kernel_owner(ctl)?;
            Ok(lock)
        };
        let _lock = preflight().map_err(|e| KpmConfigError::BeforeDispatch(e.to_string()))?;
        let result = match &self.client {
            KpmClient::KpatchCli { path } => run_kpatch_kpm_ctl0_config(path, wire),
            KpmClient::ApatchSupercall { key, style } => apatch_kpm_ctl0_config(key, *style, wire),
        };
        result.map_err(|e| KpmConfigError::Unknown(e.to_string()))
    }
}

fn require_no_kernel_owner(ctl: &Path) -> Result<()> {
    if crate::observed_kernel_backend(ctl)?.is_some() {
        return Err("kernel control owner is present; KPM load/config refused".into());
    }
    Ok(())
}

impl KpmClient {
    /// No runtime is different from a runtime whose loaded modules cannot be
    /// observed. The latter must prevent a conflicting kmod load.
    pub(crate) fn observe_loaded() -> Result<bool> {
        if !Path::new(APATCH_DIR).is_dir() && find_kpatch().is_none() {
            return Ok(false);
        }
        Self::detect()?.list_contains()
    }

    pub(crate) fn detect() -> Result<Self> {
        match Self::detect_outcome()? {
            KpmClientDetection::Ready(client) => Ok(client),
            KpmClientDetection::AwaitingAuthentication(detail) => Err(detail.into()),
        }
    }

    pub(crate) fn detect_outcome() -> Result<KpmClientDetection> {
        if Path::new(APATCH_DIR).is_dir() {
            let mut failures = Vec::new();
            for key in apatch_auth_candidates() {
                match apatch_probe(&key) {
                    Ok(style) => {
                        return Ok(KpmClientDetection::Ready(Self::ApatchSupercall {
                            key,
                            style,
                        }));
                    }
                    Err(e) => {
                        let label = if key == APATCH_TRUSTED_SU_KEY {
                            "trusted su"
                        } else {
                            "saved key"
                        };
                        failures.push(format!("{label}: {e}"));
                    }
                }
            }
            return Ok(KpmClientDetection::AwaitingAuthentication(format!(
                "APatch/FolkPatch KPM requires a valid saved superkey at {SUPERKEY_FILE} \
                 or a trusted '{APATCH_TRUSTED_SU_KEY}' supercall grant (attempts: {})",
                failures.join("; ")
            )));
        }
        let path = find_kpatch().ok_or("kpatch CLI not found")?;
        kpatch_hello(&path)?;
        Ok(KpmClientDetection::Ready(Self::KpatchCli { path }))
    }

    fn list_contains(&self) -> Result<bool> {
        match self {
            Self::KpatchCli { path } => kpatch_kpm_list_contains(path),
            Self::ApatchSupercall { key, style } => {
                let list = apatch_kpm_list(key, *style)?;
                Ok(list.split_whitespace().any(|token| token == KPM_NAME))
            }
        }
    }

    /// KernelPatch's module info for `vpnhide`. `Ok(None)` when the runtime has no way
    /// to report it (a `kpatch` CLI without `kpm info`).
    fn info(&self) -> Result<Option<KpmInfo>> {
        let text = match self {
            Self::KpatchCli { path } => match kpatch_kpm_info(path) {
                Ok(text) => text,
                Err(_) => return Ok(None),
            },
            Self::ApatchSupercall { key, style } => apatch_kpm_info(key, *style)?,
        };
        Ok(KpmInfo::parse(&text).filter(|info| info.name == KPM_NAME))
    }

    fn load_from(&self, artifact: &Path, options: KpmLoadOptions) -> Result<()> {
        match self {
            Self::KpatchCli { path } => {
                let mut cmd = Command::new(path);
                cmd.args(["kpm", "load"]).arg(artifact);
                if let Some(args) = options.args() {
                    cmd.arg(args);
                }
                let out = output_with_timeout(&mut cmd, CHILD_COMMAND_TIMEOUT)?;
                if out.status.success() {
                    Ok(())
                } else {
                    Err(format!("kpm load failed with status {}", out.status).into())
                }
            }
            Self::ApatchSupercall { key, style } => {
                let auth = auth_label(key);
                use std::os::unix::ffi::OsStrExt;
                let path = CString::new(artifact.as_os_str().as_bytes())?;
                let key = CString::new(key.as_str())?;
                let args = options.args().map(CString::new).transpose()?;
                let rc = unsafe {
                    syscall(
                        APATCH_SUPERCALL_NR as c_long,
                        key.as_ptr(),
                        supercall_cmd(*style, SUPERCALL_KPM_LOAD),
                        path.as_ptr(),
                        args.as_ref().map_or(ptr::null(), |args| args.as_ptr()),
                        ptr::null_mut::<c_void>(),
                    )
                };
                supercall_ok(rc, "kpm load", auth)
            }
        }
    }

    pub(crate) fn ctl0_read(&self, wire: &str) -> Result<String> {
        let _lock = KpmCtlLock::acquire()?;
        normalize_kpm_reply(wire, self.ctl0_read_raw(wire)?)
    }

    pub(crate) fn ctl0_stats(&self) -> Result<String> {
        let _lock = KpmCtlLock::acquire()?;
        collect_kpm_stats_pages(|wire| self.ctl0_read_raw(wire))
    }

    fn ctl0_read_raw(&self, wire: &str) -> Result<String> {
        match self {
            Self::KpatchCli { path } => run_kpatch_kpm_ctl0_read_raw(path, wire),
            Self::ApatchSupercall { key, style } => apatch_kpm_ctl0_read_raw(key, *style, wire),
        }
    }
}

/// Reject a config before KernelPatch copies it through `char args[1024]`.
/// `compat_strncpy_from_user` silently truncates an overlong argument and uses
/// one byte for the trailing NUL, so sending 1024 bytes would otherwise reach
/// the module as a malformed prefix and hide the useful capacity error.
pub(crate) fn validate_kpm_config_wire(wire: &str) -> Result<()> {
    if wire.len() >= KPM_ARGS_LEN {
        return Err(format!(
            "KPM config is {} bytes; the transport accepts at most {} bytes plus a trailing NUL; reduce the native target count or the number of distinct per-app hook selections",
            wire.len(),
            KPM_ARGS_LEN - 1,
        )
        .into());
    }
    Ok(())
}

fn kpatch_kpm_list_contains(kpatch: &Path) -> Result<bool> {
    let mut cmd = Command::new(kpatch);
    cmd.args(["kpm", "list"]);
    let out = output_with_timeout(&mut cmd, CHILD_COMMAND_TIMEOUT)?;
    if !out.status.success() {
        return Err(format!("kpatch kpm list failed with status {}", out.status).into());
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    Ok(stdout.split_whitespace().any(|token| token == KPM_NAME))
}

fn kpatch_kpm_info(kpatch: &Path) -> Result<String> {
    let mut cmd = Command::new(kpatch);
    cmd.args(["kpm", "info", KPM_NAME]);
    let out = output_with_timeout(&mut cmd, CHILD_COMMAND_TIMEOUT)?;
    // Like ctl0 reads, the CLI may exit with the supercall's byte count: trust stdout.
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    if text.contains("name=") {
        Ok(text)
    } else {
        Err(format!("kpatch kpm info failed with status {}", out.status).into())
    }
}

fn kpatch_hello(kpatch: &Path) -> Result<()> {
    let mut cmd = Command::new(kpatch);
    cmd.arg("hello");
    let out = output_with_timeout(&mut cmd, CHILD_COMMAND_TIMEOUT)?;
    if out.status.success() && !String::from_utf8_lossy(&out.stdout).trim().is_empty() {
        Ok(())
    } else {
        Err(format!(
            "KernelPatch inactive or kpatch hello failed with status {}",
            out.status
        )
        .into())
    }
}

fn run_kpatch_kpm_ctl0_config(kpatch: &Path, wire: &str) -> Result<()> {
    let mut cmd = Command::new(kpatch);
    cmd.args(["kpm", "ctl0", KPM_NAME, wire]);
    let out = output_with_timeout(&mut cmd, CHILD_COMMAND_TIMEOUT)?;
    if kpatch_ctl0_config_status_ok(out.status, wire) {
        Ok(())
    } else {
        Err(format!("kpm ctl0 failed with status {}", out.status).into())
    }
}

fn run_kpatch_kpm_ctl0_read_raw(kpatch: &Path, wire: &str) -> Result<String> {
    let mut cmd = Command::new(kpatch);
    cmd.args(["kpm", "ctl0", KPM_NAME, wire]);
    let out = output_with_timeout(&mut cmd, CHILD_COMMAND_TIMEOUT)?;
    // The kpatch CLI prints the reply to stdout (`fprintf(stdout, "%s", buf)`)
    // and exits with the supercall's return value — for a READ that is the reply
    // BYTE COUNT (e.g. 64), NOT 0. So a non-zero exit is the normal success case
    // here; treating it as failure (the old behaviour) dropped every status/stats
    // read on KPatch-Next, leaving the dashboard with no KPM stats. On a real
    // error the supercall returns a negative rc and never fills the buffer, so
    // stdout is empty. Trust stdout: the reply text is authoritative, the exit
    // code is not (it can't even round-trip a reply longer than 255 bytes).
    let reply = String::from_utf8_lossy(&out.stdout).into_owned();
    if reply.is_empty() {
        Err("KPM ctl0 returned an empty reply".into())
    } else {
        Ok(reply)
    }
}

/// Validate a KPM readback and preserve the protocol's missing-newline
/// truncation signal as an explicit comment. KPatch's CLI and the APatch client
/// both cap ctl0 replies at 4096 bytes, so retrying with a larger userspace
/// buffer is not portable. Keeping the complete-line prefix plus a marker lets
/// the app retain backend status while refusing to total partial counters.
pub(crate) fn normalize_kpm_reply(wire: &str, mut reply: String) -> Result<String> {
    if reply.is_empty() {
        return Err("KPM ctl0 returned an empty reply".into());
    }
    let expected = peek_kind(wire.as_bytes()).ok_or("invalid KPM ctl0 request header")?;
    let actual = peek_kind(reply.as_bytes()).ok_or("invalid KPM ctl0 reply header")?;
    if actual != expected || !matches!(actual, Kind::Status | Kind::Stats) {
        return Err(format!("unexpected KPM ctl0 reply kind {actual:?} for {expected:?}").into());
    }
    if !reply.ends_with('\n') {
        reply.push('\n');
        reply.push_str(KPM_TRUNCATION_MARKER);
        reply.push('\n');
    }
    Ok(reply)
}

const MAX_KPM_STATS_PAGES: usize = 512;

#[derive(Debug, PartialEq, Eq)]
enum StatsPageNext {
    Cursor(u32),
    Done,
}

#[derive(Debug, PartialEq, Eq)]
struct StatsPage {
    rows: Vec<String>,
    next: StatsPageNext,
}

fn parse_prefixed_hex_u64(token: &str) -> Option<u64> {
    let digits = token
        .strip_prefix("0x")
        .or_else(|| token.strip_prefix("0X"))?;
    (!digits.is_empty())
        .then(|| u64::from_str_radix(digits, 16).ok())
        .flatten()
}

fn parse_prefixed_hex_u32(token: &str) -> Option<u32> {
    parse_prefixed_hex_u64(token).and_then(|value| u32::try_from(value).ok())
}

/// Parse and validate the additive KPM `page` trailer. `None` is a complete
/// legacy reply from an older backend; a present trailer is strict so a broken
/// cursor cannot silently duplicate or omit cumulative counters.
fn parse_kpm_stats_page(reply: &str, requested: u32) -> Result<Option<StatsPage>> {
    if peek_kind(reply.as_bytes()) != Some(Kind::Stats) {
        return Err("invalid KPM stats reply header".into());
    }
    let lines = reply.lines().map(str::trim).collect::<Vec<_>>();
    let page_positions = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.split_whitespace().next() == Some("page"))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if page_positions.is_empty() {
        return Ok(None);
    }
    if page_positions.len() != 1 || page_positions[0] + 1 != lines.len() {
        return Err("KPM stats reply has a misplaced or repeated page trailer".into());
    }

    let trailer = lines[page_positions[0]]
        .split_whitespace()
        .collect::<Vec<_>>();
    if trailer.len() != 4 {
        return Err("malformed KPM stats page trailer".into());
    }
    let echoed = parse_prefixed_hex_u32(trailer[1]).ok_or("invalid echoed stats cursor")?;
    if echoed != requested {
        return Err(
            format!("KPM stats page echoed cursor 0x{echoed:x}, expected 0x{requested:x}").into(),
        );
    }
    let declared_rows =
        parse_prefixed_hex_u32(trailer[3]).ok_or("invalid stats page row count")? as usize;

    let mut rows = Vec::new();
    let mut previous_uid = requested;
    for line in &lines[1..page_positions[0]] {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields = line.split_whitespace().collect::<Vec<_>>();
        let uid = fields
            .first()
            .and_then(|token| parse_prefixed_hex_u32(token))
            .ok_or("invalid UID in KPM stats page")?;
        if uid <= previous_uid || fields.len() < 2 {
            return Err("KPM stats page UIDs are not strictly increasing".into());
        }
        for field in &fields[1..] {
            let (hook, count) = field
                .split_once(':')
                .ok_or("invalid hook counter in KPM stats page")?;
            parse_prefixed_hex_u32(hook).ok_or("invalid hook id in KPM stats page")?;
            let count = parse_prefixed_hex_u64(count).ok_or("invalid count in KPM stats page")?;
            if count == 0 {
                return Err("zero count in sparse KPM stats page".into());
            }
        }
        previous_uid = uid;
        rows.push((*line).to_owned());
    }
    if rows.len() != declared_rows {
        return Err(format!(
            "KPM stats page declared {declared_rows} rows but carried {}",
            rows.len()
        )
        .into());
    }

    let next = if trailer[2] == "done" {
        if !reply.ends_with('\n') {
            return Err("final KPM stats page is missing its newline".into());
        }
        StatsPageNext::Done
    } else {
        let next = parse_prefixed_hex_u32(trailer[2]).ok_or("invalid next stats cursor")?;
        if next <= requested || next < previous_uid {
            return Err("KPM stats page cursor did not advance".into());
        }
        if reply.ends_with('\n') {
            return Err("non-final KPM stats page unexpectedly ends with a newline".into());
        }
        StatsPageNext::Cursor(next)
    };
    Ok(Some(StatsPage { rows, next }))
}

pub(crate) fn collect_kpm_stats_pages(
    mut read: impl FnMut(&str) -> Result<String>,
) -> Result<String> {
    let mut cursor = 0u32;
    let mut aggregate = format!("vpnhide {TELEMETRY_VERSION} stats\n");

    for page_index in 0..MAX_KPM_STATS_PAGES {
        let request = if page_index == 0 {
            aggregate.trim_end().to_owned()
        } else {
            format!("vpnhide {} stats\nafter 0x{cursor:x}\n", TELEMETRY_VERSION)
        };
        let reply = read(&request)?;
        let Some(page) = parse_kpm_stats_page(&reply, cursor)? else {
            if page_index != 0 {
                return Err("KPM stopped paginating stats mid-read".into());
            }
            return normalize_kpm_reply(&request, reply);
        };
        for row in page.rows {
            aggregate.push_str(&row);
            aggregate.push('\n');
        }
        match page.next {
            StatsPageNext::Done => return Ok(aggregate),
            StatsPageNext::Cursor(next) => cursor = next,
        }
    }
    Err(format!("KPM stats exceeded {MAX_KPM_STATS_PAGES} pages").into())
}

pub(crate) fn kpatch_ctl0_config_status_ok(status: ExitStatus, wire: &str) -> bool {
    if status.success() {
        return true;
    }
    let Some(expected_targets) = parse_config(wire.as_bytes()).map(|cfg| cfg.targets.len()) else {
        return false;
    };
    // Compatibility with older vpnhide KPM builds: `ctl0 config` returned the
    // applied target count, which KPatch-Next exposes as the shell exit status.
    status.code() == Some(expected_targets as i32)
}

fn apatch_probe(key: &str) -> Result<ApatchCommandStyle> {
    let candidates = apatch_command_candidates();
    let mut failures = Vec::new();
    for style in candidates {
        let rc = apatch_hello(key, style)?;
        if rc == SUPERCALL_HELLO_MAGIC {
            return Ok(style);
        }
        failures.push(format!("{style:?}: rc={rc}"));
    }
    Err(format!(
        "KernelPatch inactive or bad APatch SuperKey (hello attempts: {})",
        failures.join(", ")
    )
    .into())
}

fn apatch_hello(key: &str, style: ApatchCommandStyle) -> Result<c_long> {
    let key = CString::new(key)?;
    let rc = unsafe {
        syscall(
            APATCH_SUPERCALL_NR as c_long,
            key.as_ptr(),
            supercall_cmd(style, SUPERCALL_HELLO),
        )
    };
    Ok(rc)
}

fn apatch_kpm_list(key: &str, style: ApatchCommandStyle) -> Result<String> {
    let auth = auth_label(key);
    let key = CString::new(key)?;
    let mut buf = [0u8; 4096];
    let rc = unsafe {
        syscall(
            APATCH_SUPERCALL_NR as c_long,
            key.as_ptr(),
            supercall_cmd(style, SUPERCALL_KPM_LIST),
            buf.as_mut_ptr().cast::<c_char>(),
            buf.len() as c_long,
        )
    };
    supercall_ok(rc, "kpm list", auth)?;
    let len = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    Ok(String::from_utf8_lossy(&buf[..len]).into_owned())
}

fn apatch_kpm_info(key: &str, style: ApatchCommandStyle) -> Result<String> {
    let auth = auth_label(key);
    let key = CString::new(key)?;
    let name = CString::new(KPM_NAME)?;
    let mut buf = [0u8; 2048];
    let rc = unsafe {
        syscall(
            APATCH_SUPERCALL_NR as c_long,
            key.as_ptr(),
            supercall_cmd(style, SUPERCALL_KPM_INFO),
            name.as_ptr(),
            buf.as_mut_ptr().cast::<c_char>(),
            buf.len() as c_long,
        )
    };
    supercall_ok(rc, "kpm info", auth)?;
    let len = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    Ok(String::from_utf8_lossy(&buf[..len]).into_owned())
}

fn apatch_kpm_ctl0_config(key: &str, style: ApatchCommandStyle, wire: &str) -> Result<()> {
    let (rc, _) = apatch_kpm_ctl0_raw(key, style, wire)?;
    supercall_ok(rc, "kpm ctl0", auth_label(key))
}

fn apatch_kpm_ctl0_read_raw(key: &str, style: ApatchCommandStyle, wire: &str) -> Result<String> {
    let (rc, out) = apatch_kpm_ctl0_raw(key, style, wire)?;
    supercall_ok(rc, "kpm ctl0", auth_label(key))?;
    let len = apatch_output_len(rc, &out);
    if len == 0 {
        Err("KPM ctl0 returned an empty reply".into())
    } else {
        Ok(String::from_utf8_lossy(&out[..len]).into_owned())
    }
}

fn apatch_kpm_ctl0_raw(
    key: &str,
    style: ApatchCommandStyle,
    wire: &str,
) -> Result<(c_long, [u8; 4096])> {
    let key = CString::new(key)?;
    let name = CString::new(KPM_NAME)?;
    let wire = CString::new(wire)?;
    let mut out = [0u8; 4096];
    let rc = unsafe {
        syscall(
            APATCH_SUPERCALL_NR as c_long,
            key.as_ptr(),
            supercall_cmd(style, SUPERCALL_KPM_CONTROL),
            name.as_ptr(),
            wire.as_ptr(),
            out.as_mut_ptr().cast::<c_char>(),
            out.len() as c_long,
        )
    };
    Ok((rc, out))
}

fn apatch_output_len(rc: c_long, out: &[u8]) -> usize {
    if rc > 0 {
        return usize::try_from(rc).unwrap_or(out.len()).min(out.len());
    }
    out.iter().position(|b| *b == 0).unwrap_or(0)
}

pub(crate) fn apatch_command_candidates() -> Vec<ApatchCommandStyle> {
    apatch_command_candidates_for_hint(apatch_kernel_version_hint())
}

fn apatch_kernel_version_hint() -> Option<i64> {
    parse_apatch_kernel_version_hint(&crate::sys::kernel_log().ok()?)
}

/// A refused supercall says nothing about *why* on its own: the same rc comes
/// back for a kernel without KPM support and for a key the runtime will not
/// accept for module management. `auth` names the credential that was used, so
/// a bug report distinguishes "the saved SuperKey was rejected" from "we only
/// had the trusted-su grant" without a second capture.
fn supercall_ok(rc: c_long, op: &str, auth: &str) -> Result<()> {
    if rc >= 0 {
        Ok(())
    } else {
        Err(format!("{op} supercall failed with rc={rc} (auth: {auth})").into())
    }
}

/// How the APatch supercall authenticated — the saved SuperKey, or KernelPatch's
/// trusted-`su` grant, which authenticates a hello ping but is not necessarily
/// accepted for KPM management.
fn auth_label(key: &str) -> &'static str {
    if key == APATCH_TRUSTED_SU_KEY {
        "trusted su"
    } else {
        "saved superkey"
    }
}

#[cfg(test)]
mod identity_tests {
    use super::{KpmInfo, kpm_version_compatible};

    #[test]
    fn module_info_parses_kernelpatch_get_module_info() {
        let text = "name=vpnhide\nversion=1.3.1-12-gabcdef0-dirty\nlicense=GPL v2\nauthor=okhsunrog\ndescription=Hide VPN\nargs=filesystem_hiding=1\nload_event=post-fs-data\nload_source=manual\n";
        let info = KpmInfo::parse(text).unwrap();
        assert_eq!(info.name, "vpnhide");
        assert_eq!(info.version, "1.3.1-12-gabcdef0-dirty");
        assert!(info.filesystem_hiding());
        let plain = KpmInfo::parse("name=vpnhide\nversion=1.3.1\nargs=\n").unwrap();
        assert!(!plain.filesystem_hiding());
        assert_eq!(KpmInfo::parse("garbage"), None);
    }

    #[test]
    fn version_floor_is_the_control_v2_release() {
        assert_eq!(kpm_version_compatible("1.2.0"), Some(true));
        assert_eq!(kpm_version_compatible("1.3.1-4-g1234567"), Some(true));
        assert_eq!(kpm_version_compatible("v2.0.0"), Some(true));
        assert_eq!(kpm_version_compatible("1.1.1"), Some(false));
        assert_eq!(kpm_version_compatible("0.9.9"), Some(false));
        assert_eq!(kpm_version_compatible("dev"), None);
        assert_eq!(kpm_version_compatible("1.2"), None);
    }

    #[test]
    fn state_snapshot_refuses_a_busy_configured_lock_before_runtime_access() {
        use super::{CoreKpm, KpmClient, KpmCtlLock};
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "vpnhide-kpm-state-lock-{}-{nonce}",
            std::process::id()
        ));
        let held = KpmCtlLock::acquire_at(&path, true).unwrap();
        let client = CoreKpm {
            client: KpmClient::KpatchCli {
                path: "/must-not-be-executed".into(),
            },
            lock_path: path.clone(),
        };
        let error = client.read_state().unwrap_err();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::WouldBlock
        );
        drop(held);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn concurrent_observation_makes_delivery_fail_before_dispatch_without_retry() {
        use super::{CoreKpm, KpmClient, KpmConfigError};
        use std::os::unix::fs::PermissionsExt;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "vpnhide-observe-deliver-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&root).unwrap();
        let program = root.join("kpatch");
        std::fs::write(
            &program,
            r#"#!/bin/sh
case "$1 $2" in
  "kpm list")
    : > "${0%/*}/observing"
    while [ ! -f "${0%/*}/release" ]; do sleep 0.01; done
    exit 0 ;;
  "kpm ctl0") : > "${0%/*}/dispatched" ;;
esac
"#,
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let client = || CoreKpm {
            client: KpmClient::KpatchCli {
                path: program.clone(),
            },
            lock_path: root.join("control.lock"),
        };
        let observer = client();
        let reader = std::thread::spawn(move || {
            observer.read_state_with_budget(std::time::Duration::from_secs(3))
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !root.join("observing").exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(root.join("observing").exists());
        let result = client().configure_once(
            &root.join("no-kernel-ctl"),
            "vpnhide 2 config\ndebug 0\nend 0\n",
        );
        std::fs::write(root.join("release"), "").unwrap();
        assert!(reader.join().unwrap().unwrap().is_none());
        assert!(
            matches!(result, Err(KpmConfigError::BeforeDispatch(ref detail)) if detail.contains("os error 11")),
            "{result:?}"
        );
        assert!(
            !root.join("dispatched").exists(),
            "contention must not dispatch or retry a config"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn state_snapshot_deadline_stops_multi_page_reads_without_leaving_a_worker() {
        use super::{CoreKpm, KpmClient};
        use std::os::unix::fs::PermissionsExt;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "vpnhide-kpm-state-budget-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&root).unwrap();
        let program = root.join("kpatch");
        std::fs::write(&program, r#"#!/bin/sh
case "$1 $2" in
  "kpm list") printf 'vpnhide\n' ;;
  "kpm ctl0") case "$4" in
    "vpnhide 1 status") printf 'vpnhide 1 status\nbackend 0x1\nkver 0x60100\nhooks 0x3ff\nerror 0x0\n' ;;
    "vpnhide 1 stats"*)
      printf 'page\n' >> "${0%/*}/pages"
      sleep 0.35
      printf 'vpnhide 1 stats\n0x2800 0x0:0x1\npage 0x0 0x2800 0x1' ;;
  esac ;;
esac
"#).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let client = CoreKpm {
            client: KpmClient::KpatchCli { path: program },
            lock_path: root.join("control.lock"),
        };
        let error = client
            .read_state_with_budget(std::time::Duration::from_millis(250))
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::TimedOut
        );
        assert_eq!(
            std::fs::read_to_string(root.join("pages")).unwrap(),
            "page\n"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
