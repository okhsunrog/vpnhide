//! Literal procfs interface evidence. A denied read is not hiding proof.
use crate::{
    ControlFailure, MAX_INTERFACES, NativeFixtureSpec, NativeReadActor, ObservedAddress,
    ProcReadDenial, Sample, fixture_addresses,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcInterfaceSurface {
    DeviceStats,
    Ipv6Addresses,
}
impl ProcInterfaceSurface {
    pub const ALL: [Self; 2] = [Self::DeviceStats, Self::Ipv6Addresses];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcInterfaceCheck {
    pub actor: NativeReadActor,
    pub surface: ProcInterfaceSurface,
}
impl ProcInterfaceCheck {
    pub fn required() -> Vec<Self> {
        NativeReadActor::ALL
            .into_iter()
            .flat_map(|actor| {
                ProcInterfaceSurface::ALL
                    .into_iter()
                    .map(move |surface| Self { actor, surface })
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcInterfaceActorMeasurement {
    pub actor: NativeReadActor,
    pub devices: Sample<Vec<String>>,
    pub ipv6: Sample<Vec<ObservedAddress>>,
}
impl ProcInterfaceActorMeasurement {
    pub fn bounded(&self) -> bool {
        self.devices.value().is_none_or(|v| {
            v.len() <= MAX_INTERFACES
                && v.iter().collect::<BTreeSet<_>>().len() == v.len()
                && v.iter().all(|n| {
                    !n.is_empty()
                        && n.len() <= 15
                        && n.is_ascii()
                        && !n.bytes().any(|b| b.is_ascii_control())
                })
        }) && self.ipv6.value().is_none_or(|v| {
            v.len() <= crate::MAX_ADDRESSES
                && v.iter().collect::<BTreeSet<_>>().len() == v.len()
                && v.iter().all(|a| a.valid() && a.address.len() == 16)
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcInterfaceMeasurement {
    pub actors: Vec<ProcInterfaceActorMeasurement>,
}
impl ProcInterfaceMeasurement {
    pub fn bounded(&self) -> bool {
        let mut actors = BTreeSet::new();
        self.actors.len() <= NativeReadActor::ALL.len()
            && self
                .actors
                .iter()
                .all(|a| actors.insert(a.actor) && a.bounded())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcInterfaceAccessDenial {
    pub check: ProcInterfaceCheck,
    pub raw: ProcReadDenial,
    pub protected: ProcReadDenial,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopedProcInterfaceResult {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub access_denials: Vec<ProcInterfaceAccessDenial>,
    pub proven: Vec<ProcInterfaceCheck>,
    pub leaked: Vec<ProcInterfaceCheck>,
    pub controls: Vec<ControlFailure>,
}
impl ScopedProcInterfaceResult {
    pub fn complete(&self) -> bool {
        self.controls.is_empty()
            && self.leaked.is_empty()
            && ProcInterfaceCheck::required()
                .iter()
                .all(|c| self.proven.contains(c))
    }
    pub(crate) fn all_access_denied(&self) -> bool {
        ProcInterfaceCheck::required().iter().all(|check| {
            self.access_denials
                .iter()
                .any(|denial| denial.check == *check)
        })
    }

    fn problem(&mut self, reason: ControlFailure) {
        if !self.controls.contains(&reason) {
            self.controls.push(reason);
        }
    }
}
// Record counters can change without changing interface identity. Preserve the
// ordinary loopback identity, not arbitrary traffic-dependent byte counters.
fn controlled<T>(
    result: &mut ScopedProcInterfaceResult,
    check: ProcInterfaceCheck,
    samples: [Option<&Sample<T>>; 5],
    raw: Option<&Sample<T>>,
    has_fixture: impl Fn(&T) -> bool,
    positive: impl Fn(&T) -> bool,
    ordinary: impl Fn(&T) -> bool,
) {
    fn denial<T>(sample: Option<&Sample<T>>) -> Option<ProcReadDenial> {
        match sample? {
            Sample::Error(error) => error.proc_read_denial(),
            Sample::Value(_) => None,
        }
    }
    let [n, s, w, e, c] = samples.map(|sample| sample.and_then(Sample::value));
    let absent = |sample: &T| !has_fixture(sample) && ordinary(sample);
    let present = |sample: &T| positive(sample) && ordinary(sample);
    if let (Some(raw), Some(protected)) = (denial(raw), denial(samples[2]))
        && n.is_some_and(absent)
        && s.is_some_and(present)
        && e.is_some_and(present)
        && c.is_some_and(absent)
    {
        result.access_denials.push(ProcInterfaceAccessDenial {
            check,
            raw,
            protected,
        });
        return;
    }
    let [Some(n), Some(s), Some(w), e, c] = [n, s, w, e, c] else {
        result.problem(ControlFailure::ProbeUnavailable);
        return;
    };
    if !absent(n) {
        result.problem(ControlFailure::MissingNegativeControl);
        return;
    }
    if !present(s) {
        result.problem(ControlFailure::MissingPositiveControl);
        return;
    }
    let leaked = has_fixture(w);
    if leaked {
        result.leaked.push(check);
    }
    let mut valid = true;
    if !ordinary(w) {
        result.problem(ControlFailure::OrdinaryObjectsChanged);
        valid = false;
    }
    for failure in
        crate::later_control_failures(e, ControlFailure::FixtureChanged, |sample| present(sample))
    {
        result.problem(failure);
        valid = false;
    }
    for failure in crate::later_control_failures(c, ControlFailure::CleanupUnconfirmed, |sample| {
        absent(sample)
    }) {
        result.problem(failure);
        valid = false;
    }
    if valid && !leaked {
        result.proven.push(check);
    }
}

pub fn evaluate_proc_interface_fixture(
    spec: &NativeFixtureSpec,
    samples: [&ProcInterfaceMeasurement; 5],
) -> ScopedProcInterfaceResult {
    evaluate_proc_interface_controls(spec, samples, None)
}

fn evaluate_proc_interface_controls(
    spec: &NativeFixtureSpec,
    samples: [&ProcInterfaceMeasurement; 5],
    app_raw: Option<&ProcInterfaceMeasurement>,
) -> ScopedProcInterfaceResult {
    let mut result = ScopedProcInterfaceResult::default();
    if !spec.valid() || samples[..3].iter().any(|m| !m.bounded()) {
        result.problem(ControlFailure::InvalidSpec);
        return result;
    }
    let samples = samples.map(|m| m.bounded().then_some(m));
    if samples.iter().any(Option::is_none) {
        result.problem(ControlFailure::InvalidSpec);
    }
    for actor in NativeReadActor::ALL {
        let measurements =
            samples.map(|m| m.and_then(|m| m.actors.iter().find(|a| a.actor == actor)));
        let raw = app_raw
            .filter(|value| value.bounded())
            .and_then(|value| value.actors.iter().find(|value| value.actor == actor));
        let has_name = |entries: &Vec<String>| entries.contains(&spec.interface.name);
        controlled(
            &mut result,
            ProcInterfaceCheck {
                actor,
                surface: ProcInterfaceSurface::DeviceStats,
            },
            measurements.map(|m| m.map(|m| &m.devices)),
            raw.map(|m| &m.devices),
            has_name,
            has_name,
            |v| v.iter().any(|n| n == "lo"),
        );
        let address = fixture_addresses(spec.session).1;
        let marker = |v: &Vec<ObservedAddress>| {
            v.iter().any(|a| {
                a.address == address
                    || a.name.as_deref() == Some(&spec.interface.name)
                    || a.index == Some(spec.interface.index)
            })
        };
        controlled(
            &mut result,
            ProcInterfaceCheck {
                actor,
                surface: ProcInterfaceSurface::Ipv6Addresses,
            },
            measurements.map(|m| m.map(|m| &m.ipv6)),
            raw.map(|m| &m.ipv6),
            marker,
            |v| {
                v.iter().any(|a| {
                    a.address == address
                        && a.index == Some(spec.interface.index)
                        && a.name.as_deref() == Some(&spec.interface.name)
                })
            },
            |v| {
                v.iter().any(|a| {
                    a.address == [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]
                        && a.index == Some(spec.ordinary.interface.index)
                        && a.name.as_deref() == Some("lo")
                        && a.prefix == Some(128)
                })
            },
        );
    }
    result
}

pub fn evaluate_native_proc_interface_fixture(
    spec: &NativeFixtureSpec,
    samples: [&crate::NativeMeasurement; 5],
) -> ScopedProcInterfaceResult {
    evaluate_native_proc_interface_controls(spec, samples, None)
}

pub(crate) fn evaluate_native_proc_interface_controls(
    spec: &NativeFixtureSpec,
    samples: [&crate::NativeMeasurement; 5],
    app_raw: Option<&crate::NativeMeasurement>,
) -> ScopedProcInterfaceResult {
    let [Some(n), Some(s), Some(w), e, c] = samples.map(|m| m.proc_interfaces.as_ref()) else {
        return ScopedProcInterfaceResult {
            controls: vec![ControlFailure::ProbeUnavailable],
            ..Default::default()
        };
    };
    let unavailable = ProcInterfaceMeasurement { actors: vec![] };
    evaluate_proc_interface_controls(
        spec,
        [
            n,
            s,
            w,
            e.unwrap_or(&unavailable),
            c.unwrap_or(&unavailable),
        ],
        app_raw.and_then(|m| m.proc_interfaces.as_ref()),
    )
}
