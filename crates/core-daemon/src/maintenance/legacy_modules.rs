//! Exact project-module retirement tokens. Bundled payload alternatives never
//! enter this inventory. No caller supplies a module name or a removal path.
use super::image::{ImageFile, read_file};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{self, Read},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::Path,
    time::{Duration, Instant},
};

const IDS: [&str; 6] = [
    "vpnhide_kmod",
    "vpnhide_kpm",
    "vpnhide_zygisk",
    "vpnhide_builtin",
    "vpnhide_ports",
    "vpnhide_lsposed",
];
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "legacy module identity unavailable or changed",
    )
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyModuleFence {
    pub relative_path: String,
    pub root: (u64, u64),
    pub files: BTreeMap<String, ImageFile>,
    pub directories: BTreeMap<String, (u64, u64, u32)>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyInventory {
    pub modules: Vec<LegacyModuleFence>,
}

fn directory(path: &Path, uid: u32) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let m = file.metadata()?;
    if m.uid() != uid || m.mode() & 0o022 != 0 {
        return Err(invalid());
    }
    Ok(file)
}
fn capture(root: &Path, relative: &str, uid: u32) -> io::Result<LegacyModuleFence> {
    let dir = directory(&root.join(relative), uid)?;
    let meta = dir.metadata()?;
    let retained = dir.try_clone()?;
    let mut pending = vec![(dir, String::new())];
    let mut files = BTreeMap::new();
    let mut directories = BTreeMap::new();
    let mut remaining = 256usize;
    let mut bytes = 0u64;
    let deadline = Instant::now() + Duration::from_secs(30);
    while let Some((dir, parent)) = pending.pop() {
        for entry in std::fs::read_dir(format!("/proc/self/fd/{}", dir.as_raw_fd()))? {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "legacy inspection deadline",
                ));
            }
            remaining = remaining.checked_sub(1).ok_or_else(invalid)?;
            let name = entry?.file_name().into_string().map_err(|_| invalid())?;
            let path = if parent.is_empty() {
                name.clone()
            } else {
                format!("{parent}/{name}")
            };
            let name = std::ffi::CString::new(name).map_err(|_| invalid())?;
            let fd = unsafe {
                libc::openat(
                    dir.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_PATH | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                )
            };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let pinned = unsafe { File::from_raw_fd(fd) };
            let m = pinned.metadata()?;
            if m.uid() != uid || m.mode() & 0o022 != 0 {
                return Err(invalid());
            }
            if m.is_dir() {
                directories.insert(path.clone(), (m.dev(), m.ino(), m.mode()));
                pending.push((pinned, path));
                continue;
            }
            if !m.is_file() || m.nlink() != 1 {
                return Err(invalid());
            }
            if ["disable", "remove", "update"].contains(&path.as_str()) {
                if m.len() != 0 {
                    return Err(invalid());
                }
                continue;
            }
            bytes = bytes.checked_add(m.len()).ok_or_else(invalid)?;
            if bytes > 64 * 1024 * 1024 {
                return Err(invalid());
            }
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_CLOEXEC)
                .open(format!("/proc/self/fd/{}", pinned.as_raw_fd()))?;
            files.insert(path, read_file(&file, uid, 64 * 1024 * 1024, deadline)?);
        }
    }
    let id = relative.rsplit('/').next().ok_or_else(invalid)?;
    let mut text = String::new();
    crate::payloads::open_beneath(&retained, "module.prop")?
        .take(64 * 1024 + 1)
        .read_to_string(&mut text)?;
    if text.len() > 64 * 1024 {
        return Err(invalid());
    }
    let p = super::manager::simple_properties(&text)?;
    if p.get("id").map(String::as_str) != Some(id)
        || p.get("author").map(String::as_str) != Some("okhsunrog")
    {
        return Err(invalid());
    }
    Ok(LegacyModuleFence {
        relative_path: relative.into(),
        root: (meta.dev(), meta.ino()),
        files,
        directories,
    })
}
impl LegacyInventory {
    /// Post-boot readback, never a request to disable a newly discovered module.
    /// Known pre-boot retirement must be recorded separately by the executor.
    pub fn retirement_observed(&self, root: &Path) -> io::Result<bool> {
        self.retirement_observed_owned(root, 0)
    }
    pub(super) fn retirement_observed_owned(&self, root: &Path, uid: u32) -> io::Result<bool> {
        let current = Self::capture_owned(root, uid)?;
        for module in &current.modules {
            let id = module.relative_path.rsplit('/').next();
            let known = self.modules.iter().any(|old| {
                old.relative_path.rsplit('/').next() == id
                    && old.files.len() == module.files.len()
                    && old.files.iter().all(|(name, file)| {
                        module
                            .files
                            .get(name)
                            .is_some_and(|v| v.identity == file.identity && v.mode == file.mode)
                    })
                    && old.directories.keys().eq(module.directories.keys())
            });
            if !known {
                return Ok(false);
            }
            let dir = directory(&root.join(&module.relative_path), uid)?;
            for marker in ["disable", "remove"] {
                let file = match crate::payloads::open_beneath(&dir, marker) {
                    Ok(v) => v,
                    Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
                    Err(e) => return Err(e),
                };
                let m = file.metadata()?;
                if m.uid() != uid || m.mode() & 0o022 != 0 || m.nlink() != 1 || m.len() != 0 {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
    pub fn capture(root: &Path) -> io::Result<Self> {
        Self::capture_owned(root, 0)
    }
    pub(super) fn capture_owned(root: &Path, uid: u32) -> io::Result<Self> {
        let mut modules = Vec::new();
        for base in ["data/adb/modules", "data/adb/modules_update"] {
            for id in IDS {
                let relative = format!("{base}/{id}");
                match std::fs::symlink_metadata(root.join(&relative)) {
                    Ok(_) => modules.push(capture(root, &relative, uid)?),
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(Self { modules })
    }
    pub fn require_unchanged(&self, root: &Path) -> io::Result<()> {
        self.require_unchanged_owned(root, 0)
    }
    pub(super) fn require_unchanged_owned(&self, root: &Path, uid: u32) -> io::Result<()> {
        if self != &Self::capture_owned(root, uid)? {
            return Err(invalid());
        }
        Ok(())
    }
    /// Call only after syncing retirement intent and verifying the replacement.
    /// Root-manager marker semantics defer script/removal work until boot. Both
    /// live and staged old generations are marked so a staged copy cannot win.
    pub fn retire(&self, root: &Path) -> io::Result<()> {
        self.retire_owned(root, 0)
    }
    pub(super) fn retire_owned(&self, root: &Path, uid: u32) -> io::Result<()> {
        self.require_unchanged_owned(root, uid)?;
        for module in &self.modules {
            let dir = directory(&root.join(&module.relative_path), uid)?;
            let m = dir.metadata()?;
            if (m.dev(), m.ino()) != module.root {
                return Err(invalid());
            }
            for marker in [c"disable", c"remove"] {
                let fd = unsafe {
                    libc::openat(
                        dir.as_raw_fd(),
                        marker.as_ptr(),
                        libc::O_WRONLY
                            | libc::O_CREAT
                            | libc::O_EXCL
                            | libc::O_CLOEXEC
                            | libc::O_NOFOLLOW
                            | libc::O_NONBLOCK,
                        0o600,
                    )
                };
                if fd >= 0 {
                    unsafe { File::from_raw_fd(fd) }.sync_all()?;
                } else if io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists {
                    return Err(io::Error::last_os_error());
                }
                let name = marker.to_str().map_err(|_| invalid())?;
                let file = crate::payloads::open_beneath(&dir, name)?;
                let m = file.metadata()?;
                if m.uid() != uid || m.mode() & 0o022 != 0 || m.nlink() != 1 || m.len() != 0 {
                    return Err(invalid());
                }
            }
            dir.sync_all()?;
        }
        self.require_unchanged_owned(root, uid)
    }
}
