//! One shared read projection of retained Core evidence. Scoped proof, leaks,
//! incomplete execution and cache eligibility remain distinct facts.
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{
    ControlFailure, DIAGNOSTIC_CONTRACT, DiagnosticOutcome, DiagnosticPlan, DiagnosticRun,
    NativeCheck, RequiredCheck, ScopedNativeResult,
    framework::{FrameworkCheck, FrameworkControlFailure},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssessmentPhase {
    Running,
    Incomplete,
    Infrastructure,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReuseBlocker {
    ConditionsChanged,
    LoadedGenerationsIncomplete,
    PlatformUnknown,
    LoadedGenerationsUnknown,
    InvalidFingerprint,
    UnsupportedPlan,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticAssessment {
    pub phase: AssessmentPhase,
    pub required: Vec<RequiredCheck>,
    pub proven: Vec<RequiredCheck>,
    pub leaked: Vec<RequiredCheck>,
    pub unverified: Vec<RequiredCheck>,
    pub native_controls: Vec<ControlFailure>,
    pub framework_controls: Vec<FrameworkControlFailure>,
    pub reuse_blockers: Vec<ReuseBlocker>,
    pub infrastructure_detail: Option<String>,
    pub cleanup_confirmed: Option<bool>,
}
fn native(check: NativeCheck) -> RequiredCheck {
    match check {
        NativeCheck::IoctlFlags => RequiredCheck::IoctlFlags,
        NativeCheck::IoctlMtu => RequiredCheck::IoctlMtu,
        NativeCheck::Getifaddrs => RequiredCheck::Getifaddrs,
        NativeCheck::NetlinkGetlink => RequiredCheck::NetlinkLinks,
    }
}
fn address(check: crate::AddressCheck) -> RequiredCheck {
    match check.vector {
        crate::AddressVector::GetifaddrsIpv4 | crate::AddressVector::GetifaddrsIpv6 => {
            RequiredCheck::Getifaddrs
        }
        crate::AddressVector::NetlinkIpv4 | crate::AddressVector::NetlinkIpv6 => {
            RequiredCheck::NetlinkAddresses
        }
        _ => RequiredCheck::IoctlAddresses,
    }
}
pub(crate) fn framework_checks(
    checks: &[FrameworkCheck],
    require_both_legacy: bool,
) -> BTreeSet<RequiredCheck> {
    let mut result = BTreeSet::new();
    for check in checks {
        match check {
            FrameworkCheck::Networks => {
                result.insert(RequiredCheck::FrameworkNetworks);
            }
            FrameworkCheck::Capabilities => {
                result.insert(RequiredCheck::FrameworkCapabilities);
            }
            FrameworkCheck::LinkProperties => {
                result.insert(RequiredCheck::FrameworkLinkProperties);
            }
            FrameworkCheck::LegacyTypeVpn | FrameworkCheck::LegacyEnumeration => {
                if !require_both_legacy
                    || [
                        FrameworkCheck::LegacyTypeVpn,
                        FrameworkCheck::LegacyEnumeration,
                    ]
                    .iter()
                    .all(|c| checks.contains(c))
                {
                    result.insert(RequiredCheck::FrameworkLegacyInfo);
                }
            }
        }
    }
    result
}
impl DiagnosticRun {
    pub fn assessment(&self) -> DiagnosticAssessment {
        use RequiredCheck::*;
        let required = DiagnosticPlan::supported(
            self.plan.required.contains(&Filesystem),
            self.plan.required.contains(&PackageVisibility),
            self.plan.required.contains(&Ports),
        )
        .required;
        let mut reuse_blockers = Vec::new();
        if self.stale {
            reuse_blockers.push(ReuseBlocker::ConditionsChanged);
        }
        if !self.fingerprint.loaded_generations_complete {
            reuse_blockers.push(ReuseBlocker::LoadedGenerationsIncomplete);
        }
        if self.fingerprint.platform_sha256.is_none() {
            reuse_blockers.push(ReuseBlocker::PlatformUnknown);
        }
        if self.fingerprint.loaded_generations_sha256.is_none() {
            reuse_blockers.push(ReuseBlocker::LoadedGenerationsUnknown);
        }
        // Fill only missing applicability reasons; this never turns incomplete
        // execution into success even when a fingerprint becomes reusable.
        if !self.fingerprint.reusable() && reuse_blockers.is_empty() {
            reuse_blockers.push(ReuseBlocker::InvalidFingerprint);
        }
        if self.plan.contract != DIAGNOSTIC_CONTRACT || self.plan.required != required {
            reuse_blockers.push(ReuseBlocker::UnsupportedPlan);
        }
        let (phase, result, infrastructure_detail, cleanup_confirmed) = match &self.outcome {
            DiagnosticOutcome::Running => (AssessmentPhase::Running, None, None, None),
            DiagnosticOutcome::Incomplete { native, .. } => {
                (AssessmentPhase::Incomplete, Some(native), None, None)
            }
            DiagnosticOutcome::Infrastructure {
                detail,
                native,
                cleanup_confirmed,
            } => (
                AssessmentPhase::Infrastructure,
                native.as_ref(),
                Some(detail.clone()),
                *cleanup_confirmed,
            ),
        };
        let mut proven = BTreeSet::new();
        let mut leaked = BTreeSet::new();
        let mut native_controls = Vec::new();
        let interface_getifaddrs = matches!(result, Some(ScopedNativeResult::Proven { checks }) if checks.contains(&NativeCheck::Getifaddrs));
        match result {
            Some(ScopedNativeResult::Proven { checks }) => proven.extend(
                checks
                    .iter()
                    .copied()
                    .filter(|c| *c != NativeCheck::Getifaddrs)
                    .map(native),
            ),
            Some(ScopedNativeResult::Leaked { checks, incomplete }) => {
                leaked.extend(checks.iter().copied().map(native));
                native_controls.extend(incomplete);
            }
            Some(ScopedNativeResult::Unverified {
                reason,
                observed_leaks,
            }) => {
                leaked.extend(observed_leaks.iter().copied().map(native));
                native_controls.push(*reason);
            }
            None => {}
        }
        if let Some(addresses) = &self.addresses {
            let required_checks = crate::AddressCheck::required();
            for group in [IoctlAddresses, NetlinkAddresses, Getifaddrs] {
                if (group != Getifaddrs || interface_getifaddrs)
                    && required_checks
                        .iter()
                        .filter(|c| address(**c) == group)
                        .all(|c| addresses.proven.contains(c))
                {
                    proven.insert(group);
                }
            }
            leaked.extend(addresses.leaked.iter().copied().map(address));
            for control in &addresses.controls {
                if !native_controls.contains(control) {
                    native_controls.push(*control);
                }
            }
        }
        if let Some(proc_interfaces) = &self.proc_interfaces {
            if proc_interfaces.complete() {
                proven.insert(ProcInterfaces);
            }
            if !proc_interfaces.leaked.is_empty() {
                leaked.insert(ProcInterfaces);
            }
            for control in &proc_interfaces.controls {
                if !native_controls.contains(control) {
                    native_controls.push(*control);
                }
            }
        }
        if let Some(binding) = &self.binding {
            if binding.complete() {
                proven.insert(RequiredCheck::BindToDevice);
            }
            if !binding.leaked.is_empty() {
                leaked.insert(RequiredCheck::BindToDevice);
            }
            for control in &binding.controls {
                if !native_controls.contains(control) {
                    native_controls.push(*control);
                }
            }
        }
        let framework_controls = if let Some(result) = &self.framework {
            proven.extend(framework_checks(&result.proven, true));
            leaked.extend(framework_checks(&result.leaked, false));
            result.controls.clone()
        } else {
            Vec::new()
        };
        proven.retain(|check| !leaked.contains(check));
        let unverified = required
            .iter()
            .filter(|check| !proven.contains(check) && !leaked.contains(check))
            .copied()
            .collect();
        DiagnosticAssessment {
            phase,
            required,
            proven: proven.into_iter().collect(),
            leaked: leaked.into_iter().collect(),
            unverified,
            native_controls,
            framework_controls,
            reuse_blockers,
            infrastructure_detail,
            cleanup_confirmed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DiagnosticFingerprint, framework::ScopedFrameworkResult};
    fn run(native: ScopedNativeResult) -> DiagnosticRun {
        DiagnosticRun {
            proc_interfaces: None,
            addresses: None,
            binding: None,
            session: 1,
            fingerprint: DiagnosticFingerprint {
                boot_id: "boot".into(),
                core_epoch: 1,
                policy_sha256: "a".repeat(64),
                runtime_sha256: "b".repeat(64),
                platform_sha256: None,
                loaded_generations_sha256: None,
                loaded_generations_complete: false,
            },
            plan: DiagnosticPlan::supported(false, false, false),
            stale: false,
            framework: None,
            outcome: DiagnosticOutcome::Incomplete {
                native,
                missing: vec![],
            },
        }
    }
    #[test]
    fn previous_or_truncated_optional_plan_cannot_be_reused_as_current_proof() {
        let mut evidence = run(ScopedNativeResult::Proven { checks: vec![] });
        evidence.plan.contract = 5;
        evidence.plan.required.push(RequiredCheck::SysInterfaces);
        assert!(
            evidence
                .assessment()
                .reuse_blockers
                .contains(&ReuseBlocker::UnsupportedPlan)
        );
        evidence.plan = DiagnosticPlan::supported(true, false, false);
        evidence
            .plan
            .required
            .retain(|c| *c != RequiredCheck::SysInterfaces);
        let assessment = evidence.assessment();
        assert!(
            assessment
                .reuse_blockers
                .contains(&ReuseBlocker::UnsupportedPlan)
        );
        assert!(assessment.required.contains(&RequiredCheck::SysInterfaces));
        assert!(
            assessment
                .unverified
                .contains(&RequiredCheck::SysInterfaces)
        );
    }
    #[test]
    fn nested_leaks_survive_infrastructure_cleanup_failures_and_staleness() {
        for native in [
            ScopedNativeResult::Leaked {
                checks: vec![NativeCheck::IoctlMtu],
                incomplete: vec![ControlFailure::CleanupUnconfirmed],
            },
            ScopedNativeResult::Unverified {
                reason: ControlFailure::ProbeUnavailable,
                observed_leaks: vec![NativeCheck::IoctlMtu],
            },
        ] {
            let mut run = run(native.clone());
            run.stale = true;
            run.outcome = DiagnosticOutcome::Infrastructure {
                detail: "worker died".into(),
                native: Some(native),
                cleanup_confirmed: Some(false),
            };
            run.framework = Some(ScopedFrameworkResult {
                proven: vec![FrameworkCheck::Networks],
                leaked: vec![FrameworkCheck::Networks],
                controls: vec![FrameworkControlFailure::CleanupUnconfirmed],
                ..Default::default()
            });
            let a = run.assessment();
            assert_eq!(a.phase, AssessmentPhase::Infrastructure);
            assert_eq!(
                a.leaked,
                [RequiredCheck::IoctlMtu, RequiredCheck::FrameworkNetworks]
            );
            assert!(a.proven.is_empty());
            assert!(a.reuse_blockers.contains(&ReuseBlocker::ConditionsChanged));
            assert_eq!(a.cleanup_confirmed, Some(false));
        }
    }
    #[test]
    fn scoped_success_does_not_shrink_supported_plan_or_imply_full_success() {
        let mut run = run(ScopedNativeResult::Proven {
            checks: vec![
                NativeCheck::IoctlFlags,
                NativeCheck::IoctlMtu,
                NativeCheck::Getifaddrs,
                NativeCheck::NetlinkGetlink,
            ],
        });
        run.framework = Some(ScopedFrameworkResult {
            proven: vec![
                FrameworkCheck::Networks,
                FrameworkCheck::Capabilities,
                FrameworkCheck::LinkProperties,
            ],
            ..Default::default()
        });
        let a = run.assessment();
        assert_eq!(a.phase, AssessmentPhase::Incomplete);
        assert_eq!(a.proven.len(), 6);
        assert_eq!(a.unverified.len(), 12);
        assert!(!a.required.contains(&RequiredCheck::SysInterfaces));
        assert!(a.unverified.contains(&RequiredCheck::Getifaddrs));
        assert!(
            a.reuse_blockers
                .contains(&ReuseBlocker::LoadedGenerationsIncomplete)
        );
        assert!(!a.reuse_blockers.contains(&ReuseBlocker::ConditionsChanged));
        run.plan.required.clear();
        let a = run.assessment();
        assert_eq!(a.required.len(), 18);
        assert!(a.reuse_blockers.contains(&ReuseBlocker::UnsupportedPlan));
    }
    #[test]
    fn independent_bind_controls_preserve_other_proofs_and_direct_actor_leaks() {
        let mut run = run(ScopedNativeResult::Proven {
            checks: vec![
                NativeCheck::IoctlFlags,
                NativeCheck::IoctlMtu,
                NativeCheck::Getifaddrs,
                NativeCheck::NetlinkGetlink,
            ],
        });
        run.record_binding(crate::ScopedBindResult {
            proven: vec![crate::BindActor::Libc],
            leaked: vec![crate::BindActor::Syscall],
            controls: vec![ControlFailure::OrdinaryObjectsChanged],
        });
        let assessment = run.assessment();
        assert_eq!(assessment.proven.len(), 3);
        assert_eq!(assessment.leaked, vec![RequiredCheck::BindToDevice]);
        assert!(
            assessment
                .native_controls
                .contains(&ControlFailure::OrdinaryObjectsChanged)
        );
        run.record_binding(crate::ScopedBindResult {
            proven: crate::BindActor::ALL.to_vec(),
            leaked: vec![],
            controls: vec![],
        });
        assert!(
            run.assessment()
                .proven
                .contains(&RequiredCheck::BindToDevice)
        );
        assert!(run.assessment().unverified.len() >= 14);
    }
    #[test]
    fn proc_group_needs_every_surface_and_read_actor_and_retains_leaks() {
        let mut run = run(ScopedNativeResult::Proven { checks: vec![] });
        run.record_proc_interfaces(crate::ScopedProcInterfaceResult {
            proven: crate::ProcInterfaceCheck::required(),
            ..Default::default()
        });
        assert!(
            run.assessment()
                .proven
                .contains(&RequiredCheck::ProcInterfaces)
        );
        let checks = run.proc_interfaces.as_mut().unwrap();
        let missing = checks.proven.pop().unwrap();
        assert!(
            run.assessment()
                .unverified
                .contains(&RequiredCheck::ProcInterfaces)
        );
        run.proc_interfaces.as_mut().unwrap().leaked.push(missing);
        assert!(
            run.assessment()
                .leaked
                .contains(&RequiredCheck::ProcInterfaces)
        );
        assert!(
            !run.assessment()
                .proven
                .contains(&RequiredCheck::ProcInterfaces)
        );
    }

    #[test]
    fn address_groups_require_all_families_and_both_real_syscall_actors() {
        let mut run = run(ScopedNativeResult::Proven {
            checks: vec![
                NativeCheck::IoctlFlags,
                NativeCheck::IoctlMtu,
                NativeCheck::Getifaddrs,
                NativeCheck::NetlinkGetlink,
            ],
        });
        assert!(
            run.assessment()
                .unverified
                .contains(&RequiredCheck::Getifaddrs)
        );
        run.record_addresses(crate::ScopedAddressResult {
            proven: crate::AddressCheck::required(),
            ..Default::default()
        });
        for group in [
            RequiredCheck::IoctlAddresses,
            RequiredCheck::NetlinkAddresses,
            RequiredCheck::Getifaddrs,
        ] {
            assert!(run.assessment().proven.contains(&group));
        }
        let mut addresses = run.addresses.take().unwrap();
        addresses
            .proven
            .retain(|c| c.vector != crate::AddressVector::GetifaddrsIpv6);
        addresses.leaked.push(crate::AddressCheck {
            actor: crate::NativeReadActor::Syscall,
            vector: crate::AddressVector::IoctlAddress,
        });
        run.record_addresses(addresses);
        let result = run.assessment();
        assert!(result.unverified.contains(&RequiredCheck::Getifaddrs));
        assert!(result.leaked.contains(&RequiredCheck::IoctlAddresses));
        assert!(!result.proven.contains(&RequiredCheck::IoctlAddresses));
        assert!(result.proven.contains(&RequiredCheck::NetlinkAddresses));
    }
}
