//! Publisher-authenticated replacement identity. This library neither extracts
//! archives nor executes their contents. A hash supplied by the caller is never
//! a trust anchor. An unconfigured build fails closed for package installation.
use ring::signature;
use serde::{Deserialize, Serialize};
use std::io::Read;

include!(concat!(env!("OUT_DIR"), "/trust.rs"));
pub use vpnhide_artifact_model::package::{
    DOMAIN, Descriptor, Envelope, FileIdentity, MAX_ENVELOPE_BYTES, MAX_PACKAGE_BYTES,
    RuntimeIdentity, TrustError,
};

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
        floor.validate()?;
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
        floor.require_successor(&AcceptedFloor {
            build_sequence: d.build_sequence,
            package_sha256: Some(d.package_sha256.clone()),
        })?;
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
impl AcceptedFloor {
    pub fn validate(&self) -> Result<(), TrustError> {
        match (self.build_sequence, self.package_sha256.as_deref()) {
            (0, None) => Ok(()),
            (sequence, Some(hash)) if sequence > 0 && hex_digest(hash, 64) => Ok(()),
            _ => Err(TrustError::Malformed),
        }
    }
    pub fn require_successor(&self, next: &Self) -> Result<(), TrustError> {
        self.validate()?;
        next.validate()?;
        if next.build_sequence < self.build_sequence
            || (next.build_sequence == self.build_sequence && next != self)
        {
            return Err(TrustError::Rollback);
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
