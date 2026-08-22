use std::ffi::CString;
use std::fs::{self, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::raw::{c_char, c_long, c_void};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::Duration;
use std::{env, ptr, thread};

use vpnhide_apatch_abi::{
    APATCH_SUPERCALL_NR, CommandStyle as ApatchCommandStyle,
    command_candidates as apatch_command_candidates_for_hint, encode_command as supercall_cmd,
    parse_kernel_version_hint as parse_apatch_kernel_version_hint,
};
use vpnhide_protocol::{KPM_ARGS_LEN, Kind, TELEMETRY_VERSION, parse_config, peek_kind};

use crate::{
    APATCH_DIR, APATCH_TRUSTED_SU_KEY, CHILD_COMMAND_TIMEOUT, KPM_CTL_LOCK, KPM_MODULE_FILE,
    KPM_NAME, KPM_TRUNCATION_MARKER, LOCK_EX, Result, SUPERCALL_HELLO, SUPERCALL_HELLO_MAGIC,
    SUPERCALL_KPM_CONTROL, SUPERCALL_KPM_LIST, SUPERCALL_KPM_LOAD, SUPERKEY_FILE, flock,
    output_with_timeout, syscall,
};

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
    if let Ok(key) = read_superkey() {
        keys.push(key);
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
        // lifecycle.rs instead passes an explicit `filesystem_hiding=0` — same
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
        if let Some(parent) = Path::new(KPM_CTL_LOCK).parent() {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(KPM_CTL_LOCK)?;
        fs::set_permissions(KPM_CTL_LOCK, fs::Permissions::from_mode(0o600))?;
        if unsafe { flock(file.as_raw_fd(), LOCK_EX) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(Self { _file: file })
    }
}

impl KpmClient {
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

    pub(crate) fn ensure_loaded(&self, options: KpmLoadOptions) -> Result<()> {
        if self.list_contains()? {
            return Ok(());
        }
        if !Path::new(KPM_MODULE_FILE).is_file() {
            return Err(format!("{KPM_MODULE_FILE} not found").into());
        }
        self.load(options)?;
        if self.list_contains()? {
            Ok(())
        } else {
            Err("kpm load returned success but vpnhide is not listed".into())
        }
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

    fn load(&self, options: KpmLoadOptions) -> Result<()> {
        match self {
            Self::KpatchCli { path } => {
                let mut cmd = Command::new(path);
                cmd.args(["kpm", "load", KPM_MODULE_FILE]);
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
                let path = CString::new(KPM_MODULE_FILE)?;
                let key = CString::new(key.as_str())?;
                let args = options.args().map(CString::new).transpose()?;
                let rc = unsafe {
                    syscall(
                        APATCH_SUPERCALL_NR,
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

    pub(crate) fn ctl0_config(&self, wire: &str) -> Result<()> {
        validate_kpm_config_wire(wire)?;
        let _lock = KpmCtlLock::acquire()?;
        const ATTEMPTS: usize = 4;
        for attempt in 0..ATTEMPTS {
            let result = match self {
                Self::KpatchCli { path } => run_kpatch_kpm_ctl0_config(path, wire),
                Self::ApatchSupercall { key, style } => apatch_kpm_ctl0_config(key, *style, wire),
            };
            match result {
                Ok(()) => return Ok(()),
                Err(_) if attempt + 1 < ATTEMPTS => {
                    // A concurrent boot/app ctl0 config gets the KPM's short
                    // busy return instead of spinning inside the kernel. The
                    // critical section is only a bounded target-array copy,
                    // so a brief retry also covers runtimes that flatten
                    // negative return codes to a generic CLI failure.
                    thread::sleep(Duration::from_millis(20));
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!()
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
        return Ok(false);
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    Ok(stdout.split_whitespace().any(|token| token == KPM_NAME))
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
            APATCH_SUPERCALL_NR,
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
            APATCH_SUPERCALL_NR,
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
            APATCH_SUPERCALL_NR,
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

fn apatch_kernel_version_hint() -> Option<c_long> {
    let out = Command::new("dmesg").output().ok()?;
    if !out.status.success() {
        return None;
    }
    parse_apatch_kernel_version_hint(&String::from_utf8_lossy(&out.stdout))
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
