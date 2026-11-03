//! Only publisher-authenticated bytes enter root-private replacement storage.
//! App-owned files are inputs, never execution paths. This module performs no
//! manager operation and never overwrites the running module generation.
use std::{
    ffi::CString,
    fs::{File, OpenOptions},
    io::{self, Read, Seek},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::Path,
};
use vpnhide_package_trust::{
    AcceptedFloor, Descriptor, Target, TrustError, TrustStore, VerifiedDescriptor,
};
mod floor;
pub use floor::{FloorLease, FloorStore};

#[derive(Debug)]
pub enum PackageError {
    Trust(TrustError),
    Storage(io::Error),
    UnsafeStorage,
    SourceChanged,
    Busy,
    StorageFull,
    FloorDurabilityUnknown(io::Error),
}
impl std::fmt::Display for PackageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for PackageError {}
impl From<io::Error> for PackageError {
    fn from(e: io::Error) -> Self {
        Self::Storage(e)
    }
}
impl From<TrustError> for PackageError {
    fn from(e: TrustError) -> Self {
        Self::Trust(e)
    }
}

pub struct PackageStaging {
    persistent: File,
    directory: File,
    app: File,
    app_uid: u32,
    root_uid: u32,
}
/// The descriptor and file are retained together after private-copy readback.
/// Callers must journal intent before handing a path to a manager adapter.
#[derive(Debug)]
pub struct PreparedPackage {
    persistent: (u64, u64),
    verified: VerifiedDescriptor,
    file: File,
}
impl PreparedPackage {
    pub fn descriptor(&self) -> &Descriptor {
        self.verified.descriptor()
    }
    pub(crate) fn verified(&self) -> &VerifiedDescriptor {
        &self.verified
    }
    pub fn verify_bundle(&self) -> Result<(), PackageError> {
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC)
            .open(format!("/proc/self/fd/{}", self.file.as_raw_fd()))?;
        let mut bytes = Vec::new();
        (&mut file)
            .take(vpnhide_package_trust::MAX_PACKAGE_BYTES + 1)
            .read_to_end(&mut bytes)?;
        vpnhide_artifact_model::archive::verify_package(&bytes, self.descriptor())?;
        Ok(())
    }
    pub fn file(&self) -> &File {
        &self.file
    }
    fn verify_private(&self, uid: u32) -> Result<(), PackageError> {
        private(&self.file, uid)?;
        let before = self.file.metadata()?;
        if !before.is_file()
            || before.nlink() != 1
            || before.len() != self.descriptor().package_bytes
        {
            return Err(PackageError::UnsafeStorage);
        }
        // Reopen the pinned inode with an independent offset. Cloning the file
        // would share its cursor with a future manager reader.
        let input = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC)
            .open(format!("/proc/self/fd/{}", self.file.as_raw_fd()))?;
        vpnhide_package_trust::FileIdentity {
            bytes: self.descriptor().package_bytes,
            sha256: self.descriptor().package_sha256.clone(),
        }
        .verify(input)?;
        let after = self.file.metadata()?;
        let identity = |m: &std::fs::Metadata| {
            (
                m.dev(),
                m.ino(),
                m.len(),
                m.uid(),
                m.mode(),
                m.nlink(),
                m.mtime(),
                m.mtime_nsec(),
                m.ctime(),
                m.ctime_nsec(),
            )
        };
        if identity(&before) != identity(&after) {
            return Err(PackageError::SourceChanged);
        }
        Ok(())
    }
}
fn directory(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}
fn private(file: &File, uid: u32) -> Result<(), PackageError> {
    let metadata = file.metadata()?;
    if metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
        return Err(PackageError::UnsafeStorage);
    }
    Ok(())
}
fn openat(directory: &File, name: &str, flags: i32) -> io::Result<File> {
    let name = CString::new(name).map_err(io::Error::other)?;
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            0o600,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
impl PackageStaging {
    /// `persistent` is the existing root-private journal parent, outside the
    /// replaced module. `app` and its expected UID come from the Core endpoint,
    /// never from an RPC-supplied path or UID.
    pub fn open(persistent: &Path, app: &Path, expected_uid: u32) -> Result<Self, PackageError> {
        Self::open_owned(persistent, app, expected_uid, 0)
    }
    pub(super) fn open_owned(
        persistent: &Path,
        app: &Path,
        expected_uid: u32,
        root_uid: u32,
    ) -> Result<Self, PackageError> {
        let parent = directory(persistent)?;
        private(&parent, root_uid)?;
        let name = c"maintenance-packages";
        if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } < 0
            && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists
        {
            return Err(io::Error::last_os_error().into());
        }
        let staged_dir = openat(
            &parent,
            "maintenance-packages",
            libc::O_RDONLY | libc::O_DIRECTORY,
        )?;
        private(&staged_dir, root_uid)?;
        parent.sync_all()?;
        let app = directory(app)?;
        if app.metadata()?.uid() != expected_uid {
            return Err(PackageError::UnsafeStorage);
        }
        Ok(Self {
            persistent: parent,
            directory: staged_dir,
            app,
            app_uid: expected_uid,
            root_uid,
        })
    }
    pub fn prepare(
        &self,
        envelope: &[u8],
        target: &Target,
        floor: &FloorLease<'_>,
        operation: u64,
    ) -> Result<PreparedPackage, PackageError> {
        let verified = self.authenticate(envelope, target, floor)?;
        self.copy_verified(&verified, operation)
    }
    fn authenticate(
        &self,
        envelope: &[u8],
        target: &Target,
        floor: &FloorLease<'_>,
    ) -> Result<VerifiedDescriptor, PackageError> {
        let (trust, sequence) = TrustStore::compiled_install_authority()?;
        let verified = trust.authenticate(envelope)?;
        if !floor.matches(&self.persistent)? {
            return Err(PackageError::UnsafeStorage);
        }
        verified.admit(target, &floor.read()?)?;
        verified.require_build_at_least(sequence.get())?;
        Ok(verified)
    }
    /// The controller has no retained replacement and owns its exclusive
    /// preparation permit. Only then may it release inert prior package files.
    pub(super) fn prepare_replacement(
        &self,
        envelope: &[u8],
        target: &Target,
        floor: &FloorLease<'_>,
        operation: u64,
        _permit: &super::replacement::PreparationPermit,
    ) -> Result<PreparedPackage, PackageError> {
        let verified = self.authenticate(envelope, target, floor)?;
        let mut keep =
            std::collections::BTreeSet::from([verified.descriptor().package_sha256.clone()]);
        if let Some(hash) = floor.read()?.package_sha256 {
            keep.insert(hash);
        }
        self.release_inert(&keep, floor)?;
        self.copy_verified(&verified, operation)
    }
    fn release_inert(
        &self,
        keep: &std::collections::BTreeSet<String>,
        floor: &FloorLease<'_>,
    ) -> Result<(), PackageError> {
        if !floor.matches(&self.persistent)? {
            return Err(PackageError::UnsafeStorage);
        }
        private(&self.directory, self.root_uid)?;
        let _lease = self.lease()?;
        self.recover_temporary()?;
        for entry in std::fs::read_dir(format!("/proc/self/fd/{}", self.directory.as_raw_fd()))? {
            let name = entry?
                .file_name()
                .into_string()
                .map_err(|_| PackageError::UnsafeStorage)?;
            let Some(hash) = name.strip_suffix(".zip") else {
                continue;
            };
            if keep.contains(hash) {
                continue;
            }
            let file = openat(&self.directory, &name, libc::O_PATH)?;
            private(&file, self.root_uid)?;
            let m = file.metadata()?;
            if !m.is_file() || m.nlink() != 1 {
                return Err(PackageError::UnsafeStorage);
            }
            let name = CString::new(name).map_err(io::Error::other)?;
            if unsafe { libc::unlinkat(self.directory.as_raw_fd(), name.as_ptr(), 0) } < 0 {
                return Err(io::Error::last_os_error().into());
            }
        }
        self.directory.sync_all()?;
        Ok(())
    }
    pub(super) fn copy_verified(
        &self,
        verified: &VerifiedDescriptor,
        operation: u64,
    ) -> Result<PreparedPackage, PackageError> {
        private(&self.directory, self.root_uid)?;
        let _lease = self.lease()?;
        if operation == 0 {
            return Err(PackageError::UnsafeStorage);
        }
        let d = verified.descriptor();
        let count = self.recover_temporary()?;
        let final_name = format!("{}.zip", d.package_sha256);
        match self.read_private(verified, &final_name) {
            Ok(package) => return Ok(package),
            Err(PackageError::Storage(e)) if e.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        if count >= 2 {
            return Err(PackageError::StorageFull);
        }
        let source_name = format!("files/core-maintenance/{}.zip", d.package_sha256);
        let mut source = crate::payloads::open_beneath(&self.app, &source_name)?;
        let before = source.metadata()?;
        if !before.is_file()
            || before.uid() != self.app_uid
            || before.nlink() != 1
            || before.len() != d.package_bytes
        {
            return Err(PackageError::SourceChanged);
        }
        let temporary_name = format!("copy-{operation}");
        let mut output = openat(
            &self.directory,
            &temporary_name,
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL,
        )?;
        let cleanup = Temporary {
            directory: &self.directory,
            name: CString::new(temporary_name).map_err(io::Error::other)?,
        };
        let count = io::copy(&mut (&mut source).take(d.package_bytes + 1), &mut output)?;
        let after = source.metadata()?;
        if count != d.package_bytes
            || before.dev() != after.dev()
            || before.ino() != after.ino()
            || before.len() != after.len()
            || before.uid() != after.uid()
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
            return Err(PackageError::SourceChanged);
        }
        output.sync_all()?;
        output.rewind()?;
        verified.verify_package(&mut output)?;
        let final_name = CString::new(final_name).map_err(io::Error::other)?;
        // Publish without replacing an existing inode. A crash after rename but
        // before journal admission leaves authenticated inert bytes, not effects.
        if unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                self.directory.as_raw_fd(),
                cleanup.name.as_ptr(),
                self.directory.as_raw_fd(),
                final_name.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        } < 0
            && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists
        {
            return Err(io::Error::last_os_error().into());
        }
        drop(output);
        drop(cleanup);
        self.directory.sync_all()?;
        self.read_private(verified, final_name.to_str().map_err(io::Error::other)?)
    }
    fn read_private(
        &self,
        verified: &VerifiedDescriptor,
        name: &str,
    ) -> Result<PreparedPackage, PackageError> {
        let d = verified.descriptor();
        let mut file = openat(&self.directory, name, libc::O_RDONLY)?;
        private(&file, self.root_uid)?;
        let meta = file.metadata()?;
        if !meta.is_file() || meta.len() != d.package_bytes || meta.nlink() != 1 {
            return Err(PackageError::UnsafeStorage);
        }
        verified.verify_package(&mut file)?;
        file.rewind()?;
        Ok(PreparedPackage {
            persistent: {
                let meta = self.persistent.metadata()?;
                (meta.dev(), meta.ino())
            },
            verified: verified.clone(),
            file,
        })
    }
    fn lease(&self) -> Result<crate::file_lock::FileLock, PackageError> {
        let file = openat(
            &self.directory,
            "staging.lock",
            libc::O_RDWR | libc::O_CREAT,
        )?;
        private(&file, self.root_uid)?;
        let meta = file.metadata()?;
        if !meta.is_file() || meta.nlink() != 1 {
            return Err(PackageError::UnsafeStorage);
        }
        crate::file_lock::FileLock::exclusive(file).map_err(|error| {
            if error.kind() == io::ErrorKind::WouldBlock {
                PackageError::Busy
            } else {
                error.into()
            }
        })
    }
    /// Only interrupted inert copies are recovered here. The durable executor
    /// must release a plan before its final package can be pruned.
    fn recover_temporary(&self) -> Result<usize, PackageError> {
        let mut packages = 0;
        let mut entries = 0;
        let mut total = 0_u64;
        for entry in
            std::fs::read_dir(format!("/proc/self/fd/{}", self.directory.as_raw_fd()))?.take(6)
        {
            entries += 1;
            if entries > 5 {
                return Err(PackageError::StorageFull);
            }
            let name = entry?
                .file_name()
                .into_string()
                .map_err(|_| PackageError::UnsafeStorage)?;
            if name == "staging.lock" {
                continue;
            }
            let temporary = name.strip_prefix("copy-").is_some_and(|v| {
                !v.is_empty() && v.len() <= 20 && v.bytes().all(|b| b.is_ascii_digit())
            });
            let archive = name.strip_suffix(".zip").is_some_and(|v| {
                v.len() == 64
                    && v.bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            });
            if !temporary && !archive {
                return Err(PackageError::UnsafeStorage);
            }
            let file = openat(&self.directory, &name, libc::O_RDONLY)?;
            private(&file, self.root_uid)?;
            let meta = file.metadata()?;
            if !meta.is_file() || meta.nlink() != 1 {
                return Err(PackageError::UnsafeStorage);
            }
            if temporary {
                let name = CString::new(name).map_err(io::Error::other)?;
                if unsafe { libc::unlinkat(self.directory.as_raw_fd(), name.as_ptr(), 0) } < 0 {
                    return Err(io::Error::last_os_error().into());
                }
            } else {
                packages += 1;
                total = total
                    .checked_add(meta.len())
                    .ok_or(PackageError::StorageFull)?;
                if meta.len() > vpnhide_package_trust::MAX_PACKAGE_BYTES
                    || total > 2 * vpnhide_package_trust::MAX_PACKAGE_BYTES
                {
                    return Err(PackageError::StorageFull);
                }
            }
        }
        self.directory.sync_all()?;
        Ok(packages)
    }
}
struct Temporary<'a> {
    directory: &'a File,
    name: CString,
}
impl Drop for Temporary<'_> {
    fn drop(&mut self) {
        unsafe {
            libc::unlinkat(self.directory.as_raw_fd(), self.name.as_ptr(), 0);
        }
    }
}

#[cfg(test)]
mod tests;
