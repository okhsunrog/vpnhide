//! Require both root controls and a positively observed same-worker RAW sample.
//! Missing RAW withholds proof; it cannot erase an independently qualified leak.
use crate::*;

pub struct PairedNativeResult {
    pub filesystem: ScopedFilesystemResult,
    pub sys_interfaces: ScopedSysInterfaceResult,
    pub native: ScopedNativeResult,
    pub addresses: ScopedAddressResult,
    pub binding: ScopedBindResult,
    pub proc_interfaces: ScopedProcInterfaceResult,
}
fn merge<T: Copy + PartialEq>(
    proven: &mut Vec<T>,
    leaked: &mut Vec<T>,
    controls: &mut Vec<ControlFailure>,
    app_proven: &[T],
    app_leaked: &[T],
    app_controls: &[ControlFailure],
) {
    proven.retain(|v| app_proven.contains(v));
    for v in app_leaked {
        if !leaked.contains(v) {
            leaked.push(*v);
        }
    }
    for v in app_controls {
        if !controls.contains(v) {
            controls.push(*v);
        }
    }
    proven.retain(|v| !leaked.contains(v));
}
#[cfg(test)]
fn leaks(value: &ScopedNativeResult) -> &[NativeCheck] {
    match value {
        ScopedNativeResult::Proven { .. } => &[],
        ScopedNativeResult::Leaked { checks, .. } => checks,
        ScopedNativeResult::Unverified { observed_leaks, .. } => observed_leaks,
    }
}
fn qualify_native_raw(
    mut result: ScopedNativeResult,
    spec: &NativeFixtureSpec,
    raw: &NativeMeasurement,
) -> ScopedNativeResult {
    if let ScopedNativeResult::Proven { checks } = &mut result {
        let bounded = raw.bounded();
        checks.retain(|check| bounded && crate::native_raw_positive(spec, raw, *check));
        if checks.is_empty() {
            return ScopedNativeResult::Unverified {
                reason: ControlFailure::MissingPositiveControl,
                observed_leaks: Vec::new(),
            };
        }
    }
    result
}
/// `app_raw` is supplied only after Core correlates the active interval and
/// confirms revocation. The app cannot label its own observation protected/RAW.
pub fn evaluate_paired_native_fixture(
    spec: &NativeFixtureSpec,
    samples: [&NativeMeasurement; 5],
    app_raw: Option<&NativeMeasurement>,
) -> PairedNativeResult {
    let unavailable = NativeMeasurement::unavailable(ProbeError::Unsupported);
    let raw = app_raw.unwrap_or(&unavailable);
    let [negative, root_start, worker, root_end, cleanup] = samples;
    let app_samples = [negative, raw, worker, root_end, cleanup];
    let native = qualify_native_raw(
        evaluate_native_fixture(spec, negative, root_start, worker, root_end, cleanup),
        spec,
        raw,
    );
    let mut addresses = evaluate_native_address_fixture(spec, samples);
    let app_addresses = evaluate_native_address_fixture(spec, app_samples);
    merge(
        &mut addresses.proven,
        &mut addresses.leaked,
        &mut addresses.controls,
        &app_addresses.proven,
        &app_addresses.leaked,
        &app_addresses.controls,
    );
    let mut proc_interfaces = evaluate_native_proc_interface_controls(spec, samples, app_raw);
    if !proc_interfaces.all_access_denied() {
        let app_proc = evaluate_native_proc_interface_fixture(spec, app_samples);
        merge(
            &mut proc_interfaces.proven,
            &mut proc_interfaces.leaked,
            &mut proc_interfaces.controls,
            &app_proc.proven,
            &app_proc.leaked,
            &app_proc.controls,
        );
    }
    let mut sys_interfaces =
        evaluate_sys_interface_fixture(spec, samples.map(|m| m.sys_interfaces.as_ref()));
    let app_sys =
        evaluate_sys_interface_fixture(spec, app_samples.map(|m| m.sys_interfaces.as_ref()));
    merge(
        &mut sys_interfaces.proven,
        &mut sys_interfaces.leaked,
        &mut sys_interfaces.controls,
        &app_sys.proven,
        &app_sys.leaked,
        &app_sys.controls,
    );
    let mut filesystem = evaluate_filesystem_fixture(spec, samples.map(|m| m.filesystem.as_ref()));
    let app_fs = evaluate_filesystem_fixture(spec, app_samples.map(|m| m.filesystem.as_ref()));
    merge(
        &mut filesystem.proven,
        &mut filesystem.leaked,
        &mut filesystem.controls,
        &app_fs.proven,
        &app_fs.leaked,
        &app_fs.controls,
    );
    let mut binding = evaluate_bind_fixture(spec, negative, root_start, worker, root_end, cleanup);
    let app_binding = evaluate_bind_fixture(spec, negative, raw, worker, root_end, cleanup);
    merge(
        &mut binding.proven,
        &mut binding.leaked,
        &mut binding.controls,
        &app_binding.proven,
        &app_binding.leaked,
        &app_binding.controls,
    );
    PairedNativeResult {
        filesystem,
        sys_interfaces,
        native,
        addresses,
        binding,
        proc_interfaces,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn fixture() -> (NativeFixtureSpec, NativeMeasurement, NativeMeasurement) {
        let ordinary = InterfaceId {
            name: "lo".into(),
            index: 1,
        };
        let spec = NativeFixtureSpec {
            session: 7,
            interface: InterfaceId {
                name: "tun000000000007".into(),
                index: 42,
            },
            flags: 4240,
            mtu: 1543,
            ordinary: OrdinaryControl {
                interface: ordinary.clone(),
                flags: 73,
                mtu: 65536,
            },
        };
        let lo = ObservedInterface {
            name: ordinary.name,
            index: Some(ordinary.index),
            ifaddrs: None,
        };
        let fixture = ObservedInterface {
            name: spec.interface.name.clone(),
            index: Some(spec.interface.index),
            ifaddrs: None,
        };
        let negative = NativeMeasurement {
            routes: None,
            filesystem: None,
            sys_interfaces: None,
            proc_interfaces: None,
            addresses: None,
            binding: None,
            flags: Sample::Error(ProbeError::Errno(19)),
            mtu: Sample::Error(ProbeError::Errno(19)),
            ifaddrs: Sample::Value(vec![lo.clone()]),
            links: Sample::Value(vec![lo.clone()]),
            ordinary_flags: Sample::Value(73),
            ordinary_mtu: Sample::Value(65536),
        };
        let mut raw = negative.clone();
        raw.flags = Sample::Value(4240);
        raw.mtu = Sample::Value(1543);
        raw.ifaddrs = Sample::Value(vec![lo.clone(), fixture.clone()]);
        raw.links = Sample::Value(vec![lo, fixture]);
        (spec, negative, raw)
    }
    #[test]
    fn root_visible_but_app_raw_missing_or_hidden_cannot_prove_concealment() {
        let (spec, n, r) = fixture();
        for raw in [None, Some(&n)] {
            let result = evaluate_paired_native_fixture(&spec, [&n, &r, &n, &r, &n], raw);
            assert!(
                matches!(result.native,ScopedNativeResult::Unverified { observed_leaks, .. } if observed_leaks.is_empty())
            );
        }
        assert!(
            matches!(evaluate_paired_native_fixture(&spec,[&n,&r,&n,&r,&n],Some(&r)).native,
            ScopedNativeResult::Proven { checks } if checks.len()==4)
        );
    }
    #[test]
    fn missing_app_raw_and_later_controls_do_not_erase_a_qualified_protected_leak() {
        let (spec, n, r) = fixture();
        let missing = NativeMeasurement::unavailable(ProbeError::Timeout);
        for raw in [None, Some(&r)] {
            let result =
                evaluate_paired_native_fixture(&spec, [&n, &r, &r, &missing, &missing], raw);
            assert_eq!(leaks(&result.native).len(), 4);
            assert!(!matches!(result.native, ScopedNativeResult::Proven { .. }));
        }
    }
}
