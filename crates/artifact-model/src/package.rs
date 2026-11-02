//! Signed package data model shared by publisher tooling and authenticated readers.
//! Parsing a descriptor does not authenticate it.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
pub const DOMAIN: &[u8] = b"VPNHIDE-CORE-REPLACEMENT-v1\n";
pub const MAX_PACKAGE_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_ENVELOPE_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustError {
    Unconfigured,
    Malformed,
    Signature,
    Compatibility,
    Rollback,
    WrongTarget,
    PackageSize,
    PackageHash,
    Read,
}
impl std::fmt::Display for TrustError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for TrustError {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub descriptor: String,
    pub signature: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Descriptor {
    pub schema: u32,
    pub module_id: String,
    pub source_commit: String,
    /// Monotonic sequence in the pinned publisher's CI workflow, not versionCode.
    pub build_sequence: u64,
    pub package_sha256: String,
    pub package_bytes: u64,
    pub rpc: u32,
    pub journal_min: u32,
    pub journal_max: u32,
    pub kernel_control: u32,
    pub runtime: RuntimeIdentity,
}
/// Post-customize identities. Root managers may replace the live module.prop
/// during staging; its version string cannot establish the installed generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeIdentity {
    pub daemon_arm64: FileIdentity,
    pub daemon_arm: FileIdentity,
    pub payload_manifest: FileIdentity,
    pub service: FileIdentity,
    pub sepolicy: FileIdentity,
    pub module_prop: FileIdentity,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileIdentity {
    pub bytes: u64,
    pub sha256: String,
}
impl FileIdentity {
    pub fn valid(&self, limit: u64) -> bool {
        self.bytes > 0 && self.bytes <= limit && hex_digest(&self.sha256, 64)
    }
    pub fn verify(&self, mut input: impl Read) -> Result<(), TrustError> {
        if !self.valid(MAX_PACKAGE_BYTES) {
            return Err(TrustError::Malformed);
        }
        let mut hash = Sha256::new();
        let mut total = 0_u64;
        let mut block = [0_u8; 64 * 1024];
        loop {
            let n = input.read(&mut block).map_err(|_| TrustError::Read)?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > self.bytes {
                return Err(TrustError::PackageSize);
            }
            hash.update(&block[..n]);
        }
        if total != self.bytes {
            return Err(TrustError::PackageSize);
        }
        if format!("{:x}", hash.finalize()) != self.sha256 {
            return Err(TrustError::PackageHash);
        }
        Ok(())
    }
}
impl Descriptor {
    pub fn validate(&self) -> Result<(), TrustError> {
        if self.schema != 2
            || self.module_id != "vpnhide_core"
            || !hex_digest(&self.source_commit, 40)
            || !hex_digest(&self.package_sha256, 64)
            || self.build_sequence == 0
            || self.package_bytes == 0
            || self.package_bytes > MAX_PACKAGE_BYTES
            || self.rpc == 0
            || self.journal_min == 0
            || self.journal_min > self.journal_max
            || self.kernel_control != 2
            || !self.runtime.daemon_arm64.valid(128 * 1024 * 1024)
            || !self.runtime.daemon_arm.valid(128 * 1024 * 1024)
            || !self.runtime.payload_manifest.valid(64 * 1024)
            || !self.runtime.service.valid(64 * 1024)
            || !self.runtime.sepolicy.valid(64 * 1024)
            || !self.runtime.module_prop.valid(64 * 1024)
        {
            return Err(TrustError::Malformed);
        }
        Ok(())
    }
}
fn hex_digest(text: &str, len: usize) -> bool {
    text.len() == len
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
