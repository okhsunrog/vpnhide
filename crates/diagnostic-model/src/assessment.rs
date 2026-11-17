//! One shared read projection of retained Core evidence. Scoped proof, leaks,
//! incomplete execution and cache eligibility remain distinct facts.
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{
    ControlFailure, DIAGNOSTIC_CONTRACT, DiagnosticFailure, DiagnosticOutcome, DiagnosticPlan,
    DiagnosticRun, DiagnosticStage, NativeCheck, RequiredCheck, ScopedNativeResult,
    framework::{
        FrameworkCheck, FrameworkControlFailure, FrameworkFailure, FrameworkFailureScope,
        ScopedFrameworkResult,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssessmentPhase {
    Running,
    Finished,
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
    pub checks: Vec<CheckAssessment>,
    pub native_controls: Vec<ControlFailure>,
    pub framework_controls: Vec<FrameworkControlFailure>,
    pub reuse_blockers: Vec<ReuseBlocker>,
    pub failure: Option<DiagnosticFailure>,
    pub cleanup_confirmed: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckAssessment {
    pub check: RequiredCheck,
    pub outcome: CheckOutcome,
    pub android_access_denied: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CheckOutcome {
    Proven,
    Leaked,
    Unverified { reason: UnverifiedReason },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UnverifiedReason {
    AwaitingEvidence,
    EvidenceUnavailable,
    AndroidAccessDenied,
    NativeControls {
        controls: Vec<ControlFailure>,
    },
    FrameworkControls {
        failures: Vec<FrameworkFailure>,
    },
    Interrupted {
        stage: DiagnosticStage,
        failure: crate::DiagnosticFailureKind,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticVerdict {
    Proven,
    Leaked,
    Unverified,
}

impl DiagnosticVerdict {
    pub fn for_checks<'a>(checks: impl IntoIterator<Item = &'a CheckAssessment>) -> Self {
        let mut any = false;
        let mut all_proven = true;
        for row in checks {
            any = true;
            match row.outcome {
                CheckOutcome::Leaked => return Self::Leaked,
                CheckOutcome::Proven => {}
                CheckOutcome::Unverified { .. } => all_proven = false,
            }
        }
        if any && all_proven {
            Self::Proven
        } else {
            Self::Unverified
        }
    }
}

impl DiagnosticAssessment {
    pub fn verdict(&self) -> DiagnosticVerdict {
        DiagnosticVerdict::for_checks(&self.checks)
    }

    pub fn leaked(&self) -> impl Iterator<Item = RequiredCheck> + '_ {
        self.checks
            .iter()
            .filter_map(|row| matches!(row.outcome, CheckOutcome::Leaked).then_some(row.check))
    }
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
fn framework_check(check: FrameworkCheck) -> RequiredCheck {
    match check {
        FrameworkCheck::Networks => RequiredCheck::FrameworkNetworks,
        FrameworkCheck::Capabilities => RequiredCheck::FrameworkCapabilities,
        FrameworkCheck::LinkProperties => RequiredCheck::FrameworkLinkProperties,
        FrameworkCheck::DefaultNetwork => RequiredCheck::FrameworkDefaultNetwork,
        FrameworkCheck::Callbacks => RequiredCheck::FrameworkCallbacks,
        FrameworkCheck::PendingIntent => RequiredCheck::FrameworkPendingIntent,
        FrameworkCheck::PackageVisibility => RequiredCheck::PackageVisibility,
        FrameworkCheck::LegacyTypeVpn | FrameworkCheck::LegacyEnumeration => {
            RequiredCheck::FrameworkLegacyInfo
        }
    }
}

pub(crate) fn framework_checks(
    checks: &[FrameworkCheck],
    require_both_legacy: bool,
) -> BTreeSet<RequiredCheck> {
    let legacy_complete = checks.contains(&FrameworkCheck::LegacyTypeVpn)
        && checks.contains(&FrameworkCheck::LegacyEnumeration);
    checks
        .iter()
        .copied()
        .map(framework_check)
        .filter(|check| {
            *check != RequiredCheck::FrameworkLegacyInfo || !require_both_legacy || legacy_complete
        })
        .collect()
}

fn network_view(check: RequiredCheck) -> bool {
    matches!(
        check,
        RequiredCheck::FrameworkNetworks
            | RequiredCheck::FrameworkCapabilities
            | RequiredCheck::FrameworkLinkProperties
    )
}

fn framework_failures(
    result: &ScopedFrameworkResult,
    check: RequiredCheck,
) -> Vec<FrameworkFailure> {
    result
        .failures
        .iter()
        .copied()
        .filter(|failure| match failure.scope {
            FrameworkFailureScope::Run => true,
            FrameworkFailureScope::NetworkView => network_view(check),
            FrameworkFailureScope::Check(source) => {
                let source = framework_check(source);
                source == check || (network_view(source) && network_view(check))
            }
        })
        .collect()
}

fn is_framework(check: RequiredCheck) -> bool {
    use RequiredCheck::*;
    match check {
        FrameworkNetworks
        | FrameworkDefaultNetwork
        | FrameworkCapabilities
        | FrameworkLinkProperties
        | FrameworkLegacyInfo
        | FrameworkCallbacks
        | FrameworkPendingIntent
        | PackageVisibility => true,
        IoctlFlags | IoctlMtu | Getifaddrs | NetlinkLinks | IoctlAddresses | BindToDevice
        | NetlinkAddresses | NetlinkRoutes | NetlinkRules | ProcRoutes | ProcInterfaces
        | SysInterfaces | Filesystem | Ports => false,
    }
}

fn add_controls(controls: &mut Vec<ControlFailure>, extra: &[ControlFailure]) {
    for &control in extra {
        if !controls.contains(&control) {
            controls.push(control);
        }
    }
}

impl DiagnosticRun {
    pub fn assessment(&self) -> DiagnosticAssessment {
        use RequiredCheck::*;
        let required = DiagnosticPlan::supported(self.plan.required.contains(&Filesystem)).required;
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
        let (phase, result, failure, cleanup_confirmed) = match &self.outcome {
            DiagnosticOutcome::Running => (AssessmentPhase::Running, None, None, None),
            DiagnosticOutcome::Finished { native, .. } => {
                (AssessmentPhase::Finished, Some(native), None, None)
            }
            DiagnosticOutcome::Infrastructure {
                failure,
                native,
                cleanup_confirmed,
            } => (
                AssessmentPhase::Infrastructure,
                native.as_ref(),
                Some(failure.clone()),
                *cleanup_confirmed,
            ),
        };
        let mut proven = BTreeSet::new();
        let mut leaked = BTreeSet::new();
        let mut native_controls = Vec::new();
        let mut scoped_controls =
            std::collections::BTreeMap::<RequiredCheck, Vec<ControlFailure>>::new();
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
                for check in [IoctlFlags, IoctlMtu, Getifaddrs, NetlinkLinks] {
                    scoped_controls.entry(check).or_default().push(*reason);
                }
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
            add_controls(&mut native_controls, &addresses.controls());
            for group in [IoctlAddresses, NetlinkAddresses, Getifaddrs] {
                let controls = scoped_controls.entry(group).or_default();
                for failure in &addresses.failures {
                    if failure.check.is_none_or(|check| address(check) == group) {
                        add_controls(controls, &[failure.reason]);
                    }
                }
            }
        }
        if let Some(proc_interfaces) = &self.proc_interfaces {
            if proc_interfaces.complete() {
                proven.insert(ProcInterfaces);
            }
            if !proc_interfaces.leaked.is_empty() {
                leaked.insert(ProcInterfaces);
            } else if proc_interfaces.all_access_denied() {
                native_controls.push(ControlFailure::ProcInterfaceRequestDenied);
            }
            add_controls(&mut native_controls, &proc_interfaces.controls);
            add_controls(
                scoped_controls.entry(ProcInterfaces).or_default(),
                &proc_interfaces.controls,
            );
        }
        if self.plan.required.contains(&Filesystem)
            && let Some(fs) = &self.filesystem
        {
            if fs.complete() {
                proven.insert(Filesystem);
            }
            if !fs.leaked.is_empty() {
                leaked.insert(Filesystem);
            }
            add_controls(&mut native_controls, &fs.controls);
            add_controls(scoped_controls.entry(Filesystem).or_default(), &fs.controls);
        }
        // Optional directory enumeration remains independent of path lookups.
        if self.plan.required.contains(&SysInterfaces)
            && let Some(sys) = &self.sys_interfaces
        {
            if sys.complete() {
                proven.insert(SysInterfaces);
            }
            if !sys.leaked.is_empty() {
                leaked.insert(SysInterfaces);
            }
            add_controls(&mut native_controls, &sys.controls);
            add_controls(
                scoped_controls.entry(SysInterfaces).or_default(),
                &sys.controls,
            );
        }
        if let Some(binding) = &self.binding {
            if binding.complete() {
                proven.insert(RequiredCheck::BindToDevice);
            }
            if !binding.leaked.is_empty() {
                leaked.insert(RequiredCheck::BindToDevice);
            }
            add_controls(&mut native_controls, &binding.controls);
            add_controls(
                scoped_controls.entry(BindToDevice).or_default(),
                &binding.controls,
            );
        }
        if let Some(routes) = &self.routes {
            let group_of = |check: &crate::RouteCheck| {
                if check.surface.rule() {
                    NetlinkRules
                } else if check.surface.netlink() {
                    NetlinkRoutes
                } else {
                    ProcRoutes
                }
            };
            for group in [NetlinkRoutes, ProcRoutes, NetlinkRules] {
                if crate::RouteCheck::required()
                    .into_iter()
                    .chain(crate::RouteCheck::rules())
                    .filter(|check| group_of(check) == group)
                    .all(|check| routes.proven.contains(&check))
                {
                    proven.insert(group);
                }
                if routes.leaked.iter().any(|check| group_of(check) == group) {
                    leaked.insert(group);
                }
            }
            if !proven.contains(&ProcRoutes)
                && !leaked.contains(&ProcRoutes)
                && crate::RouteCheck::required()
                    .into_iter()
                    .filter(|check| check.surface.proc_route())
                    .all(|check| {
                        routes
                            .access_denials
                            .iter()
                            .any(|denial| denial.check == check)
                    })
            {
                native_controls.push(ControlFailure::ProcRouteRequestDenied);
            }
            add_controls(&mut native_controls, &routes.controls());
            for group in [NetlinkRoutes, ProcRoutes, NetlinkRules] {
                let controls = scoped_controls.entry(group).or_default();
                for failure in &routes.failures {
                    if failure
                        .check
                        .as_ref()
                        .is_none_or(|check| group_of(check) == group)
                    {
                        add_controls(controls, &[failure.reason]);
                    }
                }
            }
        }
        // Root-only controls cannot establish the admitted app's RAW visibility.
        // Native results are paired with that sample by Core's shared evaluators;
        // retain qualified root-controlled leaks even if the RAW scope is absent.
        if self
            .native_raw
            .as_ref()
            .and_then(|r| r.qualified_sample())
            .is_none()
        {
            for check in std::mem::take(&mut proven) {
                add_controls(
                    scoped_controls.entry(check).or_default(),
                    &[ControlFailure::MissingPositiveControl],
                );
            }
            if phase != AssessmentPhase::Running
                && !native_controls.contains(&ControlFailure::MissingPositiveControl)
            {
                native_controls.push(ControlFailure::MissingPositiveControl);
            }
        }
        let framework_controls = if let Some(result) = &self.framework {
            proven.extend(framework_checks(&result.proven, true));
            leaked.extend(framework_checks(&result.leaked, false));
            result.controls()
        } else {
            Vec::new()
        };
        if required.contains(&Ports)
            && let Some(result) = &self.ports
        {
            if result.complete() {
                proven.insert(Ports);
            }
            if !result.leaked.is_empty() {
                leaked.insert(Ports);
            }
            add_controls(&mut native_controls, &result.controls);
            add_controls(scoped_controls.entry(Ports).or_default(), &result.controls);
        }
        if self.link_request_denial.is_some() {
            native_controls.push(ControlFailure::NetlinkLinkRequestDenied);
        }
        if let Some(kernel) = &self.kernel_filters {
            for (check, denial, status) in [
                (
                    NetlinkLinks,
                    ControlFailure::NetlinkLinkRequestDenied,
                    kernel.netlink_links,
                ),
                (
                    ProcRoutes,
                    ControlFailure::ProcRouteRequestDenied,
                    kernel.proc_routes,
                ),
                (
                    ProcInterfaces,
                    ControlFailure::ProcInterfaceRequestDenied,
                    kernel.proc_interfaces,
                ),
            ] {
                if status == crate::KernelFilterStatus::Proven && native_controls.contains(&denial)
                {
                    proven.insert(check);
                }
                if status == crate::KernelFilterStatus::Leaked {
                    leaked.insert(check);
                }
            }
        }
        let checks = required
            .into_iter()
            .map(|check| {
                let denial = match check {
                    NetlinkLinks => Some(ControlFailure::NetlinkLinkRequestDenied),
                    ProcRoutes => Some(ControlFailure::ProcRouteRequestDenied),
                    ProcInterfaces => Some(ControlFailure::ProcInterfaceRequestDenied),
                    _ => None,
                };
                let android_access_denied = denial.is_some_and(|d| native_controls.contains(&d));
                let outcome = if leaked.contains(&check) {
                    CheckOutcome::Leaked
                } else if proven.contains(&check) {
                    CheckOutcome::Proven
                } else {
                    let framework = is_framework(check);
                    let failures = self
                        .framework
                        .as_ref()
                        .filter(|_| framework)
                        .map_or_else(Vec::new, |result| framework_failures(result, check));
                    let controls = scoped_controls.remove(&check).unwrap_or_default();
                    let interruption = failure.as_ref().filter(|failure| match failure.stage {
                        DiagnosticStage::FrameworkAndPorts => framework || check == Ports,
                        DiagnosticStage::Cleanup => false,
                        _ => true,
                    });
                    let reason = if let Some(failure) = interruption
                        && (failures.is_empty()
                            || failure.kind == crate::DiagnosticFailureKind::Timeout)
                    {
                        UnverifiedReason::Interrupted {
                            stage: failure.stage,
                            failure: failure.kind,
                        }
                    } else if !failures.is_empty() {
                        UnverifiedReason::FrameworkControls { failures }
                    } else if android_access_denied {
                        UnverifiedReason::AndroidAccessDenied
                    } else if !controls.is_empty() {
                        UnverifiedReason::NativeControls { controls }
                    } else if phase == AssessmentPhase::Running {
                        UnverifiedReason::AwaitingEvidence
                    } else {
                        UnverifiedReason::EvidenceUnavailable
                    };
                    CheckOutcome::Unverified { reason }
                };
                CheckAssessment {
                    check,
                    outcome,
                    android_access_denied,
                }
            })
            .collect();
        DiagnosticAssessment {
            phase,
            checks,
            native_controls,
            framework_controls,
            reuse_blockers,
            failure,
            cleanup_confirmed,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    impl DiagnosticAssessment {
        fn proven_ids(&self) -> Vec<RequiredCheck> {
            self.checks
                .iter()
                .filter_map(|row| matches!(row.outcome, CheckOutcome::Proven).then_some(row.check))
                .collect()
        }
        fn leaked_ids(&self) -> Vec<RequiredCheck> {
            self.leaked().collect()
        }
        fn unverified_ids(&self) -> Vec<RequiredCheck> {
            self.checks
                .iter()
                .filter_map(|row| {
                    matches!(row.outcome, CheckOutcome::Unverified { .. }).then_some(row.check)
                })
                .collect()
        }
    }
    use crate::{DiagnosticFingerprint, framework::ScopedFrameworkResult};
    fn thread_identity() -> crate::DiagnosticThreadIdentity {
        crate::DiagnosticThreadIdentity {
            uid: 10000,
            pid: 42,
            process_start_ticks: 100,
            tid: 43,
            thread_start_ticks: 101,
        }
    }
    pub(crate) fn paired_receipt() -> crate::NativeRawReceipt {
        crate::NativeRawReceipt::Paired {
            identity: crate::DiagnosticThreadIdentity {
                uid: 10000,
                pid: 42,
                process_start_ticks: 100,
                tid: 43,
                thread_start_ticks: 101,
            },
            sample: crate::native_pair::tests::fixture().2,
        }
    }
    #[test]
    fn kernel_link_coverage_retains_android_denial_and_prioritizes_a_leak() {
        use crate::KernelFilterStatus::*;
        use RequiredCheck::NetlinkLinks;
        let mut run = run(ScopedNativeResult::Proven { checks: vec![] });
        run.kernel_filters = Some(crate::KernelFilterResult {
            identity: thread_identity(),
            selinux_context: "u:r:ksu:s0".into(),
            netlink_links: Proven,
            proc_interfaces: Unverified,
            proc_routes: Unverified,
        });
        assert!(!run.assessment().proven_ids().contains(&NetlinkLinks));
        run.link_request_denial = Some(crate::LinkRequestDenial {
            raw_errno: 13,
            protected_errno: 13,
        });
        let assessment = run.assessment();
        assert!(assessment.proven_ids().contains(&NetlinkLinks));
        assert!(
            assessment
                .native_controls
                .contains(&ControlFailure::NetlinkLinkRequestDenied)
        );
        run.kernel_filters.as_mut().unwrap().netlink_links = Leaked;
        let assessment = run.assessment();
        assert!(assessment.leaked_ids().contains(&NetlinkLinks));
        assert!(!assessment.proven_ids().contains(&NetlinkLinks));
        assert!(
            assessment
                .native_controls
                .contains(&ControlFailure::NetlinkLinkRequestDenied)
        );
    }

    fn run(native: ScopedNativeResult) -> DiagnosticRun {
        DiagnosticRun {
            kernel_filters: None,
            link_request_denial: None,
            ports: None,
            native_raw: Some(paired_receipt()),
            routes: None,
            filesystem: None,
            sys_interfaces: None,
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
            plan: DiagnosticPlan::supported(false),
            stale: false,
            framework: None,
            outcome: DiagnosticOutcome::Finished { native },
        }
    }
    #[test]
    fn routes_require_each_actor_and_family_and_keep_leaks_without_a_raw_pair() {
        let checks = crate::RouteCheck::required();
        let mut run = run(ScopedNativeResult::Proven { checks: vec![] });
        let netlink: Vec<_> = checks
            .iter()
            .copied()
            .filter(|check| check.surface.netlink())
            .collect();
        let leak = checks
            .iter()
            .copied()
            .find(|check| !check.surface.netlink())
            .unwrap();
        let mut evidence = crate::ScopedRouteResult {
            proven: netlink[1..].to_vec(),
            leaked: vec![leak],
            ..Default::default()
        };
        run.record_routes(evidence.clone());
        assert!(
            run.assessment()
                .unverified_ids()
                .contains(&RequiredCheck::NetlinkRoutes)
        );
        evidence.proven = netlink;
        evidence.proven.extend(crate::RouteCheck::rules().skip(1));
        run.record_routes(evidence.clone());
        assert!(
            run.assessment()
                .unverified_ids()
                .contains(&RequiredCheck::NetlinkRules)
        );
        evidence.proven.extend(crate::RouteCheck::rules().take(1));
        run.record_routes(evidence);
        assert!(
            run.assessment()
                .proven_ids()
                .contains(&RequiredCheck::NetlinkRoutes)
        );
        assert!(
            run.assessment()
                .leaked_ids()
                .contains(&RequiredCheck::ProcRoutes)
        );
        assert!(
            run.assessment()
                .proven_ids()
                .contains(&RequiredCheck::NetlinkRules)
        );
        run.native_raw = None;
        assert!(
            run.assessment()
                .unverified_ids()
                .contains(&RequiredCheck::NetlinkRoutes)
        );
        assert!(
            run.assessment()
                .leaked_ids()
                .contains(&RequiredCheck::ProcRoutes)
        );
    }

    #[test]
    fn directory_evidence_requires_enabled_feature_and_cannot_complete_stat_access_open() {
        let mut run = run(ScopedNativeResult::Proven { checks: vec![] });
        run.sys_interfaces = Some(crate::ScopedSysInterfaceResult {
            proven: crate::SysInterfaceCheck::required(),
            ..Default::default()
        });
        assert!(
            !run.assessment()
                .proven_ids()
                .contains(&RequiredCheck::SysInterfaces)
        );
        run.plan = DiagnosticPlan::supported(true);
        assert!(
            run.assessment()
                .proven_ids()
                .contains(&RequiredCheck::SysInterfaces)
        );
        assert!(
            run.assessment()
                .unverified_ids()
                .contains(&RequiredCheck::Filesystem)
        );
        run.native_raw = None;
        assert!(
            !run.assessment()
                .proven_ids()
                .contains(&RequiredCheck::SysInterfaces)
        );
        run.sys_interfaces
            .as_mut()
            .unwrap()
            .leaked
            .push(crate::SysInterfaceCheck::required()[0]);
        assert!(
            run.assessment()
                .leaked_ids()
                .contains(&RequiredCheck::SysInterfaces)
        );
        run.plan = DiagnosticPlan::supported(false);
        assert!(
            !run.assessment()
                .leaked_ids()
                .contains(&RequiredCheck::SysInterfaces)
        );
    }
    #[test]
    fn lookup_proof_is_optional_requires_raw_and_cannot_substitute_for_directory_coverage() {
        let mut run = run(ScopedNativeResult::Proven { checks: vec![] });
        run.filesystem = Some(crate::ScopedFilesystemResult {
            proven: crate::FilesystemCheck::required(),
            ..Default::default()
        });
        assert!(
            !run.assessment()
                .proven_ids()
                .contains(&RequiredCheck::Filesystem)
        );
        run.plan = DiagnosticPlan::supported(true);
        assert!(
            run.assessment()
                .proven_ids()
                .contains(&RequiredCheck::Filesystem)
        );
        assert!(
            run.assessment()
                .unverified_ids()
                .contains(&RequiredCheck::SysInterfaces)
        );
        run.native_raw = None;
        assert!(
            !run.assessment()
                .proven_ids()
                .contains(&RequiredCheck::Filesystem)
        );
        run.filesystem
            .as_mut()
            .unwrap()
            .leaked
            .push(crate::FilesystemCheck::required()[0]);
        assert!(
            run.assessment()
                .leaked_ids()
                .contains(&RequiredCheck::Filesystem)
        );
        run.plan = DiagnosticPlan::supported(false);
        assert!(
            !run.assessment()
                .leaked_ids()
                .contains(&RequiredCheck::Filesystem)
        );
    }
    #[test]
    fn root_only_or_failed_raw_receipts_withhold_native_proof_without_erasing_leaks() {
        for receipt in [
            None,
            Some(crate::NativeRawReceipt::UnsupportedBackend {
                identity: thread_identity(),
            }),
            Some(crate::NativeRawReceipt::Failed {
                identity: thread_identity(),
                stage: crate::NativeRawStage::Revocation,
                sample: Some(crate::native_pair::tests::fixture().2),
            }),
        ] {
            let mut r = run(ScopedNativeResult::Proven {
                checks: vec![NativeCheck::IoctlFlags],
            });
            r.native_raw = receipt;
            assert!(r.assessment().proven_ids().is_empty());
            assert!(
                r.assessment()
                    .unverified_ids()
                    .contains(&RequiredCheck::IoctlFlags)
            );
            r.outcome = DiagnosticOutcome::Finished {
                native: ScopedNativeResult::Leaked {
                    checks: vec![NativeCheck::IoctlFlags],
                    incomplete: vec![],
                },
            };
            assert!(
                r.assessment()
                    .leaked_ids()
                    .contains(&RequiredCheck::IoctlFlags)
            );
        }
    }
    #[test]
    fn previous_or_truncated_plan_cannot_be_reused_as_current_proof() {
        let mut evidence = run(ScopedNativeResult::Proven { checks: vec![] });
        evidence.plan.contract = DIAGNOSTIC_CONTRACT - 1;
        evidence
            .plan
            .required
            .retain(|c| !matches!(c, RequiredCheck::PackageVisibility | RequiredCheck::Ports));
        assert!(
            evidence
                .assessment()
                .reuse_blockers
                .contains(&ReuseBlocker::UnsupportedPlan)
        );
        let assessment = evidence.assessment();
        assert!(
            assessment
                .unverified_ids()
                .contains(&RequiredCheck::PackageVisibility)
        );
        assert!(assessment.unverified_ids().contains(&RequiredCheck::Ports));
        evidence.plan = DiagnosticPlan::supported(true);
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
        assert!(
            assessment
                .checks
                .iter()
                .any(|row| row.check == RequiredCheck::SysInterfaces)
        );
        assert!(
            assessment
                .unverified_ids()
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
                failure: DiagnosticFailure {
                    stage: DiagnosticStage::FrameworkAndPorts,
                    kind: crate::DiagnosticFailureKind::Unavailable,
                    detail: "worker died".into(),
                },
                native: Some(native),
                cleanup_confirmed: Some(false),
            };
            run.framework = Some(ScopedFrameworkResult {
                proven: vec![FrameworkCheck::Networks],
                leaked: vec![FrameworkCheck::Networks],
                failures: vec![FrameworkFailure {
                    scope: FrameworkFailureScope::Run,
                    phase: None,
                    reason: FrameworkControlFailure::CleanupUnconfirmed,
                }],
                ..Default::default()
            });
            let a = run.assessment();
            assert_eq!(a.phase, AssessmentPhase::Infrastructure);
            assert_eq!(
                a.leaked_ids(),
                [RequiredCheck::IoctlMtu, RequiredCheck::FrameworkNetworks]
            );
            assert!(a.proven_ids().is_empty());
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
        assert_eq!(a.phase, AssessmentPhase::Finished);
        assert_eq!(a.proven_ids().len(), 6);
        assert_eq!(a.unverified_ids().len(), 14);
        assert!(
            !a.checks
                .iter()
                .any(|row| row.check == RequiredCheck::SysInterfaces)
        );
        assert!(a.unverified_ids().contains(&RequiredCheck::Getifaddrs));
        assert!(
            a.reuse_blockers
                .contains(&ReuseBlocker::LoadedGenerationsIncomplete)
        );
        assert!(!a.reuse_blockers.contains(&ReuseBlocker::ConditionsChanged));
        run.plan.required.clear();
        let a = run.assessment();
        assert_eq!(a.checks.len(), 20);
        assert!(a.reuse_blockers.contains(&ReuseBlocker::UnsupportedPlan));
    }

    #[test]
    fn check_reasons_stay_with_their_own_evidence_and_verdicts_are_derived() {
        use crate::framework::FrameworkPhase;
        let mut run = run(ScopedNativeResult::Proven { checks: vec![] });
        let pending_failure = FrameworkFailure {
            scope: FrameworkFailureScope::Check(FrameworkCheck::PendingIntent),
            phase: Some(FrameworkPhase::Revoked),
            reason: FrameworkControlFailure::ProbeUnavailable,
        };
        run.framework = Some(ScopedFrameworkResult {
            proven: vec![FrameworkCheck::Callbacks],
            leaked: vec![FrameworkCheck::PackageVisibility],
            failures: vec![pending_failure],
            ..Default::default()
        });
        let assessment = run.assessment();
        let row = |check| {
            assessment
                .checks
                .iter()
                .find(|row| row.check == check)
                .unwrap()
        };
        assert_eq!(
            row(RequiredCheck::FrameworkCallbacks).outcome,
            CheckOutcome::Proven
        );
        assert_eq!(
            row(RequiredCheck::FrameworkPendingIntent).outcome,
            CheckOutcome::Unverified {
                reason: UnverifiedReason::FrameworkControls {
                    failures: vec![pending_failure]
                },
            }
        );
        assert_eq!(
            row(RequiredCheck::FrameworkDefaultNetwork).outcome,
            CheckOutcome::Unverified {
                reason: UnverifiedReason::EvidenceUnavailable,
            }
        );
        assert_eq!(assessment.verdict(), DiagnosticVerdict::Leaked);
        assert_eq!(
            DiagnosticVerdict::for_checks([row(RequiredCheck::FrameworkCallbacks)]),
            DiagnosticVerdict::Proven
        );
        assert_eq!(
            DiagnosticVerdict::for_checks([]),
            DiagnosticVerdict::Unverified
        );
    }

    #[test]
    fn framework_and_ports_timeout_preserves_confirmed_native_evidence() {
        let native = ScopedNativeResult::Proven {
            checks: vec![NativeCheck::IoctlFlags],
        };
        let mut run = run(native.clone());
        run.outcome = DiagnosticOutcome::Infrastructure {
            failure: DiagnosticFailure {
                stage: DiagnosticStage::FrameworkAndPorts,
                kind: crate::DiagnosticFailureKind::Timeout,
                detail: "deadline".into(),
            },
            native: Some(native),
            cleanup_confirmed: Some(true),
        };
        let assessment = run.assessment();
        assert_eq!(assessment.proven_ids(), [RequiredCheck::IoctlFlags]);
        for check in [RequiredCheck::FrameworkCallbacks, RequiredCheck::Ports] {
            assert_eq!(
                assessment
                    .checks
                    .iter()
                    .find(|row| row.check == check)
                    .unwrap()
                    .outcome,
                CheckOutcome::Unverified {
                    reason: UnverifiedReason::Interrupted {
                        stage: DiagnosticStage::FrameworkAndPorts,
                        failure: crate::DiagnosticFailureKind::Timeout
                    },
                }
            );
        }
        assert_eq!(assessment.verdict(), DiagnosticVerdict::Unverified);
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
        assert_eq!(assessment.proven_ids().len(), 3);
        assert_eq!(assessment.leaked_ids(), vec![RequiredCheck::BindToDevice]);
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
                .proven_ids()
                .contains(&RequiredCheck::BindToDevice)
        );
        assert!(run.assessment().unverified_ids().len() >= 14);
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
                .proven_ids()
                .contains(&RequiredCheck::ProcInterfaces)
        );
        let checks = run.proc_interfaces.as_mut().unwrap();
        let missing = checks.proven.pop().unwrap();
        assert!(
            run.assessment()
                .unverified_ids()
                .contains(&RequiredCheck::ProcInterfaces)
        );
        run.proc_interfaces.as_mut().unwrap().leaked.push(missing);
        assert!(
            run.assessment()
                .leaked_ids()
                .contains(&RequiredCheck::ProcInterfaces)
        );
        assert!(
            !run.assessment()
                .proven_ids()
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
                .unverified_ids()
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
            assert!(run.assessment().proven_ids().contains(&group));
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
        assert!(result.unverified_ids().contains(&RequiredCheck::Getifaddrs));
        assert!(result.leaked_ids().contains(&RequiredCheck::IoctlAddresses));
        assert!(!result.proven_ids().contains(&RequiredCheck::IoctlAddresses));
        assert!(
            result
                .proven_ids()
                .contains(&RequiredCheck::NetlinkAddresses)
        );
    }
}
