//! Passive manifest format. Device readers still verify pinned file bytes.
use crate::invalid;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, io};

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

impl Abi {
    pub const fn zygisk_library_name(self) -> &'static str {
        match self {
            Self::Arm64 => "arm64-v8a.so",
            Self::Arm32 => "armeabi-v7a.so",
        }
    }
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
            Self::Zygisk { abi } => Ok(format!("payloads/zygisk/{}", abi.zygisk_library_name())),
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub init_contract: Option<u32>,
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
                || payload.control_version != vpnhide_protocol::CONTROL_VERSION
                || payload.bytes == 0
                || payload.bytes > MAX_PAYLOAD_BYTES
                || payload.init_contract.is_some_and(|p| {
                    p != 1 || !matches!(payload.implementation, PayloadKind::Kmod { .. })
                })
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

fn hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

#[cfg(test)]
mod tests;
