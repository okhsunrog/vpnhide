use crate::CoreNativeBackend;

/// Forensic serialization uses the same Rust wire owner; Kotlin adds no second
/// evidence codec or selection classifier to its legacy export DTO.
#[uniffi::export]
pub fn native_selection_evidence_json(value: CoreNativeSelectionEvidence) -> String {
    let evidence: rpc::NativeSelectionEvidence = value.into();
    serde_json::to_string(&evidence).expect("selection evidence serialization")
}
use vpnhide_core_rpc as rpc;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreNativeModuleEvidence {
    pub source: String,
    pub sha256: String,
    pub name: String,
    pub vermagic: String,
    pub srcversion: Option<String>,
    pub build_id: String,
}
impl From<rpc::NativeModuleEvidence> for CoreNativeModuleEvidence {
    fn from(value: rpc::NativeModuleEvidence) -> Self {
        Self {
            source: value.source,
            sha256: value.sha256,
            name: value.name,
            vermagic: value.vermagic,
            srcversion: value.srcversion,
            build_id: value.build_id,
        }
    }
}
impl From<CoreNativeModuleEvidence> for rpc::NativeModuleEvidence {
    fn from(value: CoreNativeModuleEvidence) -> Self {
        Self {
            source: value.source,
            sha256: value.sha256,
            name: value.name,
            vermagic: value.vermagic,
            srcversion: value.srcversion,
            build_id: value.build_id,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreNativeSelectionDecision {
    Candidate { backend: CoreNativeBackend },
    ReconcileResident { backend: CoreNativeBackend },
    BlockedOwnership,
    NoEligibleCandidate,
}
impl From<rpc::NativeSelectionDecision> for CoreNativeSelectionDecision {
    fn from(value: rpc::NativeSelectionDecision) -> Self {
        match value {
            rpc::NativeSelectionDecision::Candidate { backend } => Self::Candidate {
                backend: backend.into(),
            },
            rpc::NativeSelectionDecision::ReconcileResident { backend } => {
                Self::ReconcileResident {
                    backend: backend.into(),
                }
            }
            rpc::NativeSelectionDecision::BlockedOwnership => Self::BlockedOwnership,
            rpc::NativeSelectionDecision::NoEligibleCandidate => Self::NoEligibleCandidate,
        }
    }
}
impl From<CoreNativeSelectionDecision> for rpc::NativeSelectionDecision {
    fn from(value: CoreNativeSelectionDecision) -> Self {
        match value {
            CoreNativeSelectionDecision::Candidate { backend } => Self::Candidate {
                backend: backend.into(),
            },
            CoreNativeSelectionDecision::ReconcileResident { backend } => Self::ReconcileResident {
                backend: backend.into(),
            },
            CoreNativeSelectionDecision::BlockedOwnership => Self::BlockedOwnership,
            CoreNativeSelectionDecision::NoEligibleCandidate => Self::NoEligibleCandidate,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreNativeSelectionEvidence {
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
    pub modules: Vec<CoreNativeModuleEvidence>,
    pub inventory_complete: bool,
    pub unbound_files: u32,
    pub decision: CoreNativeSelectionDecision,
}
impl From<rpc::NativeSelectionEvidence> for CoreNativeSelectionEvidence {
    fn from(value: rpc::NativeSelectionEvidence) -> Self {
        Self {
            boot_id: value.boot_id,
            observed_at_boot_ms: value.observed_at_boot_ms,
            kernel_release: value.kernel_release,
            kernel_machine: value.kernel_machine,
            core_abi: value.core_abi,
            manifest_sha256: value.manifest_sha256,
            manifest_file_sha256: value.manifest_file_sha256,
            source_commit: value.source_commit,
            candidate_kmis: value.candidate_kmis,
            uname_hint: value.uname_hint,
            module_hint: value.module_hint,
            conflicting_hints: value.conflicting_hints,
            modules: value.modules.into_iter().map(Into::into).collect(),
            inventory_complete: value.inventory_complete,
            unbound_files: value.unbound_files,
            decision: value.decision.into(),
        }
    }
}
impl From<CoreNativeSelectionEvidence> for rpc::NativeSelectionEvidence {
    fn from(value: CoreNativeSelectionEvidence) -> Self {
        Self {
            boot_id: value.boot_id,
            observed_at_boot_ms: value.observed_at_boot_ms,
            kernel_release: value.kernel_release,
            kernel_machine: value.kernel_machine,
            core_abi: value.core_abi,
            manifest_sha256: value.manifest_sha256,
            manifest_file_sha256: value.manifest_file_sha256,
            source_commit: value.source_commit,
            candidate_kmis: value.candidate_kmis,
            uname_hint: value.uname_hint,
            module_hint: value.module_hint,
            conflicting_hints: value.conflicting_hints,
            modules: value.modules.into_iter().map(Into::into).collect(),
            inventory_complete: value.inventory_complete,
            unbound_files: value.unbound_files,
            decision: value.decision.into(),
        }
    }
}
