use super::*;
use crate::payloads::selection::{Assessment, Capabilities, Decision};
use vpnhide_core_rpc::{NativeModuleEvidence, NativeSelectionDecision, NativeSelectionEvidence};

pub(super) fn capture(
    env: &NativeEnv,
    catalog: &crate::payloads::Catalog,
    capabilities: &Capabilities,
    assessment: &Assessment,
) -> Result<NativeSelectionEvidence, String> {
    let evidence = NativeSelectionEvidence {
        boot_id: read_boot_id(&env.boot_id_path),
        observed_at_boot_ms: crate::boot_time::BootInstant::now()
            .and_then(|t| t.millis())
            .ok(),
        kernel_release: capabilities.kernel_release.clone(),
        kernel_machine: capabilities.kernel_machine.clone(),
        core_abi: match capabilities.abi {
            crate::payloads::Abi::Arm32 => "arm32",
            crate::payloads::Abi::Arm64 => "arm64",
        }
        .into(),
        manifest_sha256: bundled::manifest_digest(&catalog.manifest),
        manifest_file_sha256: catalog.manifest_sha256.clone(),
        source_commit: catalog.manifest.bundle_commit.clone(),
        candidate_kmis: assessment
            .kmi
            .candidates
            .iter()
            .map(|c| c.kmi.clone())
            .collect(),
        uname_hint: assessment.kmi.release_hint.clone(),
        module_hint: assessment.kmi.module_hint.clone(),
        conflicting_hints: assessment.kmi.conflicting_hints,
        modules: capabilities
            .module_evidence
            .modules
            .iter()
            .map(|module| NativeModuleEvidence {
                source: module.source.to_string_lossy().into_owned(),
                sha256: module.sha256.clone(),
                name: module.metadata.name.clone(),
                vermagic: module.metadata.vermagic.clone(),
                srcversion: module.metadata.srcversion.clone(),
                build_id: module
                    .metadata
                    .build_id
                    .as_ref()
                    .map(|bytes| bytes.iter().map(|b| format!("{b:02x}")).collect())
                    .unwrap_or_default(),
            })
            .collect(),
        inventory_complete: !capabilities.module_evidence.incomplete,
        unbound_files: u32::try_from(capabilities.module_evidence.unbound_files)
            .map_err(|e| e.to_string())?,
        decision: match assessment.decision {
            Decision::Select(backend) => NativeSelectionDecision::Candidate { backend },
            Decision::ReconcileResident(backend) => {
                NativeSelectionDecision::ReconcileResident { backend }
            }
            Decision::BlockedOwnership => NativeSelectionDecision::BlockedOwnership,
            Decision::NoEligibleCandidate => NativeSelectionDecision::NoEligibleCandidate,
        },
    };
    if !evidence.valid()
        || serde_json::to_vec(&evidence)
            .map_err(|e| e.to_string())?
            .len()
            > vpnhide_core_rpc::MAX_SELECTION_EVIDENCE_BYTES
    {
        return Err("selection evidence exceeded its validated contract".into());
    }
    Ok(evidence)
}
