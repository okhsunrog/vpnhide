//! Address-vector controls. Interface enumeration alone cannot establish IPv4/
//! IPv6 or ioctl address coverage. Native RAW is an explicit root differential.
use crate::{ControlFailure, NativeFixtureSpec, NativeReadActor, ProbeError, Sample};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_ADDRESSES: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddressIoctl {
    Address,
    Netmask,
    Broadcast,
    Destination,
}
impl AddressIoctl {
    pub const ALL: [Self; 4] = [
        Self::Address,
        Self::Netmask,
        Self::Broadcast,
        Self::Destination,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddressVector {
    IoctlAddress,
    IoctlNetmask,
    IoctlBroadcast,
    IoctlDestination,
    IfconfData,
    IfconfSize,
    GetifaddrsIpv4,
    GetifaddrsIpv6,
    NetlinkIpv4,
    NetlinkIpv6,
}
impl AddressVector {
    pub const ALL: [Self; 10] = [
        Self::IoctlAddress,
        Self::IoctlNetmask,
        Self::IoctlBroadcast,
        Self::IoctlDestination,
        Self::IfconfData,
        Self::IfconfSize,
        Self::GetifaddrsIpv4,
        Self::GetifaddrsIpv6,
        Self::NetlinkIpv4,
        Self::NetlinkIpv6,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddressCheck {
    pub actor: NativeReadActor,
    pub vector: AddressVector,
}
impl AddressCheck {
    pub fn required() -> Vec<Self> {
        NativeReadActor::ALL
            .into_iter()
            .flat_map(|actor| {
                AddressVector::ALL
                    .into_iter()
                    .filter(move |v| {
                        actor == NativeReadActor::Libc
                            || !matches!(
                                v,
                                AddressVector::GetifaddrsIpv4 | AddressVector::GetifaddrsIpv6
                            )
                    })
                    .map(move |vector| Self { actor, vector })
            })
            .collect()
    }
}

/// Names and indices can be concealed independently. Exact address bytes remain
/// a leak even if another getter strips or substitutes the interface identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedAddress {
    pub name: Option<String>,
    pub index: Option<u32>,
    pub address: Vec<u8>,
    pub prefix: Option<u8>,
}
impl ObservedAddress {
    pub fn valid(&self) -> bool {
        matches!(self.address.len(), 4 | 16)
            && self
                .prefix
                .is_none_or(|p| usize::from(p) <= self.address.len() * 8)
            // An address can leak after a hook scrubbed both identity fields.
            // Only a RAW positive with the real identity establishes stimulus.
            && self.name.as_ref().is_none_or(|n| {
                !n.is_empty()
                    && n.len() <= 15
                    && n.is_ascii()
                    && !n.bytes().any(|b| b.is_ascii_control())
            })
            && self.index.is_none_or(|i| i > 0 && i <= i32::MAX as u32)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddressIoctlObservation {
    pub kind: AddressIoctl,
    pub fixture: Sample<Vec<u8>>,
    pub ordinary: Sample<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IfconfObservation {
    pub entries: Vec<ObservedAddress>,
    pub record_bytes: u32,
    pub used_bytes: u32,
    pub size_before: u32,
    pub size_after: u32,
}
impl IfconfObservation {
    pub fn bounded(&self) -> bool {
        matches!(self.record_bytes, 32 | 40)
            && self.used_bytes as usize == self.entries.len() * self.record_bytes as usize
            && self.size_before <= MAX_ADDRESSES as u32 * self.record_bytes
            && self.size_after <= MAX_ADDRESSES as u32 * self.record_bytes
            && valid_entries(&self.entries)
    }
    fn consistent(&self) -> bool {
        self.size_before == self.used_bytes && self.size_after == self.used_bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddressActorMeasurement {
    pub actor: NativeReadActor,
    pub ioctls: Vec<AddressIoctlObservation>,
    pub ifconf: Sample<IfconfObservation>,
    pub netlink: Sample<Vec<ObservedAddress>>,
}
impl AddressActorMeasurement {
    pub fn bounded(&self) -> bool {
        let mut actors = BTreeSet::new();
        self.ioctls.len() <= AddressIoctl::ALL.len()
            && self.ioctls.iter().all(|v| {
                actors.insert(v.kind)
                    && [&v.fixture, &v.ordinary]
                        .iter()
                        .all(|s| s.value().is_none_or(|b| b.len() == 4))
            })
            && self.ifconf.value().is_none_or(IfconfObservation::bounded)
            && [&self.netlink]
                .iter()
                .all(|s| s.value().is_none_or(|v| valid_entries(v)))
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddressMeasurement {
    pub actors: Vec<AddressActorMeasurement>,
    pub ifaddrs: Sample<Vec<ObservedAddress>>,
}
impl AddressMeasurement {
    pub fn bounded(&self) -> bool {
        let mut actors = BTreeSet::new();
        self.actors.len() <= NativeReadActor::ALL.len()
            && self
                .actors
                .iter()
                .all(|v| actors.insert(v.actor) && v.bounded())
            && self.ifaddrs.value().is_none_or(|v| valid_entries(v))
    }
}

fn valid_entries(entries: &[ObservedAddress]) -> bool {
    entries.len() <= MAX_ADDRESSES
        && entries.iter().all(ObservedAddress::valid)
        && entries.iter().collect::<BTreeSet<_>>().len() == entries.len()
}

/// Reserved benchmark /32 and private /128, with no prefix route. Allocation
/// must first reject any existing occurrence; this function grants no write.
pub fn fixture_addresses(session: u64) -> ([u8; 4], [u8; 16]) {
    let bytes = session.to_be_bytes();
    let ipv4 = [198, 18 | (bytes[5] & 1), bytes[6], bytes[7]];
    let mut ipv6 = [
        0xfd, 0x76, 0x70, 0x6e, 0x68, 0x69, 0x64, 0x65, 0, 0, 0, 0, 0, 0, 0, 1,
    ];
    ipv6[8..14].copy_from_slice(&bytes[2..8]);
    (ipv4, ipv6)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopedAddressResult {
    pub proven: Vec<AddressCheck>,
    pub leaked: Vec<AddressCheck>,
    pub controls: Vec<ControlFailure>,
}
impl ScopedAddressResult {
    pub fn complete(&self) -> bool {
        self.controls.is_empty()
            && self.leaked.is_empty()
            && AddressCheck::required()
                .iter()
                .all(|c| self.proven.contains(c))
    }
    fn problem(&mut self, problem: ControlFailure) {
        if !self.controls.contains(&problem) {
            self.controls.push(problem);
        }
    }
}
fn absent(sample: &Sample<Vec<u8>>) -> bool {
    matches!(sample, Sample::Error(ProbeError::Errno(19 | 6)))
}
fn marker(spec: &NativeFixtureSpec, entry: &ObservedAddress, family: usize) -> bool {
    if entry.address.len() != family {
        return false;
    }
    let (ipv4, ipv6) = fixture_addresses(spec.session);
    entry.index == Some(spec.interface.index)
        || entry.name.as_deref() == Some(&spec.interface.name)
        || entry.address.as_slice()
            == if family == 4 {
                ipv4.as_slice()
            } else {
                ipv6.as_slice()
            }
}
fn positive(spec: &NativeFixtureSpec, entries: &[ObservedAddress], family: usize) -> bool {
    let (ipv4, ipv6) = fixture_addresses(spec.session);
    let address = if family == 4 {
        ipv4.as_slice()
    } else {
        ipv6.as_slice()
    };
    entries
        .iter()
        .any(|e| e.index == Some(spec.interface.index) && e.address == address)
}
fn ordinary(entries: &[ObservedAddress], family: usize) -> BTreeSet<&ObservedAddress> {
    entries
        .iter()
        .filter(|e| e.name.as_deref() == Some("lo") && e.address.len() == family)
        .collect()
}

fn ifconf_size(
    result: &mut ScopedAddressResult,
    spec: &NativeFixtureSpec,
    actor: NativeReadActor,
    samples: [Option<&IfconfObservation>; 5],
) {
    let [Some(n), Some(s), Some(w), e, c] = samples else {
        result.problem(ControlFailure::ProbeUnavailable);
        return;
    };
    let without_fixture = |entries: &[ObservedAddress]| {
        entries
            .iter()
            .filter(|a| !marker(spec, a, 4))
            .cloned()
            .collect::<BTreeSet<_>>()
    };
    // Qualify the observed discrepancy from complete negative/start RAW lists
    // and stable worker queries. Later failures withhold proof, not that evidence.
    if ![n, s].iter().all(|v| v.consistent()) || w.size_before != w.size_after {
        result.problem(ControlFailure::ProbeUnavailable);
        return;
    }
    if n.entries.iter().any(|a| marker(spec, a, 4))
        || !positive(spec, &s.entries, 4)
        || ordinary(&n.entries, 4).is_empty()
        || without_fixture(&n.entries) != without_fixture(&s.entries)
        || ordinary(&w.entries, 4) != ordinary(&n.entries, 4)
    {
        result.problem(ControlFailure::FixtureChanged);
        return;
    }
    let visible_other =
        w.entries.iter().filter(|a| !marker(spec, a, 4)).count() as u32 * w.record_bytes;
    let leaked = w.size_before > visible_other;
    if leaked {
        result.leaked.push(AddressCheck {
            actor,
            vector: AddressVector::IfconfSize,
        });
    }
    let mut controlled = true;
    for failure in crate::later_control_failures(e, ControlFailure::FixtureChanged, |e| {
        e.consistent()
            && positive(spec, &e.entries, 4)
            && without_fixture(&n.entries) == without_fixture(&e.entries)
    }) {
        result.problem(failure);
        controlled = false;
    }
    for failure in crate::later_control_failures(c, ControlFailure::CleanupUnconfirmed, |c| {
        c.consistent()
            && !c.entries.iter().any(|a| marker(spec, a, 4))
            && without_fixture(&n.entries) == without_fixture(&c.entries)
    }) {
        result.problem(failure);
        controlled = false;
    }
    if !leaked && w.size_before == visible_other && controlled {
        result.proven.push(AddressCheck {
            actor,
            vector: AddressVector::IfconfSize,
        });
    } else if w.size_before < visible_other {
        result.problem(ControlFailure::ProbeUnavailable);
    }
}

fn lists(
    result: &mut ScopedAddressResult,
    spec: &NativeFixtureSpec,
    check: AddressCheck,
    family: usize,
    samples: [Option<&[ObservedAddress]>; 5],
) {
    let [Some(n), Some(s), Some(w), e, c] = samples else {
        result.problem(ControlFailure::ProbeUnavailable);
        return;
    };
    if n.iter().any(|a| marker(spec, a, family)) || ordinary(n, family).is_empty() {
        result.problem(ControlFailure::MissingNegativeControl);
        return;
    }
    if !positive(spec, s, family) || ordinary(s, family) != ordinary(n, family) {
        result.problem(ControlFailure::MissingPositiveControl);
        return;
    }
    let leaked = w.iter().any(|a| marker(spec, a, family));
    if leaked {
        result.leaked.push(check);
    }
    let mut controlled = true;
    if ordinary(w, family) != ordinary(s, family) {
        result.problem(ControlFailure::OrdinaryObjectsChanged);
        controlled = false;
    }
    for failure in crate::later_control_failures(e, ControlFailure::FixtureChanged, |e| {
        positive(spec, e, family) && ordinary(e, family) == ordinary(s, family)
    }) {
        result.problem(failure);
        controlled = false;
    }
    for failure in crate::later_control_failures(c, ControlFailure::CleanupUnconfirmed, |c| {
        !c.iter().any(|a| marker(spec, a, family)) && ordinary(c, family) == ordinary(n, family)
    }) {
        result.problem(failure);
        controlled = false;
    }
    if !leaked && controlled {
        result.proven.push(check);
    }
}

fn evaluate_actor(
    result: &mut ScopedAddressResult,
    spec: &NativeFixtureSpec,
    actor: NativeReadActor,
    samples: [Option<&AddressActorMeasurement>; 5],
) {
    for kind in AddressIoctl::ALL {
        let observations =
            samples.map(|m| m.and_then(|m| m.ioctls.iter().find(|o| o.kind == kind)));
        let [Some(n), Some(s), Some(w), e, c] = observations else {
            result.problem(ControlFailure::ProbeUnavailable);
            continue;
        };
        if !absent(&n.fixture) || n.ordinary.value().is_none() {
            result.problem(ControlFailure::MissingNegativeControl);
            continue;
        }
        let expected = match kind {
            AddressIoctl::Address | AddressIoctl::Destination => {
                fixture_addresses(spec.session).0.to_vec()
            }
            AddressIoctl::Netmask => vec![255; 4],
            AddressIoctl::Broadcast => vec![0; 4],
        };
        if s.fixture.value() != Some(&expected) || s.ordinary != n.ordinary {
            result.problem(ControlFailure::MissingPositiveControl);
            continue;
        }
        let check = match kind {
            AddressIoctl::Address => AddressVector::IoctlAddress,
            AddressIoctl::Netmask => AddressVector::IoctlNetmask,
            AddressIoctl::Broadcast => AddressVector::IoctlBroadcast,
            AddressIoctl::Destination => AddressVector::IoctlDestination,
        };
        let leaked = w.fixture.value().is_some();
        if leaked {
            result.leaked.push(AddressCheck {
                actor,
                vector: check,
            });
        }
        let mut controlled = true;
        if w.ordinary != s.ordinary {
            result.problem(ControlFailure::OrdinaryObjectsChanged);
            controlled = false;
        }
        for failure in crate::later_control_failures(e, ControlFailure::FixtureChanged, |e| {
            e.fixture == s.fixture && e.ordinary == s.ordinary
        }) {
            result.problem(failure);
            controlled = false;
        }
        for failure in crate::later_control_failures(c, ControlFailure::CleanupUnconfirmed, |c| {
            absent(&c.fixture) && c.ordinary == n.ordinary
        }) {
            result.problem(failure);
            controlled = false;
        }
        if !leaked && !absent(&w.fixture) {
            result.problem(ControlFailure::ProbeUnavailable);
            controlled = false;
        }
        if !leaked && controlled {
            result.proven.push(AddressCheck {
                actor,
                vector: check,
            });
        }
    }
    for (family, vector) in [
        (4, AddressVector::NetlinkIpv4),
        (16, AddressVector::NetlinkIpv6),
    ] {
        lists(
            result,
            spec,
            AddressCheck { actor, vector },
            family,
            samples.map(|m| m.and_then(|m| m.netlink.value().map(Vec::as_slice))),
        );
    }
    lists(
        result,
        spec,
        AddressCheck {
            actor,
            vector: AddressVector::IfconfData,
        },
        4,
        samples.map(|m| m.and_then(|m| m.ifconf.value().map(|v| v.entries.as_slice()))),
    );
    ifconf_size(
        result,
        spec,
        actor,
        samples.map(|m| m.and_then(|m| m.ifconf.value())),
    );
}

pub fn evaluate_address_fixture(
    spec: &NativeFixtureSpec,
    negative: &AddressMeasurement,
    start: &AddressMeasurement,
    worker: &AddressMeasurement,
    end: &AddressMeasurement,
    cleanup: &AddressMeasurement,
) -> ScopedAddressResult {
    let mut result = ScopedAddressResult::default();
    if !spec.valid() || [negative, start, worker].iter().any(|m| !m.bounded()) {
        result.problem(ControlFailure::InvalidSpec);
        return result;
    }
    let samples = [negative, start, worker, end, cleanup].map(|m| m.bounded().then_some(m));
    if samples.iter().any(Option::is_none) {
        result.problem(ControlFailure::InvalidSpec);
    }
    for actor in NativeReadActor::ALL {
        evaluate_actor(
            &mut result,
            spec,
            actor,
            samples.map(|m| m.and_then(|m| m.actors.iter().find(|v| v.actor == actor))),
        );
    }
    for (family, vector) in [
        (4, AddressVector::GetifaddrsIpv4),
        (16, AddressVector::GetifaddrsIpv6),
    ] {
        lists(
            &mut result,
            spec,
            AddressCheck {
                actor: NativeReadActor::Libc,
                vector,
            },
            family,
            samples.map(|m| m.and_then(|m| m.ifaddrs.value().map(Vec::as_slice))),
        );
    }
    result
}

/// Core's optional component projection keeps qualified first observations even
/// when a later collection could not return this component at all.
pub fn evaluate_native_address_fixture(
    spec: &NativeFixtureSpec,
    samples: [&crate::NativeMeasurement; 5],
) -> ScopedAddressResult {
    let [Some(n), Some(s), Some(w), e, c] = samples.map(|m| m.addresses.as_ref()) else {
        return ScopedAddressResult {
            controls: vec![ControlFailure::ProbeUnavailable],
            ..Default::default()
        };
    };
    let unavailable = AddressMeasurement {
        actors: vec![],
        ifaddrs: Sample::Error(ProbeError::Unsupported),
    };
    evaluate_address_fixture(
        spec,
        n,
        s,
        w,
        e.unwrap_or(&unavailable),
        c.unwrap_or(&unavailable),
    )
}
