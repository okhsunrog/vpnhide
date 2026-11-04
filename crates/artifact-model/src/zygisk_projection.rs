//! Inert Core-owned discovery copy. This format grants no hook authority and
//! is never an independently distributed/updated backend package.
use crate::{
    invalid,
    manifest::{Abi, MANIFEST_VERSION, Manifest, Payload, PayloadKind},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io};
use vpnhide_protocol::zygisk_activation::{MANAGED_MODULE_ID, PROJECTION_PROTOCOL};

pub const FILE_NAME: &str = "vpnhide-projection.json";
pub const MAX_DESCRIPTOR_BYTES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Descriptor {
    pub schema: u32,
    pub operation: u64,
    pub source_commit: String,
    pub manifest_sha256: String,
    pub manifest_file_sha256: String,
    pub payloads: Vec<Payload>,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn hash(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl Descriptor {
    pub fn bind(manifest_bytes: &[u8], operation: u64) -> io::Result<Self> {
        let manifest = Manifest::parse(manifest_bytes)?;
        manifest.require_complete()?;
        let payloads = [Abi::Arm64, Abi::Arm32]
            .into_iter()
            .map(|abi| {
                manifest
                    .payloads
                    .iter()
                    .find(|p| p.implementation == PayloadKind::Zygisk { abi })
                    .cloned()
                    .ok_or_else(|| invalid("projection is missing an ABI"))
            })
            .collect::<io::Result<Vec<_>>>()?;
        let result = Self {
            schema: PROJECTION_PROTOCOL,
            operation,
            source_commit: manifest.bundle_commit.clone(),
            manifest_sha256: digest(&serde_json::to_vec(&manifest).map_err(invalid)?),
            manifest_file_sha256: digest(manifest_bytes),
            payloads,
        };
        result.validate()?;
        Ok(result)
    }
    pub fn parse(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() > MAX_DESCRIPTOR_BYTES {
            return Err(invalid("projection descriptor budget"));
        }
        let value: Self = serde_json::from_slice(bytes).map_err(invalid)?;
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> io::Result<()> {
        if self.schema != PROJECTION_PROTOCOL
            || self.operation == 0
            || !hash(&self.source_commit, 40)
            || !hash(&self.manifest_sha256, 64)
            || !hash(&self.manifest_file_sha256, 64)
            || self.payloads.len() != 2
            || self.payloads[0].implementation != (PayloadKind::Zygisk { abi: Abi::Arm64 })
            || self.payloads[1].implementation != (PayloadKind::Zygisk { abi: Abi::Arm32 })
        {
            return Err(invalid("invalid projection identity or ABI pair"));
        }
        Manifest {
            schema: MANIFEST_VERSION,
            bundle_commit: self.source_commit.clone(),
            payloads: self.payloads.clone(),
        }
        .validate()
    }
    pub fn require_source(&self, manifest_bytes: &[u8]) -> io::Result<()> {
        if *self != Self::bind(manifest_bytes, self.operation)? {
            return Err(invalid(
                "projection differs from authoritative source generation",
            ));
        }
        Ok(())
    }
    pub fn encode(&self) -> io::Result<Vec<u8>> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(invalid)?;
        if bytes.len() > MAX_DESCRIPTOR_BYTES {
            return Err(invalid("projection descriptor budget"));
        }
        Ok(bytes)
    }
    pub fn member_name(abi: Abi) -> String {
        format!("zygisk/{}", abi.zygisk_library_name())
    }
    pub fn module_prop() -> Vec<u8> {
        format!("id={MANAGED_MODULE_ID}\nname=VPN Hide native runtime (managed)\nversion=managed\nversionCode=1\nauthor=VPN Hide Core\ndescription=Managed by VPN Hide Core. No independent updater or activator.\nvpnhideControl={}\nvpnhideActivation=1\nvpnhideProjection={PROJECTION_PROTOCOL}\n", vpnhide_protocol::CONTROL_VERSION).into_bytes()
    }
    /// Copies are checked against authoritative final metadata. No partial ABI
    /// pair, activation file, service, installer script or update channel exists.
    pub fn files(&self, arm64: &[u8], arm32: &[u8]) -> io::Result<BTreeMap<String, Vec<u8>>> {
        self.files_owned(arm64.to_vec(), arm32.to_vec())
    }
    pub fn files_owned(
        &self,
        arm64: Vec<u8>,
        arm32: Vec<u8>,
    ) -> io::Result<BTreeMap<String, Vec<u8>>> {
        self.validate()?;
        let mut files = BTreeMap::from([
            ("module.prop".into(), Self::module_prop()),
            (FILE_NAME.into(), self.encode()?),
        ]);
        for (abi, bytes) in [(Abi::Arm64, arm64), (Abi::Arm32, arm32)] {
            let payload = self
                .payloads
                .iter()
                .find(|p| p.implementation == PayloadKind::Zygisk { abi })
                .ok_or_else(|| invalid("missing projection ABI"))?;
            if bytes.len() as u64 != payload.bytes || digest(&bytes) != payload.sha256 {
                return Err(invalid("projection library differs from pinned payload"));
            }
            crate::elf::shared_object_header(&bytes, bytes.len() as u64, abi)?;
            files.insert(Self::member_name(abi), bytes);
        }
        Ok(files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> BTreeMap<String, Vec<u8>> {
        crate::fixtures::passive_bundle_files().unwrap()
    }
    fn files(
        source: &BTreeMap<String, Vec<u8>>,
        descriptor: &Descriptor,
    ) -> io::Result<BTreeMap<String, Vec<u8>>> {
        descriptor.files(
            &source["payloads/zygisk/arm64-v8a.so"],
            &source["payloads/zygisk/armeabi-v7a.so"],
        )
    }
    #[test]
    fn projection_contains_one_passive_pair_and_no_authority_or_boot_entry() {
        let source = source();
        let descriptor = Descriptor::bind(&source["payloads/manifest.json"], 8).unwrap();
        let members = files(&source, &descriptor).unwrap();
        assert_eq!(
            members.keys().map(String::as_str).collect::<Vec<_>>(),
            vec![
                "module.prop",
                FILE_NAME,
                "zygisk/arm64-v8a.so",
                "zygisk/armeabi-v7a.so"
            ]
        );
        assert_eq!(Descriptor::parse(&members[FILE_NAME]).unwrap(), descriptor);
        assert_eq!(
            vpnhide_protocol::zygisk_activation::recipient_from_manifest(&members["module.prop"]),
            Some(vpnhide_protocol::zygisk_activation::Recipient::Managed)
        );
    }
    #[test]
    fn exact_source_and_both_final_libraries_are_required() {
        let mut source = source();
        let descriptor = Descriptor::bind(&source["payloads/manifest.json"], 1).unwrap();
        let manifest = Manifest::parse(&source["payloads/manifest.json"]).unwrap();
        assert!(
            descriptor
                .require_source(&serde_json::to_vec_pretty(&manifest).unwrap())
                .is_err()
        );
        source
            .get_mut("payloads/zygisk/armeabi-v7a.so")
            .unwrap()
            .push(0);
        assert!(files(&source, &descriptor).is_err());
        assert!(Descriptor::bind(&source["payloads/manifest.json"], 0).is_err());
        let mut wrong = descriptor.clone();
        wrong.payloads.reverse();
        assert!(wrong.encode().is_err());
        let mut partial = manifest;
        partial
            .payloads
            .retain(|p| !matches!(p.implementation, PayloadKind::Zygisk { abi: Abi::Arm32 }));
        assert!(Descriptor::bind(&serde_json::to_vec(&partial).unwrap(), 1).is_err());
    }
}
