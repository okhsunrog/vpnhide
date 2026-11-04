//! A staged image is packaging evidence, never active backend observation.
//! Manager-mutated module.prop/update/disable/remove are deliberately excluded
//! from the code-generation fence. Executable, catalog and payloads are not.
use crate::payloads::{Abi, Catalog, PayloadKind, open_beneath};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{self},
    os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt},
    path::Path,
    time::{Duration, Instant},
};
use vpnhide_package_trust::{FileIdentity, VerifiedDescriptor};

const IMAGE_BUDGET: Duration = Duration::from_secs(30);

/// Enumerate beneath the retained directory, never a manager-selected pathname.
/// Exact files and their derived directory prefixes bound the traversal; no
/// unknown subtree (including an empty one) or symlink can become executable
/// manager input alongside the signed runtime members.
pub(super) fn require_layout(
    directory: &File,
    allowed: &std::collections::BTreeSet<String>,
    large_files: &std::collections::BTreeSet<String>,
    uid: u32,
    deadline: Instant,
) -> io::Result<()> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let mut pending = vec![(directory.try_clone()?, String::new())];
    let directories = allowed
        .iter()
        .flat_map(|file| file.match_indices('/').map(|(at, _)| file[..at].to_owned()))
        .collect::<std::collections::BTreeSet<_>>();
    let mut remaining = allowed.len() + directories.len();
    while let Some((dir, parent)) = pending.pop() {
        // /proc/self/fd opens this pinned directory, not its possibly replaced
        // filesystem name. This is also the private-package enumerator's model.
        for entry in std::fs::read_dir(format!("/proc/self/fd/{}", dir.as_raw_fd()))? {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Core layout inspection deadline",
                ));
            }
            remaining = remaining.checked_sub(1).ok_or_else(invalid)?;
            let name = entry?.file_name().into_string().map_err(|_| invalid())?;
            let path = if parent.is_empty() {
                name.clone()
            } else {
                format!("{parent}/{name}")
            };
            let is_directory = directories.contains(&path);
            if !is_directory && !allowed.contains(&path) {
                return Err(invalid());
            }
            let name = std::ffi::CString::new(name).map_err(|_| invalid())?;
            // Pin metadata without opening a device driver or waiting for a
            // FIFO writer. O_PATH directories also support descriptor-relative
            // traversal; symlinks remain symlinks and fail the type checks.
            let fd = unsafe {
                libc::openat(
                    dir.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_PATH
                        | libc::O_CLOEXEC
                        | libc::O_NOFOLLOW
                        | if is_directory { libc::O_DIRECTORY } else { 0 },
                )
            };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let file = unsafe { File::from_raw_fd(fd) };
            let meta = file.metadata()?;
            if meta.uid() != uid || meta.mode() & 0o022 != 0 {
                return Err(invalid());
            }
            if is_directory {
                if !meta.is_dir() {
                    return Err(invalid());
                }
                pending.push((file, path));
            } else if !meta.is_file()
                || meta.nlink() != 1
                || (path == "update" && meta.len() != 0)
                || (!large_files.contains(&path) && meta.len() > 64 * 1024)
            {
                return Err(invalid());
            }
        }
    }
    Ok(())
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageFile {
    pub device: u64,
    pub inode: u64,
    pub identity: FileIdentity,
    pub modified: (i64, i64),
    pub changed: (i64, i64),
    pub mode: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageFence {
    pub root: (u64, u64),
    pub files: BTreeMap<String, ImageFile>,
    #[serde(default)]
    verified_layout: bool,
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Core image generation is incomplete or changed",
    )
}
fn entry_exists(directory: &File, name: &std::ffi::CStr) -> io::Result<bool> {
    use std::os::fd::AsRawFd;
    let mut meta = std::mem::MaybeUninit::<libc::stat>::uninit();
    let rc = unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            name.as_ptr(),
            meta.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if rc == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::NotFound {
        Ok(false)
    } else {
        Err(error)
    }
}
pub(super) fn read_file(
    file: &File,
    uid: u32,
    limit: u64,
    deadline: Instant,
) -> io::Result<ImageFile> {
    let before = file.metadata()?;
    if !before.is_file()
        || before.uid() != uid
        || before.mode() & 0o022 != 0
        || before.nlink() != 1
        || before.len() == 0
        || before.len() > limit
    {
        return Err(invalid());
    }
    let mut hash = Sha256::new();
    let mut count = 0_u64;
    let mut block = [0_u8; 64 * 1024];
    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Core image inspection deadline",
            ));
        }
        let n = file.read_at(&mut block, count)?;
        if n == 0 {
            break;
        }
        count += n as u64;
        if count > before.len() {
            return Err(invalid());
        }
        hash.update(&block[..n]);
    }
    let after = file.metadata()?;
    if count != before.len()
        || before.len() != after.len()
        || before.mode() != after.mode()
        || before.uid() != after.uid()
        || before.nlink() != after.nlink()
        || (
            before.mtime(),
            before.mtime_nsec(),
            before.ctime(),
            before.ctime_nsec(),
        ) != (
            after.mtime(),
            after.mtime_nsec(),
            after.ctime(),
            after.ctime_nsec(),
        )
    {
        return Err(invalid());
    }
    Ok(ImageFile {
        device: before.dev(),
        inode: before.ino(),
        identity: FileIdentity {
            bytes: count,
            sha256: format!("{:x}", hash.finalize()),
        },
        modified: (before.mtime(), before.mtime_nsec()),
        changed: (before.ctime(), before.ctime_nsec()),
        mode: before.mode(),
    })
}
impl ImageFence {
    pub(crate) fn verified_layout(&self) -> bool {
        self.verified_layout
    }
    pub(crate) fn same_content(&self, other: &Self) -> bool {
        self.files.len() == other.files.len()
            && self.files.iter().all(|(name, file)| {
                other
                    .files
                    .get(name)
                    .is_some_and(|v| v.identity == file.identity && v.mode == file.mode)
            })
    }
    /// Managers choose the installed userspace ABI. Qualify the real daemon
    /// against either signed identity rather than confusing the old Core's
    /// binary architecture with the device kernel or the next daemon ABI.
    pub(super) fn inspect_replacement(
        root: &Path,
        descriptor: &VerifiedDescriptor,
        uid: u32,
    ) -> io::Result<Self> {
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(root)?;
        let actual = read_file(
            &open_beneath(&directory, "vpnhide-core")?,
            uid,
            128 * 1024 * 1024,
            Instant::now() + IMAGE_BUDGET,
        )?;
        let runtime = &descriptor.descriptor().runtime;
        let abi = match (
            actual.identity == runtime.daemon_arm64,
            actual.identity == runtime.daemon_arm,
        ) {
            (true, false) => Abi::Arm64,
            (false, true) => Abi::Arm32,
            _ => return Err(invalid()),
        };
        Self::inspect(root, Some(descriptor), abi, uid)
    }
    pub fn capture_active(root: &Path) -> io::Result<Self> {
        // Without a replacement descriptor, inspection hashes the actual
        // running ABI's daemon bytes; the ABI selector is unused in this case.
        Self::inspect(root, None, Abi::Arm64, 0)
    }
    pub fn verify_replacement(
        root: &Path,
        descriptor: &VerifiedDescriptor,
        abi: Abi,
    ) -> io::Result<Self> {
        Self::inspect(root, Some(descriptor), abi, 0)
    }
    pub fn require_unchanged(&self, root: &Path) -> io::Result<()> {
        self.require_unchanged_owned(root, 0)
    }
    pub(super) fn require_unchanged_owned(&self, root: &Path, uid: u32) -> io::Result<()> {
        if self != &Self::inspect_layout(root, None, Abi::Arm64, uid, self.verified_layout)? {
            return Err(invalid());
        }
        Ok(())
    }
    /// Verify the actual executing inode, not just a path whose bytes may have
    /// been replaced underneath an older supervisor/worker.
    pub fn require_running(&self, running: &File) -> io::Result<()> {
        self.require_running_owned(running, 0)
    }
    pub(super) fn require_running_owned(&self, running: &File, uid: u32) -> io::Result<()> {
        let expected = self.files.get("vpnhide-core").ok_or_else(invalid)?;
        let actual = read_file(
            running,
            uid,
            128 * 1024 * 1024,
            Instant::now() + IMAGE_BUDGET,
        )?;
        if expected != &actual {
            return Err(invalid());
        }
        Ok(())
    }
    pub(super) fn inspect(
        root: &Path,
        descriptor: Option<&VerifiedDescriptor>,
        abi: Abi,
        uid: u32,
    ) -> io::Result<Self> {
        Self::inspect_layout(root, descriptor, abi, uid, descriptor.is_some())
    }
    fn inspect_layout(
        root: &Path,
        descriptor: Option<&VerifiedDescriptor>,
        abi: Abi,
        uid: u32,
        verified_layout: bool,
    ) -> io::Result<Self> {
        let deadline = Instant::now() + IMAGE_BUDGET;
        let dir = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(root)?;
        let metadata = dir.metadata()?;
        if metadata.uid() != uid || metadata.mode() & 0o022 != 0 {
            return Err(invalid());
        }
        if descriptor.is_some() {
            for marker in [c"remove", c"disable", c".ksu_hot_install", c"zygisk"] {
                if entry_exists(&dir, marker)? {
                    return Err(invalid());
                }
            }
        }
        // Existing old-layout source generations must remain fenced during
        // migration; only a signed replacement is required to have no autoload.
        let legacy_source = descriptor.is_none() && entry_exists(&dir, c"zygisk")?;
        // This internal parser verifies passive staging bytes. The resulting
        // catalog never enters backend selection or becomes active evidence.
        let catalog = Catalog::open_pinned_before(&dir, Some(deadline))?;
        catalog.manifest.require_complete()?;
        if descriptor.is_some() {
            catalog.require_gki_versions(Some(deadline))?;
        }
        let mut paths = BTreeMap::<String, Option<FileIdentity>>::new();
        paths.insert(
            "vpnhide-core".into(),
            descriptor.map(|d| match abi {
                Abi::Arm64 => d.descriptor().runtime.daemon_arm64.clone(),
                Abi::Arm32 => d.descriptor().runtime.daemon_arm.clone(),
            }),
        );
        paths.insert(
            "payloads/manifest.json".into(),
            descriptor.map(|d| d.descriptor().runtime.payload_manifest.clone()),
        );
        paths.insert(
            "service.sh".into(),
            descriptor.map(|d| d.descriptor().runtime.service.clone()),
        );
        paths.insert(
            "sepolicy.rule".into(),
            descriptor.map(|d| d.descriptor().runtime.sepolicy.clone()),
        );
        if let Some(descriptor) = descriptor {
            paths.insert(
                "module.prop".into(),
                Some(descriptor.descriptor().runtime.module_prop.clone()),
            );
        }
        for payload in &catalog.payloads {
            let identity = FileIdentity {
                bytes: payload.metadata.bytes,
                sha256: payload.metadata.sha256.clone(),
            };
            paths.insert(
                payload.metadata.implementation.relative_path()?,
                Some(identity.clone()),
            );
            if legacy_source && let PayloadKind::Zygisk { abi } = payload.metadata.implementation {
                let name = match abi {
                    Abi::Arm64 => "arm64-v8a",
                    Abi::Arm32 => "armeabi-v7a",
                };
                paths.insert(format!("zygisk/{name}.so"), Some(identity));
            }
        }
        let mut files = BTreeMap::new();
        for (path, expected) in paths {
            let file = open_beneath(&dir, &path)?;
            let actual = read_file(&file, uid, 128 * 1024 * 1024, deadline)?;
            if expected.is_some_and(|expected| actual.identity != expected) {
                return Err(invalid());
            }
            if path == "vpnhide-core" && actual.mode & 0o111 == 0 {
                return Err(invalid());
            }
            // module.prop is checked for replacement completeness, but normal
            // manager staging mutates its live copy before swapping any code.
            if path != "module.prop" {
                files.insert(path, actual);
            }
        }
        if files["payloads/manifest.json"].identity.sha256 != catalog.manifest_sha256
            || descriptor
                .is_some_and(|d| d.descriptor().source_commit != catalog.manifest.bundle_commit)
        {
            return Err(invalid());
        }
        if verified_layout {
            require_layout(
                &dir,
                &vpnhide_artifact_model::core_layout::CoreLayout::Installed(&catalog.manifest)
                    .allowed_files()?,
                &files
                    .keys()
                    .filter(|name| name.as_str() == "vpnhide-core" || name.starts_with("payloads/"))
                    .cloned()
                    .collect(),
                uid,
                deadline,
            )?;
        }
        let current = std::fs::symlink_metadata(root)?;
        if current.file_type().is_symlink()
            || (current.dev(), current.ino()) != catalog.root_identity
            || current.uid() != uid
            || current.mode() & 0o022 != 0
        {
            return Err(invalid());
        }
        Ok(Self {
            root: catalog.root_identity,
            files,
            verified_layout,
        })
    }
}

#[cfg(test)]
mod tests;
