//! Bounded selection evidence, distinct from load, policy delivery and health.
use crate::NativeBackend;
use serde::{Deserialize, Serialize};

pub const MAX_NATIVE_MODULE_EVIDENCE: usize = 32;
pub const MAX_SELECTION_EVIDENCE_BYTES: usize = 128 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeModuleEvidence {
    pub source: String,
    pub sha256: String,
    pub name: String,
    pub vermagic: String,
    pub srcversion: Option<String>,
    pub build_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NativeSelectionDecision {
    Candidate { backend: NativeBackend },
    ReconcileResident { backend: NativeBackend },
    BlockedOwnership,
    NoEligibleCandidate,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSelectionEvidence {
    pub boot_id: String,
    pub observed_at_boot_ms: Option<u64>,
    pub kernel_release: String,
    pub kernel_machine: String,
    pub core_abi: String,
    pub manifest_sha256: String,
    pub manifest_file_sha256: String,
    pub source_commit: String,
    pub candidate_kmis: Vec<String>,
    pub uname_hint: Option<String>,
    pub module_hint: Option<String>,
    pub conflicting_hints: bool,
    pub modules: Vec<NativeModuleEvidence>,
    pub inventory_complete: bool,
    pub unbound_files: u32,
    pub decision: NativeSelectionDecision,
}
fn text(value: &str, max: usize) -> bool {
    value.len() <= max && !value.bytes().any(|b| b.is_ascii_control())
}
fn hex(value: &str, bytes: usize) -> bool {
    value.len() == bytes * 2
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl NativeSelectionEvidence {
    pub fn valid(&self) -> bool {
        !self.boot_id.is_empty()
            && text(&self.boot_id, 128)
            && text(&self.kernel_release, 256)
            && text(&self.kernel_machine, 256)
            && matches!(self.core_abi.as_str(), "arm32" | "arm64")
            && hex(&self.manifest_sha256, 32)
            && hex(&self.manifest_file_sha256, 32)
            && hex(&self.source_commit, 20)
            && self.candidate_kmis.len() <= 32
            && self.candidate_kmis.iter().all(|v| text(v, 64))
            && [&self.uname_hint, &self.module_hint]
                .into_iter()
                .all(|v| v.as_ref().is_none_or(|v| text(v, 64)))
            && self.modules.len() <= MAX_NATIVE_MODULE_EVIDENCE
            && self.modules.iter().all(|m| {
                text(&m.source, 640)
                    && hex(&m.sha256, 32)
                    && !m.name.is_empty()
                    && text(&m.name, 63)
                    && text(&m.vermagic, 1024)
                    && m.srcversion.as_ref().is_none_or(|v| {
                        !v.is_empty() && v.len() <= 64 && v.bytes().all(|b| b.is_ascii_hexdigit())
                    })
                    && (4..=64).contains(&(m.build_id.len() / 2))
                    && m.build_id.len().is_multiple_of(2)
                    && m.build_id
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maximum_escaped_evidence_stays_bounded_without_claiming_health() {
        let evidence = NativeSelectionEvidence {
            boot_id: "b".repeat(128),
            observed_at_boot_ms: None,
            kernel_release: "k".repeat(256),
            kernel_machine: "m".repeat(256),
            core_abi: "arm64".into(),
            manifest_sha256: "a".repeat(64),
            manifest_file_sha256: "c".repeat(64),
            source_commit: "b".repeat(40),
            candidate_kmis: vec!["k".repeat(64); 32],
            uname_hint: None,
            module_hint: None,
            conflicting_hints: true,
            inventory_complete: false,
            unbound_files: u32::MAX,
            decision: NativeSelectionDecision::NoEligibleCandidate,
            modules: vec![
                NativeModuleEvidence {
                    source: "\\".repeat(640),
                    sha256: "a".repeat(64),
                    name: "n".repeat(63),
                    vermagic: "\\".repeat(1024),
                    srcversion: Some("A".repeat(64)),
                    build_id: "b".repeat(128)
                };
                MAX_NATIVE_MODULE_EVIDENCE
            ],
        };
        assert!(evidence.valid());
        let bytes = serde_json::to_vec(&evidence).unwrap();
        assert!(bytes.len() <= MAX_SELECTION_EVIDENCE_BYTES);
        assert_eq!(
            serde_json::from_slice::<NativeSelectionEvidence>(&bytes).unwrap(),
            evidence
        );
    }
}
