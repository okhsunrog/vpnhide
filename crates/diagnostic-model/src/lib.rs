//! Unprivileged diagnostic contracts and evidence evaluation. No fixture writes,
//! Android/root orchestration or inference from a successfully-installed mask.

use serde::{Deserialize, Serialize};

pub mod framework;
mod run;
pub use run::*;

pub const MAX_INTERFACES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ProbeError {
    Errno(i32),
    Timeout,
    Malformed,
    TooLarge,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Sample<T> {
    Value(T),
    Error(ProbeError),
}

impl<T> Sample<T> {
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Value(value) => Some(value),
            Self::Error(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterfaceId {
    pub name: String,
    pub index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedInterface {
    pub name: String,
    pub index: Option<u32>,
    pub ifaddrs: Option<IfaddrMetadata>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IfaddrMetadata {
    pub families: Vec<u16>,
    pub flags: Vec<u32>,
    pub has_null_address: bool,
}

impl ObservedInterface {
    pub fn matches(&self, id: &InterfaceId) -> bool {
        self.name == id.name && self.index == Some(id.index)
    }
}
impl From<InterfaceId> for ObservedInterface {
    fn from(id: InterfaceId) -> Self {
        Self {
            name: id.name,
            index: Some(id.index),
            ifaddrs: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrdinaryControl {
    pub interface: InterfaceId,
    pub flags: u32,
    pub mtu: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeFixtureSpec {
    pub session: u64,
    pub interface: InterfaceId,
    pub flags: u32,
    pub mtu: u32,
    pub ordinary: OrdinaryControl,
}

impl NativeFixtureSpec {
    pub fn valid(&self) -> bool {
        self.session != 0
            && self.interface.index > 0
            && self.interface.index <= i32::MAX as u32
            && self.interface.name.len() == 15
            && self.interface.name.starts_with("tun")
            && self.interface.name[3..]
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            && self.flags & 1 == 0
            && self.mtu > 0
            && self.mtu <= i32::MAX as u32
            && self.ordinary.interface.name == "lo"
            && self.ordinary.interface.index > 0
            && self.ordinary.interface.index != self.interface.index
            && self.ordinary.mtu > 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeMeasurement {
    pub flags: Sample<u32>,
    pub mtu: Sample<u32>,
    pub ifaddrs: Sample<Vec<ObservedInterface>>,
    pub links: Sample<Vec<ObservedInterface>>,
    pub ordinary_flags: Sample<u32>,
    pub ordinary_mtu: Sample<u32>,
}

impl NativeMeasurement {
    pub fn bounded(&self) -> bool {
        [&self.ifaddrs, &self.links].into_iter().all(|sample| {
            sample.value().is_none_or(|entries| {
                entries.len() <= MAX_INTERFACES
                    && coherent(entries)
                    && entries.iter().all(|e| {
                        !e.name.is_empty()
                            && e.name.len() <= 15
                            && e.name.is_ascii()
                            && !e.name.bytes().any(|b| b.is_ascii_control())
                            && e.index.is_none_or(|i| i > 0 && i <= i32::MAX as u32)
                            && e.ifaddrs
                                .as_ref()
                                .is_none_or(|m| m.families.len() <= 32 && m.flags.len() <= 8)
                    })
            })
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeCheck {
    IoctlFlags,
    IoctlMtu,
    Getifaddrs,
    NetlinkGetlink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlFailure {
    InvalidSpec,
    MissingNegativeControl,
    MissingPositiveControl,
    FixtureChanged,
    OrdinaryObjectsChanged,
    ProbeUnavailable,
    CleanupUnconfirmed,
}

/// A scoped interface-only result. This cannot establish the whole required
/// native/framework plan or refresh older game-process hook generations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScopedNativeResult {
    Proven {
        checks: Vec<NativeCheck>,
    },
    Leaked {
        checks: Vec<NativeCheck>,
        incomplete: Vec<ControlFailure>,
    },
    Unverified {
        reason: ControlFailure,
        observed_leaks: Vec<NativeCheck>,
    },
}

fn absent_value(value: &Sample<u32>) -> bool {
    // ENODEV / ENXIO mimic an absent interface. Permission failures are not
    // successful hiding and arbitrary invalid-argument errors are not controls.
    matches!(value, Sample::Error(ProbeError::Errno(19 | 6)))
}

fn absent_lists(spec: &NativeFixtureSpec, observation: &NativeMeasurement) -> bool {
    [&observation.ifaddrs, &observation.links]
        .into_iter()
        .all(|sample| {
            sample.value().is_some_and(|entries| {
                entries.iter().all(|entry| {
                    entry.name != spec.interface.name && entry.index != Some(spec.interface.index)
                })
            })
        })
}

fn ordinary_preserved(spec: &NativeFixtureSpec, observation: &NativeMeasurement) -> bool {
    observation.ordinary_flags.value() == Some(&spec.ordinary.flags)
        && observation.ordinary_mtu.value() == Some(&spec.ordinary.mtu)
        && [&observation.ifaddrs, &observation.links]
            .into_iter()
            .all(|sample| {
                sample.value().is_some_and(|entries| {
                    entries.iter().any(|e| e.matches(&spec.ordinary.interface))
                })
            })
}

fn positive(spec: &NativeFixtureSpec, observation: &NativeMeasurement) -> bool {
    observation.flags.value() == Some(&spec.flags)
        && observation.mtu.value() == Some(&spec.mtu)
        && [&observation.ifaddrs, &observation.links]
            .into_iter()
            .all(|sample| {
                sample
                    .value()
                    .is_some_and(|entries| entries.iter().any(|e| e.matches(&spec.interface)))
            })
        && ordinary_preserved(spec, observation)
}

pub fn cleanup_confirmed(spec: &NativeFixtureSpec, observation: &NativeMeasurement) -> bool {
    spec.valid()
        && observation.bounded()
        && absent_value(&observation.flags)
        && absent_value(&observation.mtu)
        && absent_lists(spec, observation)
        && ordinary_preserved(spec, observation)
}

fn coherent(entries: &[ObservedInterface]) -> bool {
    let mut names = std::collections::BTreeSet::new();
    let mut indices = std::collections::BTreeSet::new();
    entries
        .iter()
        .all(|e| names.insert(e.name.as_str()) && e.index.is_none_or(|i| indices.insert(i)))
}

pub fn evaluate_native_fixture(
    spec: &NativeFixtureSpec,
    negative: &NativeMeasurement,
    root_start: &NativeMeasurement,
    worker: &NativeMeasurement,
    root_end: &NativeMeasurement,
    cleanup: &NativeMeasurement,
) -> ScopedNativeResult {
    let unknown = |reason| ScopedNativeResult::Unverified {
        reason,
        observed_leaks: Vec::new(),
    };
    if !spec.valid()
        || [negative, root_start, worker, root_end, cleanup]
            .into_iter()
            .any(|o| !o.bounded())
    {
        return unknown(ControlFailure::InvalidSpec);
    }
    if !absent_value(&negative.flags)
        || !absent_value(&negative.mtu)
        || !absent_lists(spec, negative)
        || !ordinary_preserved(spec, negative)
    {
        return unknown(ControlFailure::MissingNegativeControl);
    }
    if !positive(spec, root_start) {
        return unknown(ControlFailure::MissingPositiveControl);
    }
    let mut incomplete = Vec::new();
    let mut problem = |reason| {
        if !incomplete.contains(&reason) {
            incomplete.push(reason);
        }
    };
    if !cleanup_confirmed(spec, cleanup) {
        problem(ControlFailure::CleanupUnconfirmed);
    }
    if worker.ordinary_flags.value().is_none()
        || worker.ordinary_mtu.value().is_none()
        || worker.ifaddrs.value().is_none()
        || worker.links.value().is_none()
    {
        problem(ControlFailure::ProbeUnavailable);
    } else if !ordinary_preserved(spec, worker) {
        problem(ControlFailure::OrdinaryObjectsChanged);
    }
    let mut leaked = Vec::new();
    for (check, sample) in [
        (NativeCheck::IoctlFlags, &worker.flags),
        (NativeCheck::IoctlMtu, &worker.mtu),
    ] {
        if !absent_value(sample) {
            if sample.value().is_some() {
                leaked.push(check);
            } else {
                problem(ControlFailure::ProbeUnavailable);
            }
        }
    }
    for (check, sample) in [
        (NativeCheck::Getifaddrs, &worker.ifaddrs),
        (NativeCheck::NetlinkGetlink, &worker.links),
    ] {
        if let Some(entries) = sample.value() {
            if entries
                .iter()
                .any(|e| e.name == spec.interface.name || e.index == Some(spec.interface.index))
            {
                leaked.push(check);
            }
            if entries.iter().any(|e| e.index.is_none()) {
                problem(ControlFailure::ProbeUnavailable);
            }
        } else {
            problem(ControlFailure::ProbeUnavailable);
        }
    }
    if !positive(spec, root_end) {
        return ScopedNativeResult::Unverified {
            reason: ControlFailure::FixtureChanged,
            observed_leaks: leaked,
        };
    }
    if !leaked.is_empty() {
        return ScopedNativeResult::Leaked {
            checks: leaked,
            incomplete,
        };
    }
    if let Some(reason) = incomplete.first() {
        return unknown(*reason);
    }
    ScopedNativeResult::Proven {
        checks: vec![
            NativeCheck::IoctlFlags,
            NativeCheck::IoctlMtu,
            NativeCheck::Getifaddrs,
            NativeCheck::NetlinkGetlink,
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn spec() -> NativeFixtureSpec {
        NativeFixtureSpec {
            session: 1,
            interface: InterfaceId {
                name: "tun000000000001".into(),
                index: 42,
            },
            flags: 16,
            mtu: 1543,
            ordinary: OrdinaryControl {
                interface: InterfaceId {
                    name: "lo".into(),
                    index: 1,
                },
                flags: 73,
                mtu: 65536,
            },
        }
    }
    fn measure(visible: bool) -> NativeMeasurement {
        let spec = spec();
        let mut entries = vec![
            spec.ordinary.interface.clone().into(),
            ObservedInterface {
                name: "tun0".into(),
                index: Some(9),
                ifaddrs: None,
            },
        ];
        if visible {
            entries.push(spec.interface.into());
        }
        NativeMeasurement {
            flags: if visible {
                Sample::Value(spec.flags)
            } else {
                Sample::Error(ProbeError::Errno(19))
            },
            mtu: if visible {
                Sample::Value(spec.mtu)
            } else {
                Sample::Error(ProbeError::Errno(19))
            },
            ifaddrs: Sample::Value(entries.clone()),
            links: Sample::Value(entries),
            ordinary_flags: Sample::Value(73),
            ordinary_mtu: Sample::Value(65536),
        }
    }
    #[test]
    fn scoped_proof_requires_its_real_controls_and_preserves_unrelated_objects() {
        let (spec, absent, positive) = (spec(), measure(false), measure(true));
        assert!(
            matches!(evaluate_native_fixture(&spec, &absent, &positive, &absent, &positive, &absent), ScopedNativeResult::Proven { checks } if checks.len() == 4)
        );
        assert_eq!(
            evaluate_native_fixture(&spec, &absent, &absent, &absent, &absent, &absent),
            ScopedNativeResult::Unverified {
                observed_leaks: Vec::new(),
                reason: ControlFailure::MissingPositiveControl
            }
        );
        assert_eq!(
            evaluate_native_fixture(&spec, &positive, &positive, &absent, &positive, &absent),
            ScopedNativeResult::Unverified {
                observed_leaks: Vec::new(),
                reason: ControlFailure::MissingNegativeControl
            }
        );
        assert_eq!(
            evaluate_native_fixture(&spec, &absent, &positive, &absent, &positive, &positive),
            ScopedNativeResult::Unverified {
                observed_leaks: Vec::new(),
                reason: ControlFailure::CleanupUnconfirmed
            }
        );
    }
    #[test]
    fn leaks_permissions_fixture_changes_and_broken_ordinary_views_are_distinct() {
        let (spec, absent, positive) = (spec(), measure(false), measure(true));
        assert!(
            matches!(evaluate_native_fixture(&spec, &absent, &positive, &positive, &positive, &absent), ScopedNativeResult::Leaked { checks, .. } if checks.len() == 4)
        );
        let mut denied = absent.clone();
        denied.flags = Sample::Error(ProbeError::Errno(13));
        assert_eq!(
            evaluate_native_fixture(&spec, &absent, &positive, &denied, &positive, &absent),
            ScopedNativeResult::Unverified {
                observed_leaks: Vec::new(),
                reason: ControlFailure::ProbeUnavailable
            }
        );
        let mut broken = absent.clone();
        broken.ordinary_mtu = Sample::Value(0);
        assert_eq!(
            evaluate_native_fixture(&spec, &absent, &positive, &broken, &positive, &absent),
            ScopedNativeResult::Unverified {
                observed_leaks: Vec::new(),
                reason: ControlFailure::OrdinaryObjectsChanged
            }
        );
        let mut changed = positive.clone();
        changed.mtu = Sample::Value(1500);
        assert_eq!(
            evaluate_native_fixture(&spec, &absent, &positive, &absent, &changed, &absent),
            ScopedNativeResult::Unverified {
                observed_leaks: Vec::new(),
                reason: ControlFailure::FixtureChanged
            }
        );
    }

    #[test]
    fn confirmed_leaks_survive_later_probe_and_cleanup_failures() {
        let (spec, absent, positive) = (spec(), measure(false), measure(true));
        let mut partial = absent.clone();
        partial.flags = Sample::Value(16);
        partial.mtu = Sample::Error(ProbeError::Errno(13));
        assert_eq!(
            evaluate_native_fixture(&spec, &absent, &positive, &partial, &positive, &absent),
            ScopedNativeResult::Leaked {
                checks: vec![NativeCheck::IoctlFlags],
                incomplete: vec![ControlFailure::ProbeUnavailable]
            }
        );
        assert!(
            matches!(evaluate_native_fixture(&spec, &absent, &positive, &positive, &positive, &positive), ScopedNativeResult::Leaked { checks, incomplete } if checks.len() == 4 && incomplete == vec![ControlFailure::CleanupUnconfirmed])
        );
    }

    #[test]
    fn malformed_identity_lists_and_unknown_indices_never_produce_proof() {
        let (spec, absent, positive) = (spec(), measure(false), measure(true));
        for index in [0, u32::MAX] {
            let mut bad = absent.clone();
            if let Sample::Value(entries) = &mut bad.links {
                entries.push(ObservedInterface {
                    name: "eth0".into(),
                    index: Some(index),
                    ifaddrs: None,
                });
            }
            assert!(!bad.bounded());
            assert!(!matches!(
                evaluate_native_fixture(&spec, &absent, &positive, &bad, &positive, &absent),
                ScopedNativeResult::Proven { .. }
            ));
        }
        let mut duplicate = absent.clone();
        if let Sample::Value(entries) = &mut duplicate.links {
            entries.push(spec.ordinary.interface.clone().into());
        }
        assert!(!duplicate.bounded());
        let mut unknown = absent.clone();
        if let Sample::Value(entries) = &mut unknown.ifaddrs {
            entries.push(ObservedInterface {
                name: "eth0".into(),
                index: None,
                ifaddrs: None,
            });
        }
        assert_eq!(
            evaluate_native_fixture(&spec, &absent, &positive, &unknown, &positive, &absent),
            ScopedNativeResult::Unverified {
                reason: ControlFailure::ProbeUnavailable,
                observed_leaks: vec![]
            }
        );
    }
}
