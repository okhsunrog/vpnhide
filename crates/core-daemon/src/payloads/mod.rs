//! Passive bundle inventory. Verification never loads or configures a backend.
//! A verified descriptor, not its original pathname, identifies the bytes to use.

use std::collections::BTreeSet;
use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub mod device;
pub mod elf;
pub mod kmi;
pub mod projection;
pub mod selection;

pub const MANIFEST_VERSION: u32 = 1;
pub const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
pub const MAX_PAYLOAD_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_BUNDLE_BYTES: u64 = 256 * 1024 * 1024;
pub const KMIS: [&str; 7] = [
    "android12-5.10",
    "android13-5.10",
    "android13-5.15",
    "android14-5.15",
    "android14-6.1",
    "android15-6.6",
    "android16-6.12",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Abi {
    Arm64,
    Arm32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PayloadKind {
    Kmod { kmi: String },
    Kpm,
    Zygisk { abi: Abi },
}

impl PayloadKind {
    pub fn relative_path(&self) -> io::Result<String> {
        match self {
            Self::Kmod { kmi } if KMIS.contains(&kmi.as_str()) => {
                Ok(format!("payloads/kmod/{kmi}/vpnhide_kmod.ko"))
            }
            Self::Kmod { .. } => Err(invalid("unsupported bundle KMI")),
            Self::Kpm => Ok("payloads/kpm/arm64/vpnhide.kpm".into()),
            Self::Zygisk { abi } => Ok(format!(
                "payloads/zygisk/{}.so",
                match abi {
                    Abi::Arm64 => "arm64-v8a",
                    Abi::Arm32 => "armeabi-v7a",
                }
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    pub implementation: PayloadKind,
    pub version: String,
    /// The immutable source identity of this payload (unchanged payload reuse is valid).
    pub source_commit: String,
    pub sha256: String,
    pub bytes: u64,
    pub control_version: u32,
    /// Hash-bound producer capability, rechecked from final ELF before use.
    /// Old catalogs have no capability and cannot authorize a loader experiment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preflight_protocol: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub bundle_commit: String,
    pub payloads: Vec<Payload>,
}

impl Manifest {
    pub fn parse(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() as u64 > MAX_MANIFEST_BYTES {
            return Err(invalid("manifest exceeds budget"));
        }
        let manifest: Self = serde_json::from_slice(bytes).map_err(invalid)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> io::Result<()> {
        if self.schema != MANIFEST_VERSION || !hex(&self.bundle_commit, 40) {
            return Err(invalid("invalid bundle schema or source identity"));
        }
        if self.payloads.is_empty() || self.payloads.len() > KMIS.len() + 3 {
            return Err(invalid("invalid payload count"));
        }
        let mut paths = BTreeSet::new();
        let mut total = 0u64;
        for payload in &self.payloads {
            if !paths.insert(payload.implementation.relative_path()?) {
                return Err(invalid("duplicate payload identity"));
            }
            if payload.version.is_empty()
                || payload.version.len() > 128
                || payload.version.chars().any(char::is_control)
                || !hex(&payload.source_commit, 40)
                || !hex(&payload.sha256, 64)
                || payload.control_version != 2
                || payload.bytes == 0
                || payload.bytes > MAX_PAYLOAD_BYTES
                || payload.preflight_protocol.is_some_and(|p| {
                    p != 1 || !matches!(payload.implementation, PayloadKind::Kmod { .. })
                })
            {
                return Err(invalid("invalid payload metadata or control contract"));
            }
            total = total
                .checked_add(payload.bytes)
                .ok_or_else(|| invalid("size overflow"))?;
        }
        if total > MAX_BUNDLE_BYTES {
            return Err(invalid("bundle exceeds budget"));
        }
        Ok(())
    }

    /// Assembly requires the complete supported distribution, even on a device
    /// that cannot use every payload. Partial catalogs are useful for fixtures.
    pub fn require_complete(&self) -> io::Result<()> {
        self.validate()?;
        if self.payloads.len() != KMIS.len() + 3 {
            return Err(invalid("incomplete unified bundle"));
        }
        Ok(()) // uniqueness + the closed identity set establishes completeness
    }
}

#[derive(Debug)]
pub struct VerifiedPayload {
    pub metadata: Payload,
    /// Retained open descriptor; a subsequent path replacement cannot redirect it.
    pub file: File,
}

#[derive(Debug)]
pub struct Catalog {
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
        let manifest_file = open_beneath(&root, "payloads/manifest.json")?;
        let mut bytes = Vec::new();
        manifest_file
            .take(MAX_MANIFEST_BYTES + 1)
            .read_to_end(&mut bytes)?;
        let manifest = Manifest::parse(&bytes)?;
        let mut payloads = Vec::with_capacity(manifest.payloads.len());
        for metadata in &manifest.payloads {
            let mut file = open_beneath(&root, &metadata.implementation.relative_path()?)?;
            if file.metadata()?.len() != metadata.bytes {
                return Err(invalid("payload size mismatch"));
            }
            let mut digest = Sha256::new();
            let count = io::copy(
                &mut Read::by_ref(&mut file).take(metadata.bytes + 1),
                &mut digest,
            )?;
            if count != metadata.bytes || format!("{:x}", digest.finalize()) != metadata.sha256 {
                return Err(invalid("payload digest mismatch"));
            }
            file.rewind()?;
            if let Some(protocol) = metadata.preflight_protocol {
                let mut bytes = Vec::with_capacity(metadata.bytes as usize);
                Read::by_ref(&mut file)
                    .take(MAX_PAYLOAD_BYTES + 1)
                    .read_to_end(&mut bytes)?;
                if elf::preflight_protocol(&bytes)? != Some(protocol) {
                    return Err(invalid("preflight capability differs from final ELF"));
                }
                file.rewind()?;
            }
            payloads.push(VerifiedPayload {
                metadata: metadata.clone(),
                file,
            });
        }
        Ok(Self { manifest, payloads })
    }
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

fn hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn invalid(error: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(test)]
mod tests;
