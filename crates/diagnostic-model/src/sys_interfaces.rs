//! Controlled directory enumeration, distinct from stat/access/open coverage.
use crate::{ControlFailure, MAX_INTERFACES, NativeFixtureSpec, NativeReadActor, Sample};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SysInterfaceSurface {
    SysClassNet,
    ProcIpv4Conf,
    ProcIpv6Conf,
}
impl SysInterfaceSurface {
    pub const ALL: [Self; 3] = [Self::SysClassNet, Self::ProcIpv4Conf, Self::ProcIpv6Conf];
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SysInterfaceCheck {
    pub actor: NativeReadActor,
    pub surface: SysInterfaceSurface,
}
impl SysInterfaceCheck {
    pub fn required() -> Vec<Self> {
        NativeReadActor::ALL
            .into_iter()
            .flat_map(|actor| {
                SysInterfaceSurface::ALL
                    .into_iter()
                    .map(move |surface| Self { actor, surface })
            })
            .collect()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SysInterfaceObservation {
    pub check: SysInterfaceCheck,
    pub names: Sample<Vec<String>>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SysInterfaceMeasurement {
    pub entries: Vec<SysInterfaceObservation>,
}
impl SysInterfaceMeasurement {
    pub fn bounded(&self) -> bool {
        let mut seen = BTreeSet::new();
        self.entries.len() <= SysInterfaceCheck::required().len()
            && self.entries.iter().all(|e| {
                seen.insert(e.check)
                    && e.names.value().is_none_or(|names| {
                        names.len() <= MAX_INTERFACES + 2
                            && names.iter().collect::<BTreeSet<_>>().len() == names.len()
                            && names.iter().all(|n| {
                                !n.is_empty()
                                    && n.len() <= 15
                                    && n.is_ascii()
                                    && !n.bytes().any(|b| b.is_ascii_control() || b == b'/')
                            })
                    })
            })
    }
    fn names(&self, check: SysInterfaceCheck) -> Option<&Vec<String>> {
        self.entries
            .iter()
            .find(|e| e.check == check)?
            .names
            .value()
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopedSysInterfaceResult {
    pub proven: Vec<SysInterfaceCheck>,
    pub leaked: Vec<SysInterfaceCheck>,
    pub controls: Vec<ControlFailure>,
}
impl ScopedSysInterfaceResult {
    pub fn complete(&self) -> bool {
        self.controls.is_empty()
            && self.leaked.is_empty()
            && SysInterfaceCheck::required()
                .iter()
                .all(|c| self.proven.contains(c))
    }
    fn problem(&mut self, failure: ControlFailure) {
        if !self.controls.contains(&failure) {
            self.controls.push(failure);
        }
    }
}
pub fn evaluate_sys_interface_fixture(
    spec: &NativeFixtureSpec,
    samples: [Option<&SysInterfaceMeasurement>; 5],
) -> ScopedSysInterfaceResult {
    let mut result = ScopedSysInterfaceResult::default();
    if !spec.valid() || samples[..3].iter().flatten().any(|m| !m.bounded()) {
        result.problem(ControlFailure::InvalidSpec);
        return result;
    }
    if samples[3..].iter().flatten().any(|m| !m.bounded()) {
        result.problem(ControlFailure::InvalidSpec);
    }
    for check in SysInterfaceCheck::required() {
        let [n, s, w, e, c] =
            samples.map(|sample| sample.filter(|m| m.bounded()).and_then(|m| m.names(check)));
        let [Some(n), Some(s), Some(w)] = [n, s, w] else {
            result.problem(ControlFailure::ProbeUnavailable);
            continue;
        };
        let ordinary = |names: &Vec<String>| names.contains(&spec.ordinary.interface.name);
        let fixture = |names: &Vec<String>| names.contains(&spec.interface.name);
        if fixture(n) || !ordinary(n) {
            result.problem(ControlFailure::MissingNegativeControl);
            continue;
        }
        if !fixture(s) || !ordinary(s) {
            result.problem(ControlFailure::MissingPositiveControl);
            continue;
        }
        let leaked = fixture(w);
        if leaked {
            result.leaked.push(check);
        }
        let mut controlled = true;
        if !ordinary(w) {
            result.problem(ControlFailure::OrdinaryObjectsChanged);
            controlled = false;
        }
        for failure in crate::later_control_failures(e, ControlFailure::FixtureChanged, |v| {
            fixture(v) && ordinary(v)
        })
        .chain(crate::later_control_failures(
            c,
            ControlFailure::CleanupUnconfirmed,
            |v| !fixture(v) && ordinary(v),
        )) {
            result.problem(failure);
            controlled = false;
        }
        if controlled && !leaked {
            result.proven.push(check);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProbeError, evaluate_paired_native_fixture};
    fn sample(present: bool) -> SysInterfaceMeasurement {
        let mut names = vec!["lo".to_owned(), "eth0".to_owned()];
        if present {
            names.push("tun000000000007".into());
        }
        SysInterfaceMeasurement {
            entries: SysInterfaceCheck::required()
                .into_iter()
                .map(|check| SysInterfaceObservation {
                    check,
                    names: Sample::Value(names.clone()),
                })
                .collect(),
        }
    }
    #[test]
    fn six_independent_positive_and_ordinary_controls_are_required() {
        let (spec, _, _) = crate::native_pair::tests::fixture();
        let n = sample(false);
        let s = sample(true);
        let w = sample(false);
        let result = evaluate_sys_interface_fixture(
            &spec,
            [Some(&n), Some(&s), Some(&w), Some(&s), Some(&n)],
        );
        assert!(result.complete());
        assert_eq!(result.proven.len(), 6);
        assert_eq!(result.proven, SysInterfaceCheck::required());
        for i in 0..6 {
            let mut partial = w.clone();
            partial.entries.remove(i);
            assert!(
                !evaluate_sys_interface_fixture(
                    &spec,
                    [Some(&n), Some(&s), Some(&partial), Some(&s), Some(&n)]
                )
                .complete()
            );
            partial = w.clone();
            partial.entries[i].names = Sample::Value(vec![]);
            let failed = evaluate_sys_interface_fixture(
                &spec,
                [Some(&n), Some(&s), Some(&partial), Some(&s), Some(&n)],
            );
            assert!(!failed.complete());
            assert!(
                failed
                    .controls
                    .contains(&ControlFailure::OrdinaryObjectsChanged)
            );
        }
    }
    #[test]
    fn denied_or_empty_positive_control_cannot_prove_hiding_or_qualify_a_leak() {
        let (spec, _, _) = crate::native_pair::tests::fixture();
        let n = sample(false);
        let s = sample(true);
        let w = sample(true);
        for invalid in [None, Some(&n)] {
            let result = evaluate_sys_interface_fixture(
                &spec,
                [Some(&n), invalid, Some(&w), Some(&s), Some(&n)],
            );
            assert!(result.proven.is_empty());
            assert!(result.leaked.is_empty());
        }
        let mut denied = s.clone();
        denied
            .entries
            .iter_mut()
            .for_each(|e| e.names = Sample::Error(ProbeError::Errno(13)));
        assert!(
            !evaluate_sys_interface_fixture(
                &spec,
                [Some(&n), Some(&s), Some(&denied), Some(&s), Some(&n)]
            )
            .complete()
        );
        for m in [&n, &s] {
            let mut malformed = m.clone();
            malformed.entries.push(malformed.entries[0].clone());
            assert!(!malformed.bounded());
        }
    }
    #[test]
    fn qualified_leaks_survive_late_failures_without_successful_proof() {
        let (spec, _, _) = crate::native_pair::tests::fixture();
        let n = sample(false);
        let s = sample(true);
        let mut w = n.clone();
        w.entries[1].names = Sample::Value(vec![spec.interface.name.clone()]); // ordinary preservation also fails
        let check = w.entries[1].check;
        for (end, cleanup, reason) in [
            (None, Some(&n), ControlFailure::FixtureChanged),
            (Some(&s), None, ControlFailure::CleanupUnconfirmed),
            (Some(&n), Some(&n), ControlFailure::FixtureChanged),
            (Some(&s), Some(&s), ControlFailure::CleanupUnconfirmed),
        ] {
            let result =
                evaluate_sys_interface_fixture(&spec, [Some(&n), Some(&s), Some(&w), end, cleanup]);
            assert_eq!(result.leaked, [check]);
            assert!(result.controls.contains(&reason));
            assert!(!result.complete());
        }
        let mut malformed = s.clone();
        malformed.entries.push(malformed.entries[0].clone());
        let result = evaluate_sys_interface_fixture(
            &spec,
            [Some(&n), Some(&s), Some(&w), Some(&malformed), Some(&n)],
        );
        assert_eq!(result.leaked, [check]);
        assert!(result.proven.is_empty());
        assert!(result.controls.contains(&ControlFailure::InvalidSpec));
    }
    #[test]
    fn root_only_or_missing_worker_raw_cannot_replace_same_worker_positive_visibility() {
        let (spec, mut n, mut s) = crate::native_pair::tests::fixture();
        n.sys_interfaces = Some(sample(false));
        s.sys_interfaces = Some(sample(true));
        for raw in [None, Some(&n)] {
            let result = evaluate_paired_native_fixture(&spec, [&n, &s, &n, &s, &n], raw);
            assert!(result.sys_interfaces.proven.is_empty());
            assert!(!result.sys_interfaces.complete());
            let leaked = evaluate_paired_native_fixture(&spec, [&n, &s, &s, &s, &n], raw);
            assert_eq!(leaked.sys_interfaces.leaked.len(), 6);
        }
        assert!(
            evaluate_paired_native_fixture(&spec, [&n, &s, &n, &s, &n], Some(&s))
                .sys_interfaces
                .complete()
        );
    }
}
