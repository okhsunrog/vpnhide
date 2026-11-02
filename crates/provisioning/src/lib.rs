//! Separate provisioning safety model. Not linked or packaged into the Core ZIP.
//! Device adapters must supply real runtime/partition observations; the offline
//! CLI intentionally exposes no boot-write command.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
pub mod kernel;
pub mod write;

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProvisionError {
    KernelImageInvalid,
    KernelConfigUnknown,
    KallsymsMissing,
    AllModeUnknown,
    EngineEvidenceMissing,
    ToolTupleInvalid,
    RuntimeUnknown,
    RuntimeConflict,
    BootFactsUnknown,
    SlotChanged,
    OtaInProgress,
    AvbUnconfirmed,
    BackupUnconfirmed,
    PlanChanged,
    ConsentMissing,
    ConsentExpired,
    SourceChanged,
    WriteUnknown,
    ReadbackMismatch,
    JournalUnavailable,
    HeldUnknown,
}
impl std::fmt::Display for ProvisionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ProvisionError {}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeFamily {
    APatch,
    FPatch,
    SukiSu,
    ReSukiSu,
    KernelSu,
    Magisk,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeEvidence {
    pub boot_id: String,
    pub family: RuntimeFamily,
    /// Actual manager/runtime command or syscall handshake identity, not its APK/directory name.
    pub manager_handshake_sha256: String,
    pub kernelpatch: KernelPatchObservation,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KernelPatchObservation {
    Unknown,
    /// A completed read-only native probe, bound to the boot, observed absence.
    Absent {
        probe_sha256: String,
    },
    Present {
        handshake_sha256: String,
        version: u32,
        kpm_protocol: u32,
    },
}
impl RuntimeEvidence {
    pub fn existing_kpm(&self) -> bool {
        !self.boot_id.is_empty()
            && digest(&self.manager_handshake_sha256)
            && matches!(&self.kernelpatch, KernelPatchObservation::Present {
                handshake_sha256, version: 0x0a05..=0x00ff_ffff, kpm_protocol: 1,
            } if digest(handshake_sha256))
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolIdentity {
    pub sha256: String,
    pub bytes: u64,
    pub source_commit: String,
}
impl ToolIdentity {
    fn valid(&self) -> bool {
        digest(&self.sha256)
            && self.bytes > 0
            && self.bytes <= 64 * 1024 * 1024
            && self.source_commit.len() == 40
            && self.source_commit.bytes().all(|b| b.is_ascii_hexdigit())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolTuple {
    pub engine: ToolIdentity,
    pub kptools: ToolIdentity,
    pub kpatch: ToolIdentity,
    pub magiskboot: ToolIdentity,
    /// Qualification tied to this exact engine hash. Source-declared support is
    /// useful research but cannot manufacture a tested boot compatibility gate.
    pub all_disabled_qualification: Option<AllDisabledQualification>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllDisabledQualification {
    pub engine_sha256: String,
    pub evidence_sha256: String,
}
impl ToolTuple {
    pub fn valid(&self) -> bool {
        [&self.engine, &self.kptools, &self.kpatch, &self.magiskboot]
            .into_iter()
            .all(|t| t.valid())
            && self.engine.source_commit == self.kptools.source_commit
            && self.engine.source_commit == self.kpatch.source_commit
    }
    pub fn verify_bytes(&self, tools: [&[u8]; 4]) -> Result<(), ProvisionError> {
        if !self.valid()
            || [&self.engine, &self.kptools, &self.kpatch, &self.magiskboot]
                .into_iter()
                .zip(tools)
                .any(|(identity, bytes)| {
                    identity.bytes != bytes.len() as u64 || identity.sha256 != sha256(bytes)
                })
        {
            return Err(ProvisionError::ToolTupleInvalid);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProvisionDecision {
    ExistingRuntime,
    OfflinePatchEligible,
    Blocked(ProvisionError),
}
pub fn assess(
    runtime: Option<&RuntimeEvidence>,
    kernel: &kernel::KernelEvidence,
    tools: &ToolTuple,
) -> ProvisionDecision {
    if let Some(runtime) = runtime
        && runtime.existing_kpm()
    {
        return ProvisionDecision::ExistingRuntime;
    }
    // Absence of KPM is never sufficient authority to install or flash anything.
    let Some(runtime) =
        runtime.filter(|r| !r.boot_id.is_empty() && digest(&r.manager_handshake_sha256))
    else {
        return ProvisionDecision::Blocked(ProvisionError::RuntimeUnknown);
    };
    match &runtime.kernelpatch {
        KernelPatchObservation::Unknown => {
            return ProvisionDecision::Blocked(ProvisionError::RuntimeUnknown);
        }
        KernelPatchObservation::Present { .. } => {
            return ProvisionDecision::Blocked(ProvisionError::RuntimeConflict);
        }
        KernelPatchObservation::Absent { probe_sha256 } if !digest(probe_sha256) => {
            return ProvisionDecision::Blocked(ProvisionError::RuntimeUnknown);
        }
        KernelPatchObservation::Absent { .. } => {}
    }
    if kernel.enabled("CONFIG_KPM") == Some(true) {
        return ProvisionDecision::Blocked(ProvisionError::RuntimeConflict);
    }
    if !tools.valid() {
        return ProvisionDecision::Blocked(ProvisionError::ToolTupleInvalid);
    }
    if kernel.enabled("CONFIG_KALLSYMS") != Some(true) {
        return ProvisionDecision::Blocked(ProvisionError::KallsymsMissing);
    }
    match kernel.enabled("CONFIG_KALLSYMS_ALL") {
        None => ProvisionDecision::Blocked(ProvisionError::AllModeUnknown),
        Some(false)
            if !tools.all_disabled_qualification.as_ref().is_some_and(|q| {
                q.engine_sha256 == tools.engine.sha256 && digest(&q.evidence_sha256)
            }) =>
        {
            ProvisionDecision::Blocked(ProvisionError::EngineEvidenceMissing)
        }
        _ => ProvisionDecision::OfflinePatchEligible,
    }
}
