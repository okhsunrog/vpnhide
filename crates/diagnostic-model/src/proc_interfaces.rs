//! Literal procfs interface evidence. A denied read is not hiding proof.
use crate::{
    ControlFailure, MAX_INTERFACES, NativeFixtureSpec, NativeReadActor, ObservedAddress, Sample,
    fixture_addresses,
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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopedProcInterfaceResult {
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
    fn problem(&mut self, reason: ControlFailure) {
        if !self.controls.contains(&reason) {
            self.controls.push(reason);
        }
    }
}
// Record counters can change without changing interface identity. Preserve the
// ordinary loopback identity, not arbitrary traffic-dependent byte counters.
fn controlled<T: PartialEq>(
    result: &mut ScopedProcInterfaceResult,
    check: ProcInterfaceCheck,
    samples: [Option<T>; 5],
    has_fixture: impl Fn(&T) -> bool,
    positive: impl Fn(&T) -> bool,
    ordinary: impl Fn(&T) -> bool,
) {
    let [Some(n), Some(s), Some(w), Some(e), Some(c)] = samples else {
        result.problem(ControlFailure::ProbeUnavailable);
        return;
    };
    if has_fixture(&n) || !ordinary(&n) {
        result.problem(ControlFailure::MissingNegativeControl);
        return;
    }
    if !positive(&s) || !ordinary(&s) {
        result.problem(ControlFailure::MissingPositiveControl);
        return;
    }
    let leaked = has_fixture(&w);
    if leaked {
        result.leaked.push(check);
    }
    let mut valid = true;
    if !ordinary(&w) {
        result.problem(ControlFailure::OrdinaryObjectsChanged);
        valid = false;
    }
    if !positive(&e) || !ordinary(&e) {
        result.problem(ControlFailure::FixtureChanged);
        valid = false;
    }
    if has_fixture(&c) || !ordinary(&c) {
        result.problem(ControlFailure::CleanupUnconfirmed);
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
    let mut result = ScopedProcInterfaceResult::default();
    if !spec.valid() || samples.iter().any(|m| !m.bounded()) {
        result.problem(ControlFailure::InvalidSpec);
        return result;
    }
    for actor in NativeReadActor::ALL {
        let [Some(n), Some(s), Some(w), Some(e), Some(c)] =
            samples.map(|m| m.actors.iter().find(|a| a.actor == actor))
        else {
            result.problem(ControlFailure::ProbeUnavailable);
            continue;
        };
        let measurements = [n, s, w, e, c];
        let has_name = |entries: &&Vec<String>| entries.contains(&spec.interface.name);
        controlled(
            &mut result,
            ProcInterfaceCheck {
                actor,
                surface: ProcInterfaceSurface::DeviceStats,
            },
            measurements.map(|m| m.devices.value()),
            has_name,
            has_name,
            |v| v.iter().any(|n| n == "lo"),
        );
        let address = fixture_addresses(spec.session).1;
        let marker = |v: &&Vec<ObservedAddress>| {
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
            measurements.map(|m| m.ipv6.value()),
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
