//! Publisher-authenticated replacement identity. This library neither extracts
//! archives nor executes their contents. A hash supplied by the caller is never
//! a trust anchor. An unconfigured build fails closed for package installation.
use ring::signature;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;

include!(concat!(env!("OUT_DIR"), "/trust.rs"));
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "algorithm", rename_all = "snake_case", deny_unknown_fields)]
pub enum PublicKey {
    RsaSha256 { modulus: String, exponent: String },
    P256Sha256 { point: String },
    P384Sha384 { point: String },
}
impl PublicKey {
    fn verify(&self, message: &[u8], sig: &[u8]) -> Result<(), TrustError> {
        let result = match self {
            Self::RsaSha256 { modulus, exponent } => {
                let n = unhex(modulus, 1024)?;
                let e = unhex(exponent, 8)?;
                signature::RsaPublicKeyComponents { n, e }.verify(
                    &signature::RSA_PKCS1_2048_8192_SHA256,
                    message,
                    sig,
                )
            }
            Self::P256Sha256 { point } => signature::UnparsedPublicKey::new(
                &signature::ECDSA_P256_SHA256_ASN1,
                unhex(point, 65)?,
            )
            .verify(message, sig),
            Self::P384Sha384 { point } => signature::UnparsedPublicKey::new(
                &signature::ECDSA_P384_SHA384_ASN1,
                unhex(point, 97)?,
            )
            .verify(message, sig),
        };
        result.map_err(|_| TrustError::Signature)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustStore {
    pub schema: u32,
    pub keys: Vec<PublicKey>,
}
impl TrustStore {
    pub fn compiled() -> Result<Self, TrustError> {
        Self::parse(COMPILED_TRUST_JSON.as_bytes())
    }
    /// Key pin and anti-rollback baseline form one installation authority. A
    /// partial local build must not turn a missing sequence into permission.
    pub fn compiled_install_authority() -> Result<(Self, std::num::NonZeroU64), TrustError> {
        let store = Self::compiled()?;
        let sequence =
            std::num::NonZeroU64::new(COMPILED_BUILD_SEQUENCE).ok_or(TrustError::Unconfigured)?;
        Ok((store, sequence))
    }
    pub fn parse(bytes: &[u8]) -> Result<Self, TrustError> {
        if bytes.len() > MAX_ENVELOPE_BYTES {
            return Err(TrustError::Malformed);
        }
        let store: Self = serde_json::from_slice(bytes).map_err(|_| TrustError::Malformed)?;
        if store.schema != 1 || store.keys.len() > 4 {
            return Err(TrustError::Malformed);
        }
        if store.keys.is_empty() {
            return Err(TrustError::Unconfigured);
        }
        Ok(store)
    }
    pub fn authenticate(&self, bytes: &[u8]) -> Result<VerifiedDescriptor, TrustError> {
        if self.schema != 1 || self.keys.is_empty() || self.keys.len() > 4 {
            return Err(TrustError::Unconfigured);
        }
        if bytes.len() > MAX_ENVELOPE_BYTES {
            return Err(TrustError::Malformed);
        }
        let envelope: Envelope =
            serde_json::from_slice(bytes).map_err(|_| TrustError::Malformed)?;
        if envelope.descriptor.len() > 4096 {
            return Err(TrustError::Malformed);
        }
        let signature = unhex(&envelope.signature, 1024)?;
        let mut message = DOMAIN.to_vec();
        message.extend_from_slice(envelope.descriptor.as_bytes());
        // The envelope cannot select a key or algorithm. Only pinned identities
        // participate, and changing exact descriptor bytes invalidates signature.
        if !self
            .keys
            .iter()
            .any(|key| key.verify(&message, &signature).is_ok())
        {
            return Err(TrustError::Signature);
        }
        let descriptor: Descriptor =
            serde_json::from_str(&envelope.descriptor).map_err(|_| TrustError::Malformed)?;
        descriptor.validate()?;
        Ok(VerifiedDescriptor(descriptor))
    }
}

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
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileIdentity {
    pub bytes: u64,
    pub sha256: String,
}
impl FileIdentity {
    fn valid(&self, limit: u64) -> bool {
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
    fn validate(&self) -> Result<(), TrustError> {
        if self.schema != 1
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
        {
            return Err(TrustError::Malformed);
        }
        Ok(())
    }
}
/// Constructible only after signature verification; deserialize cannot create it.
#[derive(Debug, Clone)]
pub struct VerifiedDescriptor(Descriptor);
impl VerifiedDescriptor {
    pub fn descriptor(&self) -> &Descriptor {
        &self.0
    }
    pub fn require_build_at_least(&self, minimum: u64) -> Result<(), TrustError> {
        if self.0.build_sequence < minimum {
            Err(TrustError::Rollback)
        } else {
            Ok(())
        }
    }
    pub fn admit(&self, target: &Target, floor: &AcceptedFloor) -> Result<(), TrustError> {
        let d = &self.0;
        if d.source_commit != target.source_commit || d.package_sha256 != target.package_sha256 {
            return Err(TrustError::WrongTarget);
        }
        if d.rpc != target.rpc || !(d.journal_min..=d.journal_max).contains(&target.current_journal)
        {
            return Err(TrustError::Compatibility);
        }
        // Equal sequence is idempotent only for the exact previously accepted
        // package. A rerun cannot silently replace bytes at the same generation.
        if d.build_sequence < floor.build_sequence
            || (d.build_sequence == floor.build_sequence
                && floor.package_sha256.as_deref() != Some(&d.package_sha256))
        {
            return Err(TrustError::Rollback);
        }
        Ok(())
    }
    pub fn verify_package(&self, input: impl Read) -> Result<(), TrustError> {
        FileIdentity {
            bytes: self.0.package_bytes,
            sha256: self.0.package_sha256.clone(),
        }
        .verify(input)
    }
}
#[derive(Debug, Clone)]
pub struct Target {
    pub source_commit: String,
    pub package_sha256: String,
    pub rpc: u32,
    pub current_journal: u32,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedFloor {
    pub build_sequence: u64,
    pub package_sha256: Option<String>,
}
fn hex_digest(text: &str, len: usize) -> bool {
    text.len() == len
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn unhex(text: &str, max_bytes: usize) -> Result<Vec<u8>, TrustError> {
    if text.is_empty()
        || text.len() > max_bytes * 2
        || !text.len().is_multiple_of(2)
        || !hex_digest(text, text.len())
    {
        return Err(TrustError::Malformed);
    }
    text.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| {
            let n = |v: u8| if v <= b'9' { v - b'0' } else { v - b'a' + 10 };
            Ok((n(c[0]) << 4) | n(c[1]))
        })
        .collect()
}

#[cfg(test)]
mod tests;
