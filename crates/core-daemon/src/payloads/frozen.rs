//! Immutable final bytes shared by load preparation and loader experiments.
//! No pathname lookup or writable duplicate can redirect a retained snapshot.
use std::fs::File;
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::FileExt;

use sha2::{Digest, Sha256};

#[derive(Debug)]
pub struct FrozenPayload {
    file: File,
    pub sha256: String,
    pub bytes: u64,
}

impl FrozenPayload {
    pub fn copy(input: &File) -> io::Result<Self> {
        Self::copy_with_limit(input, super::MAX_PAYLOAD_BYTES)
    }

    pub(crate) fn copy_with_limit(input: &File, limit: u64) -> io::Result<Self> {
        if limit == 0 || limit > super::MAX_BUNDLE_BYTES {
            return Err(io::Error::other("invalid immutable artifact budget"));
        }
        let metadata = input.metadata()?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > limit {
            return Err(io::Error::other("invalid frozen payload size or type"));
        }
        // Use the kernel ABI directly: the bionic wrapper is newer than our
        // minimum Android API. Unsupported kernels refuse before dispatch.
        let raw = unsafe {
            libc::syscall(
                libc::SYS_memfd_create,
                c"vpnhide-payload".as_ptr(),
                libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
            )
        };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: successful memfd_create returned a fresh owned descriptor.
        let mut file = unsafe { File::from_raw_fd(raw as _) };
        let mut hash = Sha256::new();
        let mut bytes = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = input.read_at(&mut buffer, bytes)?;
            if read == 0 {
                break;
            }
            bytes += read as u64;
            if bytes > limit || bytes > metadata.len() {
                return Err(io::Error::other("payload grew while freezing"));
            }
            file.write_all(&buffer[..read])?;
            hash.update(&buffer[..read]);
        }
        if bytes != metadata.len() || input.metadata()?.len() != bytes {
            return Err(io::Error::other("payload size changed while freezing"));
        }
        let required =
            libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL;
        if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_ADD_SEALS, required) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let seals = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GET_SEALS) };
        if seals < 0 {
            return Err(io::Error::last_os_error());
        }
        if seals & required != required {
            return Err(io::Error::other("payload immutability not confirmed"));
        }
        Ok(Self {
            file,
            sha256: format!("{:x}", hash.finalize()),
            bytes,
        })
    }

    pub fn file(&self) -> &File {
        &self.file
    }

    pub fn into_file(self) -> File {
        self.file
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Seek, SeekFrom};

    #[test]
    fn snapshot_survives_path_replacement_and_rejects_every_write_alias() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("payload");
        std::fs::write(&path, b"final bytes").unwrap();
        let mut input = File::open(&path).unwrap();
        input.seek(SeekFrom::Start(5)).unwrap();
        let frozen = FrozenPayload::copy(&input).unwrap();
        assert_eq!(input.stream_position().unwrap(), 5);
        std::fs::write(path, b"replacement").unwrap();
        assert_eq!(frozen.bytes, 11);
        assert_eq!(
            frozen.sha256,
            format!("{:x}", Sha256::digest(b"final bytes"))
        );
        let file = frozen.file();
        let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFD) };
        assert!(flags >= 0);
        assert_ne!(flags & libc::FD_CLOEXEC, 0);
        let duplicate = file.try_clone().unwrap();
        assert_eq!(
            duplicate.write_at(b"x", 0).unwrap_err().raw_os_error(),
            Some(libc::EPERM)
        );
        assert_eq!(
            duplicate.set_len(1).unwrap_err().raw_os_error(),
            Some(libc::EPERM)
        );
        assert_eq!(
            duplicate.set_len(12).unwrap_err().raw_os_error(),
            Some(libc::EPERM)
        );
        let mut alias = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(format!("/proc/self/fd/{}", file.as_raw_fd()))
            .unwrap();
        assert_eq!(
            alias.write(b"x").unwrap_err().raw_os_error(),
            Some(libc::EPERM)
        );
        let mut bytes = Vec::new();
        alias.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"final bytes");
    }

    #[test]
    fn invalid_inputs_refuse_without_creating_a_named_file() {
        let root = tempfile::tempdir().unwrap();
        assert!(FrozenPayload::copy(&File::open(root.path()).unwrap()).is_err());
        let empty = tempfile::tempfile().unwrap();
        assert!(FrozenPayload::copy(&empty).is_err());
        empty.set_len(super::super::MAX_PAYLOAD_BYTES + 1).unwrap();
        assert!(FrozenPayload::copy(&empty).is_err());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }
}
