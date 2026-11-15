//! Proof from Core's own capability-free process, separate from Android app
//! access. Each check needs a positive RAW observation from this process.
use crate::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelFilterStatus {
    Unverified,
    Proven,
    Leaked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KernelFilterResult {
    pub identity: DiagnosticThreadIdentity,
    pub selinux_context: String,
    pub netlink_links: KernelFilterStatus,
    pub proc_interfaces: KernelFilterStatus,
    pub proc_routes: KernelFilterStatus,
}

pub fn evaluate_kernel_interfaces(
    spec: &NativeFixtureSpec,
    samples: [&NativeMeasurement; 5],
    raw: &NativeMeasurement,
) -> [KernelFilterStatus; 2] {
    use KernelFilterStatus::*;
    let [negative, start, protected, end, cleanup] = samples;
    let link_check = NativeCheck::NetlinkGetlink;
    let links = if raw.bounded() && native_raw_positive(spec, raw, link_check) {
        match evaluate_native_fixture(spec, negative, start, protected, end, cleanup) {
            ScopedNativeResult::Proven { checks } if checks.contains(&link_check) => Proven,
            ScopedNativeResult::Leaked { checks, .. } if checks.contains(&link_check) => Leaked,
            ScopedNativeResult::Unverified { observed_leaks, .. }
                if observed_leaks.contains(&link_check) =>
            {
                Leaked
            }
            _ => Unverified,
        }
    } else {
        Unverified
    };
    let root = evaluate_native_proc_interface_fixture(spec, samples);
    let paired =
        evaluate_native_proc_interface_fixture(spec, [negative, raw, protected, end, cleanup]);
    let proc = if root
        .leaked
        .iter()
        .any(|check| paired.leaked.contains(check))
    {
        Leaked
    } else if root.complete() && paired.complete() {
        Proven
    } else {
        Unverified
    };
    [links, proc]
}

pub fn evaluate_kernel_proc_routes(
    spec: &RouteFixtureSpec,
    samples: [Option<&RouteMeasurement>; 5],
    raw: &RouteMeasurement,
    cleaned: bool,
) -> KernelFilterStatus {
    use KernelFilterStatus::*;
    let [negative, _, protected, end, cleanup] = samples;
    let root = evaluate_paired_route_fixture(spec, samples, Some(raw), cleaned);
    let paired = evaluate_route_fixture(
        spec,
        [negative, Some(raw), protected, end, cleanup],
        cleaned,
    );
    let checks = RouteCheck::required()
        .into_iter()
        .filter(|check| check.surface.proc_route());
    if root
        .leaked
        .iter()
        .any(|check| check.surface.proc_route() && paired.leaked.contains(check))
    {
        Leaked
    } else if checks.into_iter().all(|check| root.proven.contains(&check)) {
        Proven
    } else {
        Unverified
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_link_proof_needs_its_own_raw_and_protected_controls() {
        use KernelFilterStatus::*;
        let (spec, absent, visible) = crate::native_pair::tests::fixture();
        let evaluate = |raw, protected, cleanup| {
            evaluate_kernel_interfaces(
                &spec,
                [&absent, &visible, protected, &visible, cleanup],
                raw,
            )[0]
        };
        assert_eq!(evaluate(&visible, &absent, &absent), Proven);
        assert_eq!(evaluate(&absent, &absent, &absent), Unverified);
        assert_eq!(evaluate(&visible, &absent, &visible), Unverified);
        assert_eq!(evaluate(&visible, &visible, &visible), Leaked);
        assert_eq!(evaluate(&absent, &visible, &absent), Unverified);
    }
}
