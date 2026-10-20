//! The diagnostics probes, in Rust. Core runs no shell: every section the app parses (and
//! every forensic section a bug report carries) is read here with std/libc, and an external
//! binary is spawned only where it *is* the interface: `cmd package` (PackageManager — never
//! `pm`, which AOSP ships as a `#!/system/bin/sh` wrapper around `cmd package`), `iptables` /
//! `ip6tables` (xtables has no stable netlink API), `ip` / `ss` / `dumpsys` / `logcat`
//! (their human-readable views are the forensic payload), `bootctl`, `magisk` / `ksud`
//! (version queries), `kpatch` (KernelPatch's own CLI) and core itself (`observe …`).
//! Every spawn is direct (no `sh -c`), bounded in time and output, and every program is an
//! ELF binary on AOSP (none is a shell script; see `docs/core-daemon.md`).

use std::io::Read;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub mod debug_snapshot;
pub mod inventory;
pub mod kernel;
pub mod lsposed;
pub mod root_snapshot;

#[cfg(test)]
mod tests;

/// Fixed device paths the probes read. `Default` is production; tests point it at a tree.
#[derive(Debug, Clone)]
pub struct Paths {
    pub modules: PathBuf,
    pub modules_update: PathBuf,
    pub config_file: PathBuf,
    pub superkey_file: PathBuf,
    pub core_boot_status: PathBuf,
    pub core_native_boot: PathBuf,
    pub core_log: PathBuf,
    pub zygisk_status: PathBuf,
    pub proc_ctl: PathBuf,
    pub proc_diag: PathBuf,
    pub lsposed_state: PathBuf,
    pub boot_id: PathBuf,
    pub apatch_dir: PathBuf,
    /// Pre-1.0 per-component lists (`LegacyConfigImport`), as (section, path).
    pub legacy: Vec<(&'static str, PathBuf)>,
    /// Core itself, for its `observe kpm-state|kpm-list` children.
    pub core_exe: PathBuf,
}

impl Default for Paths {
    fn default() -> Self {
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
        ];
        Paths {
            modules: "/data/adb/modules".into(),
            modules_update: "/data/adb/modules_update".into(),
            config_file: "/data/system/vpnhide_config.json".into(),
            superkey_file: crate::effects::SUPERKEY_FILE.into(),
            core_boot_status: "/data/adb/vpnhide/core/boot_status".into(),
            core_native_boot: std::path::Path::new(crate::service::PERSIST_DIR)
                .join(crate::native_boot::STATE_FILE_NAME),
            core_log: "/data/adb/vpnhide/core/core.log".into(),
            zygisk_status: "/data/user/0/dev.okhsunrog.vpnhide/files/vpnhide_zygisk_active".into(),
            proc_ctl: "/proc/vpnhide_ctl".into(),
            proc_diag: "/proc/vpnhide_diag".into(),
            lsposed_state: "/data/system/vpnhide_lsposed_state".into(),
            boot_id: crate::native_boot::BOOT_ID_PATH.into(),
            apatch_dir: "/data/adb/ap".into(),
            legacy: legacy.iter().map(|(n, p)| (*n, PathBuf::from(p))).collect(),
            core_exe: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("vpnhide-core")),
        }
    }
}

impl Paths {
    pub fn module(&self, id: &str) -> PathBuf {
        self.modules.join(id)
    }
}

/// Section framing for one probe's output, with a whole-probe deadline: once it passes,
/// remaining sections are skipped and the output is flagged truncated.
pub struct Out {
    pub buf: String,
    begin: &'static str,
    end: &'static str,
    deadline: Instant,
    pub truncated: bool,
}

impl Out {
    pub fn new(begin: &'static str, end: &'static str, budget: Duration) -> Self {
        Out {
            buf: String::new(),
            begin,
            end,
            deadline: Instant::now() + budget,
            truncated: false,
        }
    }

    pub fn expired(&mut self) -> bool {
        if Instant::now() >= self.deadline {
            self.truncated = true;
        }
        self.truncated
    }

    /// Time left for a child, never zero (a spawn needs some slice).
    pub fn remaining(&self, cap: Duration) -> Duration {
        self.deadline
            .saturating_duration_since(Instant::now())
            .min(cap)
            .max(Duration::from_millis(200))
    }

    /// A named section: begin marker, `body` (newline-terminated if non-empty), end marker.
    pub fn section(&mut self, name: &str, body: impl AsRef<str>) {
        if self.expired() {
            return;
        }
        self.buf.push_str(self.begin);
        self.buf.push_str(name);
        self.buf.push('\n');
        push_lines(&mut self.buf, body.as_ref());
        self.buf.push_str(self.end);
        self.buf.push_str(name);
        self.buf.push('\n');
    }

    pub fn raw_line(&mut self, line: &str) {
        self.buf.push_str(line);
        self.buf.push('\n');
    }
}

/// Append `text`, ending it with exactly the newline `cat`/`echo` would leave.
pub fn push_lines(buf: &mut String, text: &str) {
    if text.is_empty() {
        return;
    }
    buf.push_str(text);
    if !text.ends_with('\n') {
        buf.push('\n');
    }
}

/// `cat path` for a regular file, lossy UTF-8; `None` when absent or unreadable.
pub fn read_text(path: &Path) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(8 << 20)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// `cat` of a file that may not be regular (procfs nodes report size 0 / odd types).
pub fn read_node(path: &Path) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(8 << 20)
        .read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub fn flag(value: bool) -> &'static str {
    if value { "1" } else { "0" }
}

/// `[ -s path ]`.
pub fn non_empty(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() > 0)
}

/// What a bounded child produced. `exit` is 127 when the binary could not be spawned
/// (the shell's convention, which the app's parsers already know) and -1 on a timeout.
#[derive(Debug, Default, Clone)]
pub struct Ran {
    pub exit: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Ran {
    /// stdout then stderr, as `2>&1` folds them (ordering within a stream is kept).
    pub fn combined(&self) -> String {
        let mut text = self.stdout.clone();
        if !self.stderr.is_empty() {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&self.stderr);
        }
        text
    }
}

const TOOL_OUTPUT_MAX: usize = 16 << 20;

/// Spawn `program args…` directly (never through a shell), drain both streams up to a
/// cap and kill it at `timeout`.
pub fn run(program: &str, args: &[&str], timeout: Duration) -> Ran {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let Ok(mut child) = cmd.spawn() else {
        return Ran {
            exit: 127,
            stderr: format!("{program}: not found\n"),
            ..Ran::default()
        };
    };
    fn drain(mut r: impl Read + Send + 'static) -> std::thread::JoinHandle<Vec<u8>> {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = (&mut r).take(TOOL_OUTPUT_MAX as u64).read_to_end(&mut buf);
            // Keep draining so a chatty child never blocks on a full pipe.
            let _ = std::io::copy(&mut r, &mut std::io::sink());
            buf
        })
    }
    let out = drain(child.stdout.take().expect("piped"));
    let err = drain(child.stderr.take().expect("piped"));
    let deadline = Instant::now() + timeout;
    let exit = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code().unwrap_or(-1),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break -1;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => break -1,
        }
    };
    let join = |h: std::thread::JoinHandle<Vec<u8>>| {
        let limit = Instant::now() + Duration::from_secs(2);
        while !h.is_finished() && Instant::now() < limit {
            std::thread::sleep(Duration::from_millis(10));
        }
        if h.is_finished() {
            String::from_utf8_lossy(&h.join().unwrap_or_default()).into_owned()
        } else {
            String::new()
        }
    };
    Ran {
        exit,
        stdout: join(out),
        stderr: join(err),
    }
}

/// `$(cmd)`: stdout with trailing newlines stripped.
pub fn trimmed(text: &str) -> &str {
    text.trim_end_matches('\n')
}

/// The first executable named `name` on `PATH` (`command -v`).
pub fn which(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        let path = PathBuf::from(name);
        return is_executable(&path).then_some(path);
    }
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .map(|dir| dir.join(name))
        .find(|p| is_executable(p))
}

pub fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// KernelPatch's CLI (`kpatch`) as KPatch-Next ships it, if present.
pub fn kpatch_bin() -> Option<PathBuf> {
    [
        "kpatch",
        "/data/adb/modules/KPatch-Next/bin/kpatch",
        "/data/adb/modules/kpatch-next/bin/kpatch",
    ]
    .iter()
    .find_map(|c| which(c))
}

/// `ls -ldZ path`-style line: type+mode, links, uid, gid, size, SELinux label, path.
pub fn ls_line(path: &Path) -> String {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            let label = vpnhide_activator::sys::selinux_label(path).unwrap_or_else(|| "?".into());
            let mut line = format!(
                "{} {} {} {} {} {} {}",
                mode_string(&meta),
                meta.nlink(),
                meta.uid(),
                meta.gid(),
                meta.len(),
                label,
                path.display()
            );
            if meta.file_type().is_symlink()
                && let Ok(target) = std::fs::read_link(path)
            {
                line.push_str(&format!(" -> {}", target.display()));
            }
            line
        }
        Err(e) => format!("ls: {}: {e}", path.display()),
    }
}

fn mode_string(meta: &std::fs::Metadata) -> String {
    use std::os::unix::fs::FileTypeExt;
    let ft = meta.file_type();
    let kind = if ft.is_dir() {
        'd'
    } else if ft.is_symlink() {
        'l'
    } else if ft.is_block_device() {
        'b'
    } else if ft.is_char_device() {
        'c'
    } else if ft.is_fifo() {
        'p'
    } else if ft.is_socket() {
        's'
    } else {
        '-'
    };
    let mode = meta.permissions().mode();
    let mut s = String::from(kind);
    for shift in [6, 3, 0] {
        let bits = (mode >> shift) & 7;
        s.push(if bits & 4 != 0 { 'r' } else { '-' });
        s.push(if bits & 2 != 0 { 'w' } else { '-' });
        s.push(if bits & 1 != 0 { 'x' } else { '-' });
    }
    s
}

/// `ls -la dir` (with labels): one `ls_line` per entry, sorted, capped.
pub fn ls_dir(dir: &Path, cap: usize) -> String {
    let mut lines = vec![ls_line(dir)];
    match std::fs::read_dir(dir) {
        Ok(entries) => {
            let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
            paths.sort();
            lines.extend(paths.iter().take(cap).map(|p| ls_line(p)));
        }
        Err(e) => lines.push(format!("ls: {}: {e}", dir.display())),
    }
    lines.join("\n")
}

/// `sha256sum path` (`<hex>  <path>`), streaming.
pub fn sha256_line(path: &Path) -> String {
    use sha2::Digest;
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => return format!("sha256sum: {}: {e}", path.display()),
    };
    let mut hasher = sha2::Sha256::new();
    let mut chunk = vec![0u8; 1 << 20];
    loop {
        match file.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => hasher.update(&chunk[..n]),
            Err(e) => return format!("sha256sum: {}: {e}", path.display()),
        }
    }
    let hex: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("{hex}  {}", path.display())
}

/// The size of a file or block device (`blockdev --getsize64`): seek to its end.
pub fn device_size(path: &Path) -> Option<u64> {
    use std::io::Seek;
    let mut file = std::fs::File::open(path).ok()?;
    file.seek(std::io::SeekFrom::End(0)).ok()
}

/// `getenforce`, from selinuxfs.
pub fn getenforce() -> &'static str {
    match std::fs::read_to_string("/sys/fs/selinux/enforce") {
        Ok(v) if v.trim() == "1" => "Enforcing",
        Ok(_) => "Permissive",
        Err(_) => "Disabled",
    }
}

/// Milliseconds since an arbitrary origin, for phase timings.
pub fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// 1 when a module update is staged in modules_update/ awaiting the next reboot.
pub fn pending_update(paths: &Paths, id: &str) -> bool {
    let staged = paths.modules_update.join(id);
    staged.join("module.prop").is_file()
        || (paths.module(id).join("update").is_file() && staged.is_dir())
}
