//! Fixed manager commands through a retained, checked ELF. A successful child
//! exit is not image evidence; the replacement executor owns all readback.
use super::image::{ImageFile, read_file};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read},
    os::{
        fd::AsRawFd,
        unix::{
            fs::{MetadataExt, OpenOptionsExt},
            process::CommandExt,
        },
    },
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

const OUTPUT_LIMIT: usize = 64 * 1024;
const QUERY_LIMIT: Duration = Duration::from_secs(3);
const INSTALL_LIMIT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ManagerKind {
    KernelSu,
    APatch,
    Magisk,
}
impl ManagerKind {
    fn name(self) -> &'static str {
        match self {
            Self::KernelSu => "ksud",
            Self::APatch => "apd",
            Self::Magisk => "magisk",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagerIdentity {
    pub kind: ManagerKind,
    pub relative_path: String,
    pub discovery_path: String,
    pub image: ImageFile,
    pub version: String,
    pub installer_fingerprint: String,
}
pub struct Manager {
    root: PathBuf,
    file: File,
    pub identity: ManagerIdentity,
    uid: u32,
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "manager contract unavailable or changed",
    )
}

/// Check every resolved ancestor, then pin metadata without opening a device or
/// FIFO. The root argument is fixed by Core's production profile, never RPC.
fn executable(root: &Path, relative: &str, uid: u32) -> io::Result<(String, File, ImageFile)> {
    let resolved = std::fs::canonicalize(root.join(relative))?;
    let relative = resolved.strip_prefix(root).map_err(|_| invalid())?;
    let mut current = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root)?;
    let mut parent = PathBuf::new();
    for part in relative.components() {
        let std::path::Component::Normal(part) = part else {
            return Err(invalid());
        };
        let meta = current.metadata()?;
        // AOSP /data is system:system 0771, not root-only. Trust exactly
        // that platform parent; /data/adb and every executable descendant
        // still require the retained root-owned, non-writable boundary.
        let platform_uid = if uid == 0 { 1000 } else { uid };
        let platform_data = parent == Path::new("data")
            && meta.is_dir()
            && meta.uid() == platform_uid
            && meta.gid() == platform_uid
            && meta.mode() & 0o002 == 0;
        if !platform_data && (meta.uid() != uid || meta.mode() & 0o022 != 0) {
            return Err(invalid());
        }
        parent.push(part);
        let name = std::ffi::CString::new(part.as_encoded_bytes()).map_err(|_| invalid())?;
        let fd = unsafe {
            libc::openat(
                current.as_raw_fd(),
                name.as_ptr(),
                libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        use std::os::fd::FromRawFd;
        current = unsafe { File::from_raw_fd(fd) };
    }
    let meta = current.metadata()?;
    if !meta.is_file()
        || meta.uid() != uid
        || meta.mode() & 0o022 != 0
        || meta.mode() & 0o111 == 0
        || meta.nlink() != 1
    {
        return Err(invalid());
    }
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC)
        .open(format!("/proc/self/fd/{}", current.as_raw_fd()))?;
    let mut header = [0; 4];
    file.read_exact(&mut header)?;
    if header != *b"\x7fELF" {
        return Err(invalid());
    }
    let image = read_file(&file, uid, 64 * 1024 * 1024, Instant::now() + QUERY_LIMIT)?;
    let relative = relative.to_str().ok_or_else(invalid)?.to_owned();
    Ok((relative, file, image))
}
impl Manager {
    pub fn discover(root: &Path) -> io::Result<Self> {
        Self::discover_owned(root, 0)
    }
    fn discover_owned(root: &Path, uid: u32) -> io::Result<Self> {
        let mut found = Vec::<Self>::new();
        for (kind, path) in [
            (ManagerKind::KernelSu, "data/adb/ksud"),
            (ManagerKind::APatch, "data/adb/apd"),
            (ManagerKind::Magisk, "data/adb/magisk/magisk"),
            (ManagerKind::Magisk, "data/adb/magisk/magisk64"),
            (ManagerKind::Magisk, "debug_ramdisk/magisk"),
            (ManagerKind::Magisk, "sbin/magisk"),
        ] {
            let (relative_path, file, image) = match executable(root, path, uid) {
                Ok(v) => v,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            if found.iter().any(|v| {
                v.identity.image.device == image.device && v.identity.image.inode == image.inode
            }) {
                continue;
            }
            let mut manager = Self {
                root: root.into(),
                file,
                identity: ManagerIdentity {
                    kind,
                    relative_path,
                    discovery_path: path.into(),
                    image,
                    version: String::new(),
                    installer_fingerprint: String::new(),
                },
                uid,
            };
            let version = manager.query(if kind == ManagerKind::Magisk {
                &["-v"]
            } else {
                &["--version"]
            })?;
            if version.is_empty()
                || version.len() > 256
                || version.lines().count() != 1
                || version.chars().any(char::is_control)
            {
                return Err(invalid());
            }
            if kind != ManagerKind::Magisk && !version.starts_with(&format!("{} ", kind.name())) {
                return Err(invalid());
            }
            let help = manager.query(if kind == ManagerKind::Magisk {
                &["--help"]
            } else {
                &["module", "--help"]
            })?;
            let supported = if kind == ManagerKind::Magisk {
                help.contains("--install-module")
            } else {
                help.lines()
                    .any(|l| l.split_whitespace().next() == Some("install"))
                    && help
                        .lines()
                        .any(|l| l.split_whitespace().next() == Some("uninstall"))
            };
            if !supported {
                return Err(invalid());
            }
            if kind != ManagerKind::Magisk {
                let list = manager.query(&["module", "list"])?;
                let modules: Vec<serde_json::Value> =
                    serde_json::from_str(&list).map_err(|_| invalid())?;
                if modules.len() > 512
                    || modules
                        .iter()
                        .filter(|m| m.get("id").and_then(|v| v.as_str()) == Some("vpnhide_core"))
                        .count()
                        != 1
                {
                    return Err(invalid());
                }
            }
            manager.identity.version = version;
            manager.identity.installer_fingerprint = installer_fingerprint(root, uid)?;
            found.push(manager);
        }
        if found.len() != 1 {
            return Err(invalid());
        }
        found.pop().ok_or_else(invalid)
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(format!("/proc/self/fd/{}", self.file.as_raw_fd()));
        cmd.arg0(self.identity.kind.name())
            .args(args)
            .env_clear()
            .env("PATH", "/system/bin:/system/xbin:/vendor/bin")
            .env("ANDROID_ROOT", "/system")
            .env("ANDROID_DATA", "/data")
            .env("BOOTMODE", "true")
            .current_dir("/");
        cmd
    }
    fn query(&self, args: &[&str]) -> io::Result<String> {
        let result = vpnhide_bounded_process::capture_command(
            &mut self.command(args),
            None,
            QUERY_LIMIT,
            OUTPUT_LIMIT,
        )?;
        if result.timed_out || result.truncated || !result.output.status.success() {
            return Err(invalid());
        }
        let mut text = result.output.stdout;
        text.extend(result.output.stderr);
        Ok(String::from_utf8(text)
            .map_err(|_| invalid())?
            .trim()
            .to_owned())
    }
    pub fn require_unchanged(&self) -> io::Result<()> {
        let (relative, _, image) = executable(&self.root, &self.identity.discovery_path, self.uid)?;
        if image != self.identity.image || relative != self.identity.relative_path {
            return Err(invalid());
        }
        let pinned = read_file(
            &self.file,
            self.uid,
            64 * 1024 * 1024,
            Instant::now() + QUERY_LIMIT,
        )?;
        if pinned != image {
            return Err(invalid());
        }
        if installer_fingerprint(&self.root, self.uid)? != self.identity.installer_fingerprint {
            return Err(invalid());
        }
        Ok(())
    }
    /// Caller must persist dispatch intent first. Even an IO error may follow
    /// a started child and must be treated as uncertain by that durable owner.
    pub fn stage(&self, package: &File) -> io::Result<vpnhide_bounded_process::CapturedCommand> {
        let path = format!("/proc/{}/fd/{}", std::process::id(), package.as_raw_fd());
        let args = match self.identity.kind {
            ManagerKind::KernelSu | ManagerKind::APatch => vec!["module", "install", path.as_str()],
            ManagerKind::Magisk => vec!["--install-module", path.as_str()],
        };
        vpnhide_bounded_process::capture_command(
            &mut self.command(&args),
            None,
            INSTALL_LIMIT,
            OUTPUT_LIMIT,
        )
    }
}

/// Accept the ordinary module.prop subset, refusing rather than guessing Java
/// property escapes/continuations or alternative separators. Such input cannot
/// be treated as proof that the manager has no effective custom installer.
pub(super) fn simple_properties(
    raw: &str,
) -> io::Result<std::collections::BTreeMap<String, String>> {
    let mut properties = std::collections::BTreeMap::new();
    for line in raw
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with(['#', '!']))
    {
        if line.contains('\\') {
            return Err(invalid());
        }
        let (key, value) = line.split_once('=').ok_or_else(invalid)?;
        let key = key.trim();
        if key.is_empty()
            || !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            || properties.insert(key.into(), value.trim().into()).is_some()
        {
            return Err(invalid());
        }
    }
    Ok(properties)
}
fn exists(path: &Path) -> io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

fn installer_fingerprint(root: &Path, uid: u32) -> io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::collections::BTreeSet;
    let modules = root.join("data/adb/modules");
    let staged = root.join("data/adb/modules_update");
    let link = root.join("data/adb/metamodule");
    let mut candidates = BTreeSet::<String>::new();
    let mut linked = None;
    if exists(&link)? {
        let resolved = std::fs::canonicalize(&link)?;
        if resolved.parent() != Some(modules.as_path()) {
            return Err(invalid());
        }
        let id = resolved
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(invalid)?
            .to_owned();
        candidates.insert(id.clone());
        linked = Some(id);
    }
    let mut budget = 512usize;
    for base in [&modules, &staged] {
        let dir = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(base)
        {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        let meta = dir.metadata()?;
        if meta.uid() != uid || meta.mode() & 0o022 != 0 {
            return Err(invalid());
        }
        for entry in std::fs::read_dir(format!("/proc/self/fd/{}", dir.as_raw_fd()))? {
            budget = budget.checked_sub(1).ok_or_else(invalid)?;
            let entry = entry?;
            let id = entry.file_name().into_string().map_err(|_| invalid())?;
            let meta = entry.metadata()?;
            if !entry.file_type()?.is_dir() || meta.uid() != uid || meta.mode() & 0o022 != 0 {
                return Err(invalid());
            }
            let relative = format!("{id}/module.prop");
            let file = match crate::payloads::open_beneath(&dir, &relative) {
                Ok(v) => v,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            let meta = file.metadata()?;
            if meta.uid() != uid
                || meta.mode() & 0o022 != 0
                || meta.nlink() != 1
                || meta.len() > 64 * 1024
            {
                return Err(invalid());
            }
            let mut text = String::new();
            file.take(64 * 1024 + 1).read_to_string(&mut text)?;
            if text.len() > 64 * 1024 {
                return Err(invalid());
            }
            let properties = simple_properties(&text)?;
            if properties
                .get("metamodule")
                .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            {
                candidates.insert(id);
            }
        }
    }
    for id in &candidates {
        if id.is_empty()
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            || matches!(id.as_str(), "." | "..")
        {
            return Err(invalid());
        }
        for base in [&modules, &staged] {
            for script in ["metainstall.sh", "metauninstall.sh"] {
                if exists(&base.join(id).join(script))? {
                    return Err(invalid());
                }
            }
        }
    }
    let bytes = serde_json::to_vec(&(linked, candidates)).map_err(io::Error::other)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[cfg(test)]
pub(crate) mod tests;
