//! Local typed projection; no additional RPC, diagnostic admission or root work.
use vpnhide_core_rpc::attachment_policy as policy;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreAttachmentReason {
    CoreUnavailable,
    Incomplete,
    KernelHooks,
    KernelCompatibility,
    JavaHooks,
    Configuration,
}
impl From<policy::AttachmentReason> for CoreAttachmentReason {
    fn from(value: policy::AttachmentReason) -> Self {
        use policy::AttachmentReason as R;
        match value {
            R::CoreUnavailable => Self::CoreUnavailable,
            R::Incomplete => Self::Incomplete,
            R::KernelHooks => Self::KernelHooks,
            R::KernelCompatibility => Self::KernelCompatibility,
            R::JavaHooks => Self::JavaHooks,
            R::Configuration => Self::Configuration,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreAttachmentRecommendation {
    pub kernel: bool,
    pub java: bool,
    pub kernel_reason: CoreAttachmentReason,
    pub java_reason: CoreAttachmentReason,
}
impl From<policy::AttachmentRecommendation> for CoreAttachmentRecommendation {
    fn from(value: policy::AttachmentRecommendation) -> Self {
        Self {
            kernel: value.kernel,
            java: value.java,
            kernel_reason: value.kernel_reason.into(),
            java_reason: value.java_reason.into(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreAttachmentEvidence {
    pub configuration_failure: bool,
    pub kernel_compatibility: bool,
    pub kernel_leak: bool,
    pub java_leak: bool,
}
impl From<policy::AttachmentEvidence> for CoreAttachmentEvidence {
    fn from(value: policy::AttachmentEvidence) -> Self {
        Self {
            configuration_failure: value.configuration_failure,
            kernel_compatibility: value.kernel_compatibility,
            kernel_leak: value.kernel_leak,
            java_leak: value.java_leak,
        }
    }
}
impl From<CoreAttachmentEvidence> for policy::AttachmentEvidence {
    fn from(value: CoreAttachmentEvidence) -> Self {
        Self {
            configuration_failure: value.configuration_failure,
            kernel_compatibility: value.kernel_compatibility,
            kernel_leak: value.kernel_leak,
            java_leak: value.java_leak,
        }
    }
}

#[uniffi::export]
pub fn recommend_diagnostic_attachments(
    evidence: Option<CoreAttachmentEvidence>,
    core_current: bool,
    diagnostics_current: bool,
) -> CoreAttachmentRecommendation {
    evidence
        .map(policy::AttachmentEvidence::from)
        .unwrap_or_default()
        .recommend(core_current, diagnostics_current)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_advice_uses_typed_rpc_facts_and_currentness_without_another_request() {
        use vpnhide_core_rpc as rpc;
        use vpnhide_diagnostic_model::{AssessmentPhase, DiagnosticAssessment, RequiredCheck};
        let reply: crate::StateReply = vpnhide_core_client::ClientReply {
            generation: 4,
            value: rpc::Published {
                epoch: 7,
                revision: 8,
                snapshot: rpc::StateSnapshot {
                    backend: rpc::NativeBackendStatus {
                        selected: Some(rpc::NativeBackend::Kpm),
                        ..Default::default()
                    },
                    autonomous_assessment: Some(DiagnosticAssessment {
                        phase: AssessmentPhase::Incomplete,
                        required: vec![
                            RequiredCheck::ProcInterfaces,
                            RequiredCheck::FrameworkCallbacks,
                        ],
                        proven: vec![],
                        leaked: vec![
                            RequiredCheck::ProcInterfaces,
                            RequiredCheck::FrameworkCallbacks,
                        ],
                        unverified: vec![],
                        native_controls: vec![],
                        framework_controls: vec![],
                        reuse_blockers: vec![],
                        infrastructure_detail: None,
                        cleanup_confirmed: Some(true),
                    }),
                    ..Default::default()
                },
            },
        }
        .into();
        assert_eq!((reply.connection_generation, reply.state.epoch), (4, 7));
        let evidence = reply.state.attachment_evidence;
        let advice = recommend_diagnostic_attachments(Some(evidence.clone()), true, true);
        assert!(advice.kernel && advice.java);
        assert_eq!(advice.kernel_reason, CoreAttachmentReason::KernelHooks);
        assert_eq!(advice.java_reason, CoreAttachmentReason::JavaHooks);
        let unjoined = recommend_diagnostic_attachments(Some(evidence.clone()), true, false);
        assert!(!unjoined.kernel && !unjoined.java);
        let offline = recommend_diagnostic_attachments(Some(evidence), false, true);
        assert!(!offline.kernel && !offline.java);
        assert_eq!(offline.kernel_reason, CoreAttachmentReason::CoreUnavailable);
    }
}
