//! Passive bundle inventory. Verification never loads or configures a backend.
//! A verified descriptor, not its original pathname, identifies the bytes to use.

use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use sha2::{Digest, Sha256};

pub mod device;
pub use vpnhide_artifact_model::elf;
pub mod kmi;
pub mod projection;
pub mod selection;

pub use vpnhide_artifact_model::manifest::{
    Abi, KMIS, MANIFEST_VERSION, MAX_BUNDLE_BYTES, MAX_MANIFEST_BYTES, MAX_PAYLOAD_BYTES, Manifest,
    Payload, PayloadKind,
};

#[derive(Debug)]
pub struct VerifiedPayload {
    pub metadata: Payload,
    /// Retained open descriptor; a subsequent path replacement cannot redirect it.
    pub file: File,
}

#[derive(Debug)]
pub struct Catalog {
    pub root_identity: (u64, u64),
    /// Hash of the exact bounded bytes parsed, independent of later path reads.
    pub manifest_sha256: String,
    pub manifest: Manifest,
    pub payloads: Vec<VerifiedPayload>,
}

impl Catalog {
    /// `root` must be the trusted active module directory, never modules_update.
    /// Every component beneath this pinned directory is opened without symlink following.
    /// This verifies packaging integrity, not publisher authenticity or runtime eligibility.
    pub fn open(root: &Path) -> io::Result<Self> {
        let root = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(root)?;
        Self::open_pinned(&root)
    }

    pub(crate) fn open_pinned(root: &File) -> io::Result<Self> {
        Self::open_pinned_before(root, None)
    }

    pub(crate) fn open_pinned_before(
        root: &File,
        deadline: Option<std::time::Instant>,
    ) -> io::Result<Self> {
        if !root.metadata()?.is_dir() {
            return Err(invalid("catalog root is not a directory"));
        }
        let manifest_file = open_beneath(root, "payloads/manifest.json")?;
        let mut bytes = Vec::new();
        manifest_file
            .take(MAX_MANIFEST_BYTES + 1)
            .read_to_end(&mut bytes)?;
        let manifest = Manifest::parse(&bytes)?;
        let manifest_sha256 = format!("{:x}", Sha256::digest(&bytes));
        let mut payloads = Vec::with_capacity(manifest.payloads.len());
        for metadata in &manifest.payloads {
            check_deadline(deadline)?;
            let mut file = open_beneath(root, &metadata.implementation.relative_path()?)?;
            if file.metadata()?.len() != metadata.bytes {
                return Err(invalid("payload size mismatch"));
            }
            let mut digest = Sha256::new();
            let mut count = 0u64;
            let mut buffer = [0; 64 * 1024];
            let mut reader = Read::by_ref(&mut file).take(metadata.bytes + 1);
            loop {
                check_deadline(deadline)?;
                let read = reader.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                count += read as u64;
                digest.update(&buffer[..read]);
            }
            if count != metadata.bytes || format!("{:x}", digest.finalize()) != metadata.sha256 {
                return Err(invalid("payload digest mismatch"));
            }
            file.rewind()?;
            if metadata.preflight_protocol.is_some() || metadata.init_contract.is_some() {
                let mut bytes = Vec::with_capacity(metadata.bytes as usize);
                Read::by_ref(&mut file)
                    .take(MAX_PAYLOAD_BYTES + 1)
                    .read_to_end(&mut bytes)?;
                let capabilities = elf::capabilities(&bytes)?;
                if metadata
                    .preflight_protocol
                    .is_some_and(|p| capabilities.preflight_protocol != Some(p))
                    || metadata
                        .init_contract
                        .is_some_and(|p| capabilities.init_contract != Some(p))
                {
                    return Err(invalid("capability differs from final ELF"));
                }
                file.rewind()?;
            }
            payloads.push(VerifiedPayload {
                metadata: metadata.clone(),
                file,
            });
        }
        use std::os::unix::fs::MetadataExt;
        let metadata = root.metadata()?;
        Ok(Self {
            root_identity: (metadata.dev(), metadata.ino()),
            manifest_sha256,
            manifest,
            payloads,
        })
    }
}

fn check_deadline(deadline: Option<std::time::Instant>) -> io::Result<()> {
    if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "catalog inspection deadline",
        ));
    }
    Ok(())
}

pub(crate) fn open_beneath(root: &File, path: &str) -> io::Result<File> {
    let mut dir = root.try_clone()?;
    let parts: Vec<_> = path.split('/').collect();
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() || *part == "." || *part == ".." {
            return Err(invalid("invalid relative path"));
        }
        let name = CString::new(*part).map_err(invalid)?;
        let directory = index + 1 != parts.len();
        let flags = libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | if directory { libc::O_DIRECTORY } else { 0 };
        // SAFETY: valid owned directory descriptor and NUL-terminated component.
        let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: openat returned a new owned descriptor.
        dir = unsafe { File::from_raw_fd(fd) };
        if !directory && !dir.metadata()?.is_file() {
            return Err(invalid("not a regular file"));
        }
    }
    Ok(dir)
}

fn invalid(error: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(test)]
mod tests;
