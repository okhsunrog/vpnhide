//! Accepted replacement floor outside the pruned operation receipts and replaced
//! module directory. Preparing inert bytes is not acceptance or installation.
use super::*;
use serde::{Deserialize, Serialize};

const STATE: &str = "maintenance-floor.json";
const NEXT: &str = "maintenance-floor.next";
const MAX_BYTES: u64 = 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u32,
    floor: AcceptedFloor,
}

pub struct FloorStore {
    directory: File,
    uid: u32,
    #[cfg(test)]
    fail_sync: std::sync::atomic::AtomicBool,
}
pub struct FloorLease<'a> {
    store: &'a FloorStore,
    _lock: File,
}
impl FloorStore {
    pub fn open(persistent: &Path) -> Result<Self, PackageError> {
        Self::open_owned(persistent, 0)
    }
    pub(super) fn open_owned(persistent: &Path, uid: u32) -> Result<Self, PackageError> {
        let directory = directory(persistent)?;
        private(&directory, uid)?;
        Ok(Self {
            directory,
            uid,
            #[cfg(test)]
            fail_sync: std::sync::atomic::AtomicBool::new(false),
        })
    }
    pub fn lock(&self) -> Result<FloorLease<'_>, PackageError> {
        private(&self.directory, self.uid)?;
        let file = match openat(
            &self.directory,
            "maintenance-floor.lock",
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL,
        ) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                let pinned = self.pinned_regular("maintenance-floor.lock")?;
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .custom_flags(libc::O_CLOEXEC)
                    .open(format!("/proc/self/fd/{}", pinned.as_raw_fd()))?
            }
            Err(error) => return Err(error.into()),
        };
        private(&file, self.uid)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Err(PackageError::UnsafeStorage);
        }
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } < 0 {
            let error = io::Error::last_os_error();
            return Err(if error.kind() == io::ErrorKind::WouldBlock {
                PackageError::Busy
            } else {
                error.into()
            });
        }
        Ok(FloorLease {
            store: self,
            _lock: file,
        })
    }
    fn sync(&self) -> io::Result<()> {
        #[cfg(test)]
        if self
            .fail_sync
            .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            return Err(io::Error::other("injected floor directory sync failure"));
        }
        self.directory.sync_all()
    }
    #[cfg(test)]
    pub(super) fn fail_next_sync(&self) {
        self.fail_sync
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    fn pinned_regular(&self, name: &str) -> Result<File, PackageError> {
        let file = openat(&self.directory, name, libc::O_PATH)?;
        private(&file, self.uid)?;
        let meta = file.metadata()?;
        if !meta.is_file() || meta.nlink() != 1 || meta.len() > MAX_BYTES {
            return Err(PackageError::UnsafeStorage);
        }
        Ok(file)
    }
}
impl FloorLease<'_> {
    pub(super) fn matches(&self, directory: &File) -> Result<bool, PackageError> {
        let actual = directory.metadata()?;
        let expected = self.store.directory.metadata()?;
        Ok((actual.dev(), actual.ino()) == (expected.dev(), expected.ino()))
    }
    pub fn read(&self) -> Result<AcceptedFloor, PackageError> {
        private(&self.store.directory, self.store.uid)?;
        let pinned = match self.store.pinned_regular(STATE) {
            Ok(file) => file,
            Err(PackageError::Storage(e)) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(AcceptedFloor::default());
            }
            Err(error) => return Err(error),
        };
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC)
            .open(format!("/proc/self/fd/{}", pinned.as_raw_fd()))?;
        let mut bytes = Vec::new();
        (&mut file).take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(PackageError::UnsafeStorage);
        }
        let record: Record = serde_json::from_slice(&bytes).map_err(|_| TrustError::Malformed)?;
        if record.schema != 1 {
            return Err(TrustError::Malformed.into());
        }
        record.floor.validate()?;
        Ok(record.floor)
    }
    /// Only a signature-checked privately copied package can advance the floor.
    /// The future executor calls this after durable user-command admission and
    /// before manager dispatch. This method itself performs no manager effect.
    pub fn accept_prepared(&self, package: &PreparedPackage) -> Result<(), PackageError> {
        let parent = self.store.directory.metadata()?;
        if package.persistent != (parent.dev(), parent.ino()) {
            return Err(PackageError::UnsafeStorage);
        }
        package.verify_private(self.store.uid)?;
        let current = self.read()?;
        let descriptor = package.descriptor();
        let next = AcceptedFloor {
            build_sequence: descriptor.build_sequence,
            package_sha256: Some(descriptor.package_sha256.clone()),
        };
        current.require_successor(&next)?;
        if next == current {
            // Reconfirm a visible rename whose previous directory sync failed.
            return self
                .store
                .sync()
                .map_err(PackageError::FloorDurabilityUnknown);
        }
        match self.store.pinned_regular(NEXT) {
            Ok(_) => {
                if unsafe {
                    libc::unlinkat(
                        self.store.directory.as_raw_fd(),
                        c"maintenance-floor.next".as_ptr(),
                        0,
                    )
                } < 0
                {
                    return Err(io::Error::last_os_error().into());
                }
            }
            Err(PackageError::Storage(e)) if e.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let bytes = serde_json::to_vec(&Record {
            schema: 1,
            floor: next,
        })
        .map_err(io::Error::other)?;
        let mut file = openat(
            &self.store.directory,
            NEXT,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        )?;
        use std::io::Write;
        file.write_all(&bytes)?;
        file.sync_all()?;
        if unsafe {
            libc::renameat(
                self.store.directory.as_raw_fd(),
                c"maintenance-floor.next".as_ptr(),
                self.store.directory.as_raw_fd(),
                c"maintenance-floor.json".as_ptr(),
            )
        } < 0
        {
            return Err(io::Error::last_os_error().into());
        }
        self.store
            .sync()
            .map_err(PackageError::FloorDurabilityUnknown)
    }
}
