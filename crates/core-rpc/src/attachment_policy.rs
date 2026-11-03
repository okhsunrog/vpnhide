//! Attachment advice from accepted Core facts. This does not run diagnostics,
//! select a backend, authorize collection or declare protection successful.
use crate::{NativeBackend, NativeBootOutcome, StateSnapshot};
use vpnhide_diagnostic_model::RequiredCheck;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AttachmentReason {
    CoreUnavailable,
    #[default]
    Incomplete,
    KernelHooks,
    KernelCompatibility,
    JavaHooks,
    Configuration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AttachmentRecommendation {
    pub kernel: bool,
    pub java: bool,
    pub kernel_reason: AttachmentReason,
    pub java_reason: AttachmentReason,
}

/// A bounded local read projection, not an additional RPC/journal owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AttachmentEvidence {
    pub configuration_failure: bool,
    pub kernel_compatibility: bool,
    pub kernel_leak: bool,
    pub java_leak: bool,
}

impl From<&StateSnapshot> for AttachmentEvidence {
    fn from(state: &StateSnapshot) -> Self {
        Self::from_facts(
            state.backend.boot.as_ref().map(|boot| &boot.outcome),
            state.backend.selected,
            state
                .autonomous_assessment
                .as_ref()
                .map_or(&[], |assessment| assessment.leaked.as_slice()),
        )
    }
}

impl AttachmentEvidence {
    pub fn from_facts(
        boot: Option<&NativeBootOutcome>,
        selected: Option<NativeBackend>,
        leaked: &[RequiredCheck],
    ) -> Self {
        let kernel_compatibility = match boot {
            Some(NativeBootOutcome::UnsupportedKernel { backend, .. }) => {
                *backend != NativeBackend::Zygisk
            }
            Some(NativeBootOutcome::LoadFailed {
                backend,
                signature_rejected: false,
                kprobes,
                kretprobes,
                stderr,
                ..
            }) if *backend != NativeBackend::Zygisk => {
                let detail = stderr.to_ascii_lowercase();
                *kprobes == Some(false)
                    || *kretprobes == Some(false)
                    || detail.contains("unknown symbol")
                    || detail.contains("invalid module format")
            }
            _ => false,
        };
        let configuration_failure = matches!(
            boot,
            Some(
                NativeBootOutcome::Conflict { .. }
                    | NativeBootOutcome::AwaitingAuthentication { .. }
                    | NativeBootOutcome::MissingArtifact { .. }
                    | NativeBootOutcome::CapacityExceeded { .. }
            )
        );
        let mut native_leak = false;
        let mut java_leak = false;
        for check in leaked {
            use RequiredCheck::*;
            // Exhaustive typed classification: new vectors cannot silently become
            // native merely because their serialized name lacks a Java prefix.
            match check {
                FrameworkNetworks
                | FrameworkDefaultNetwork
                | FrameworkCapabilities
                | FrameworkLinkProperties
                | FrameworkLegacyInfo
                | FrameworkCallbacks
                | FrameworkPendingIntent
                | PackageVisibility => java_leak = true,
                Ports => {}
                IoctlFlags | IoctlMtu | Getifaddrs | NetlinkLinks | IoctlAddresses
                | BindToDevice | NetlinkAddresses | NetlinkRoutes | NetlinkRules | ProcRoutes
                | ProcInterfaces | SysInterfaces | Filesystem => native_leak = true,
            }
        }
        Self {
            configuration_failure,
            kernel_compatibility,
            kernel_leak: native_leak
                && matches!(
                    selected,
                    Some(NativeBackend::Kmod | NativeBackend::Builtin | NativeBackend::Kpm)
                ),
            java_leak,
        }
    }

    /// The adapter supplies currentness of its accepted observation/config join.
    /// Stale diagnostics cannot recommend binary collection; current boot-load
    /// compatibility evidence remains useful without a fresh diagnostic run.
    pub fn recommend(
        self,
        core_current: bool,
        diagnostics_current: bool,
    ) -> AttachmentRecommendation {
        use AttachmentReason::*;
        if !core_current {
            return AttachmentRecommendation {
                kernel_reason: CoreUnavailable,
                java_reason: CoreUnavailable,
                ..Default::default()
            };
        }
        let kernel = diagnostics_current && self.kernel_leak;
        let java = diagnostics_current && self.java_leak;
        AttachmentRecommendation {
            kernel: !self.configuration_failure && (kernel || self.kernel_compatibility),
            java,
            kernel_reason: if self.configuration_failure {
                Configuration
            } else if self.kernel_compatibility {
                KernelCompatibility
            } else if kernel {
                KernelHooks
            } else {
                Incomplete
            },
            java_reason: if java { JavaHooks } else { Incomplete },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_leaks_choose_only_relevant_binary_attachments() {
        use RequiredCheck::*;
        for backend in [
            NativeBackend::Kmod,
            NativeBackend::Builtin,
            NativeBackend::Kpm,
        ] {
            let evidence = AttachmentEvidence::from_facts(
                None,
                Some(backend),
                &[
                    ProcInterfaces,
                    IoctlAddresses,
                    FrameworkCallbacks,
                    PackageVisibility,
                ],
            );
            let advice = evidence.recommend(true, true);
            assert!(advice.kernel && advice.java);
            assert_eq!(advice.kernel_reason, AttachmentReason::KernelHooks);
            assert_eq!(advice.java_reason, AttachmentReason::JavaHooks);
            assert_eq!(
                evidence.recommend(true, false),
                AttachmentRecommendation::default()
            );
            let stale = evidence.recommend(false, true);
            assert!(!stale.kernel && !stale.java);
            assert_eq!(stale.kernel_reason, AttachmentReason::CoreUnavailable);
        }
        for backend in [None, Some(NativeBackend::Zygisk)] {
            let evidence = AttachmentEvidence::from_facts(None, backend, &[ProcInterfaces, Ports]);
            assert_eq!(
                evidence.recommend(true, true),
                AttachmentRecommendation::default()
            );
        }
    }

    fn failed(backend: NativeBackend, signature_rejected: bool, stderr: &str) -> NativeBootOutcome {
        NativeBootOutcome::LoadFailed {
            backend,
            exit: Some(1),
            stderr: stderr.into(),
            dmesg_tail: vec![],
            kprobes: None,
            kretprobes: None,
            signature_rejected,
        }
    }

    #[test]
    fn compatibility_is_boot_evidence_not_a_diagnostic_success_or_generic_load_error() {
        for backend in [NativeBackend::Builtin, NativeBackend::Zygisk] {
            let boot = NativeBootOutcome::UnsupportedKernel {
                backend,
                uname: "reference".into(),
                detail: "unsupported".into(),
            };
            let advice =
                AttachmentEvidence::from_facts(Some(&boot), None, &[]).recommend(true, false);
            assert_eq!(advice.kernel, backend == NativeBackend::Builtin);
        }
        for text in ["UNKNOWN SYMBOL x", "Invalid Module Format"] {
            let boot = failed(NativeBackend::Kmod, false, text);
            let advice =
                AttachmentEvidence::from_facts(Some(&boot), None, &[]).recommend(true, false);
            assert!(advice.kernel && !advice.java);
            assert_eq!(advice.kernel_reason, AttachmentReason::KernelCompatibility);
            for refused in [
                failed(NativeBackend::Kmod, true, text),
                failed(NativeBackend::Zygisk, false, text),
            ] {
                assert!(
                    !AttachmentEvidence::from_facts(Some(&refused), None, &[])
                        .recommend(true, true)
                        .kernel
                );
            }
        }
        let mut boot = failed(NativeBackend::Kmod, false, "permission denied");
        assert!(
            !AttachmentEvidence::from_facts(Some(&boot), None, &[])
                .recommend(true, true)
                .kernel
        );
        if let NativeBootOutcome::LoadFailed { kretprobes, .. } = &mut boot {
            *kretprobes = Some(false);
        }
        assert!(
            AttachmentEvidence::from_facts(Some(&boot), None, &[])
                .recommend(true, false)
                .kernel
        );
    }

    #[test]
    fn configuration_error_suppresses_kernel_dump_but_preserves_independent_java_evidence() {
        let boot = NativeBootOutcome::AwaitingAuthentication {
            detail: "key required".into(),
        };
        let advice = AttachmentEvidence::from_facts(
            Some(&boot),
            Some(NativeBackend::Kpm),
            &[
                RequiredCheck::ProcInterfaces,
                RequiredCheck::FrameworkCapabilities,
            ],
        )
        .recommend(true, true);
        assert!(!advice.kernel && advice.java);
        assert_eq!(advice.kernel_reason, AttachmentReason::Configuration);
    }
}
