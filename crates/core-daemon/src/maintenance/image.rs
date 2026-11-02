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
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Core image generation is incomplete or changed",
    )
}
fn read_file(file: &File, uid: u32, limit: u64, deadline: Instant) -> io::Result<ImageFile> {
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
    fn require_unchanged_owned(&self, root: &Path, uid: u32) -> io::Result<()> {
        if self != &Self::inspect(root, None, Abi::Arm64, uid)? {
            return Err(invalid());
        }
        Ok(())
    }
    /// Verify the actual executing inode, not just a path whose bytes may have
    /// been replaced underneath an older supervisor/worker.
    pub fn require_running(&self, running: &File) -> io::Result<()> {
        self.require_running_owned(running, 0)
    }
    fn require_running_owned(&self, running: &File, uid: u32) -> io::Result<()> {
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
    fn inspect(
        root: &Path,
        descriptor: Option<&VerifiedDescriptor>,
        abi: Abi,
        uid: u32,
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
            for marker in [c"remove", c"disable", c".ksu_hot_install"] {
                let mut meta = std::mem::MaybeUninit::<libc::stat>::uninit();
                use std::os::fd::AsRawFd;
                let rc = unsafe {
                    libc::fstatat(
                        dir.as_raw_fd(),
                        marker.as_ptr(),
                        meta.as_mut_ptr(),
                        libc::AT_SYMLINK_NOFOLLOW,
                    )
                };
                if rc == 0 || io::Error::last_os_error().kind() != io::ErrorKind::NotFound {
                    return Err(invalid());
                }
            }
        }
        // This internal parser verifies passive staging bytes. The resulting
        // catalog never enters backend selection or becomes active evidence.
        let catalog = Catalog::open_pinned_before(&dir, Some(deadline))?;
        catalog.manifest.require_complete()?;
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
            if let PayloadKind::Zygisk { abi } = payload.metadata.implementation {
                paths.insert(
                    format!(
                        "zygisk/{}.so",
                        match abi {
                            Abi::Arm64 => "arm64-v8a",
                            Abi::Arm32 => "armeabi-v7a",
                        }
                    ),
                    Some(identity),
                );
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
        })
    }
}

#[cfg(test)]
mod tests;
