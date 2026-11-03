//! Literal route observations. These types alone do not qualify a fixture or
//! complete NetlinkRoutes/ProcRoutes; production retained-run integration follows
//! positive owned stimuli and independent cleanup/ordinary controls.
use crate::{NativeReadActor, ProbeError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_ROUTES: usize = 2048;
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteSurface {
    NetlinkIpv4,
    NetlinkIpv6,
    ProcIpv4,
    ProcIpv6,
}
impl RouteSurface {
    pub const ALL: [Self; 4] = [
        Self::NetlinkIpv4,
        Self::NetlinkIpv6,
        Self::ProcIpv4,
        Self::ProcIpv6,
    ];
    pub fn address_bytes(self) -> usize {
        match self {
            Self::NetlinkIpv4 | Self::ProcIpv4 => 4,
            Self::NetlinkIpv6 | Self::ProcIpv6 => 16,
        }
    }
    pub fn netlink(self) -> bool {
        matches!(self, Self::NetlinkIpv4 | Self::NetlinkIpv6)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteCheck {
    pub actor: NativeReadActor,
    pub surface: RouteSurface,
}
impl RouteCheck {
    pub fn required() -> Vec<Self> {
        NativeReadActor::ALL
            .into_iter()
            .flat_map(|actor| {
                RouteSurface::ALL
                    .into_iter()
                    .map(move |surface| Self { actor, surface })
            })
            .collect()
    }
}
/// Optional fields remain unknown when the source does not expose them. In
/// particular proc names are not resolved through a potentially hooked index API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedRoute {
    pub destination: Vec<u8>,
    pub prefix: u8,
    pub source: Option<Vec<u8>>,
    pub source_prefix: u8,
    pub output_index: Option<u32>,
    pub output_name: Option<String>,
    pub table: Option<u32>,
    pub protocol: Option<u8>,
    pub scope: Option<u8>,
    pub kind: Option<u8>,
    pub metric: u32,
    pub gateway: Option<Vec<u8>>,
    pub flags: u32,
}
impl ObservedRoute {
    pub fn valid(&self, surface: RouteSurface) -> bool {
        let length = surface.address_bytes();
        self.destination.len() == length
            && usize::from(self.prefix) <= length * 8
            && self.source.as_ref().is_none_or(|v| v.len() == length)
            && usize::from(self.source_prefix) <= length * 8
            && (self.source_prefix == 0 || self.source.is_some())
            && self.gateway.as_ref().is_none_or(|v| v.len() == length)
            && self
                .output_index
                .is_none_or(|i| i > 0 && i <= i32::MAX as u32)
            && self.output_name.as_ref().is_none_or(|s| {
                !s.is_empty()
                    && s.len() <= 15
                    && s.is_ascii()
                    && !s
                        .bytes()
                        .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
            })
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteObservation {
    pub check: RouteCheck,
    /// Valid rows already read survive a later malformed/truncated/failed read.
    pub routes: Vec<ObservedRoute>,
    pub error: Option<ProbeError>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteMeasurement {
    pub observations: Vec<RouteObservation>,
}
impl RouteMeasurement {
    pub fn bounded(&self) -> bool {
        let mut seen = BTreeSet::new();
        self.observations.len() <= RouteCheck::required().len()
            && self.observations.iter().all(|v| {
                seen.insert(v.check)
                    && v.routes.len() <= MAX_ROUTES
                    && v.routes.iter().all(|r| r.valid(v.check.surface))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row() -> ObservedRoute {
        ObservedRoute {
            destination: vec![198, 18, 2, 1],
            prefix: 32,
            source: None,
            source_prefix: 0,
            output_index: Some(7),
            output_name: None,
            table: Some(254),
            protocol: Some(242),
            scope: Some(253),
            kind: Some(1),
            metric: 54321,
            gateway: None,
            flags: 0,
        }
    }
    #[test]
    fn partial_rows_remain_bounded_but_duplicate_checks_or_wrong_families_do_not() {
        let observation = RouteObservation {
            check: RouteCheck {
                actor: NativeReadActor::Libc,
                surface: RouteSurface::NetlinkIpv4,
            },
            routes: vec![row()],
            error: Some(ProbeError::Timeout),
        };
        let mut value = RouteMeasurement {
            observations: vec![observation.clone()],
        };
        assert!(value.bounded());
        value.observations.push(observation);
        assert!(!value.bounded());
        value.observations.pop();
        value.observations[0].check.surface = RouteSurface::NetlinkIpv6;
        assert!(!value.bounded());
        assert_eq!(RouteCheck::required().len(), 8);
    }
}

/// Separate route stimulus identity; the existing interface fixture remains DOWN.
/// The two exact-address routes use independent VPN and ordinary interface owners.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteFixtureSpec {
    pub session: u64,
    pub vpn: crate::InterfaceId,
    pub ordinary: crate::InterfaceId,
}
impl RouteFixtureSpec {
    pub fn valid(&self) -> bool {
        self.session != 0
            && self.role_session(false) != 0
            && self.role_session(true) != 0
            && self.vpn.index > 0
            && self.ordinary.index > 0
            && self.vpn.index <= i32::MAX as u32
            && self.ordinary.index <= i32::MAX as u32
            && self.vpn.index != self.ordinary.index
            && self.vpn.name == format!("tun{:012x}", self.role_session(false) & 0xffff_ffff_ffff)
            && self.ordinary.name
                == format!("vhr{:012x}", self.role_session(true) & 0xffff_ffff_ffff)
    }
    pub fn role_session(&self, ordinary: bool) -> u64 {
        self.session ^ if ordinary { 0x8000 } else { 0x4000 }
    }
    pub fn negative(&self, measurement: &RouteMeasurement) -> bool {
        measurement.bounded()
            && RouteCheck::required().into_iter().all(|check| {
                observation(Some(measurement), check).is_some_and(|v| {
                    v.error.is_none()
                        && !v.routes.iter().any(|r| {
                            self.exposes(r, false, check.surface)
                                || self.exposes(r, true, check.surface)
                        })
                })
            })
    }
    pub fn positive_measurement(&self, measurement: &RouteMeasurement) -> bool {
        self.valid()
            && measurement.bounded()
            && RouteCheck::required().into_iter().all(|check| {
                observation(Some(measurement), check).is_some_and(|v| {
                    v.error.is_none()
                        && [false, true].into_iter().all(|ordinary| {
                            v.routes
                                .iter()
                                .any(|r| self.positive(r, ordinary, check.surface))
                        })
                })
            })
    }
    pub fn destination(&self, ordinary: bool, surface: RouteSurface) -> Vec<u8> {
        let (v4, v6) = crate::fixture_addresses(self.role_session(ordinary));
        if surface.address_bytes() == 4 {
            v4.to_vec()
        } else {
            v6.to_vec()
        }
    }
    pub fn metric(ordinary: bool) -> u32 {
        if ordinary { 54322 } else { 54321 }
    }
    fn exposes(&self, row: &ObservedRoute, ordinary: bool, surface: RouteSurface) -> bool {
        let interface = if ordinary { &self.ordinary } else { &self.vpn };
        row.destination == self.destination(ordinary, surface)
            || row.output_index == Some(interface.index)
            || row.output_name.as_deref() == Some(interface.name.as_str())
    }
    fn positive(&self, row: &ObservedRoute, ordinary: bool, surface: RouteSurface) -> bool {
        let interface = if ordinary { &self.ordinary } else { &self.vpn };
        row.destination == self.destination(ordinary, surface)
            && row.source_prefix == 0
            && usize::from(row.prefix) == surface.address_bytes() * 8
            && row.metric == Self::metric(ordinary)
            && if surface.netlink() {
                row.output_index == Some(interface.index)
                    && row.table == Some(254)
                    && row.protocol == Some(242)
                    && row.kind == Some(1)
            } else {
                row.output_name.as_deref() == Some(interface.name.as_str())
            }
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopedRouteResult {
    pub proven: Vec<RouteCheck>,
    pub leaked: Vec<RouteCheck>,
    pub controls: Vec<crate::ControlFailure>,
}
impl ScopedRouteResult {
    pub fn complete(&self) -> bool {
        self.controls.is_empty()
            && self.leaked.is_empty()
            && RouteCheck::required()
                .iter()
                .all(|c| self.proven.contains(c))
    }
    fn problem(&mut self, problem: crate::ControlFailure) {
        if !self.controls.contains(&problem) {
            self.controls.push(problem);
        }
    }
}
fn observation(
    measurement: Option<&RouteMeasurement>,
    check: RouteCheck,
) -> Option<&RouteObservation> {
    measurement
        .filter(|v| v.bounded())?
        .observations
        .iter()
        .find(|v| v.check == check)
}
/// Root controls qualify each vector before a worker row can establish a leak.
/// Later worker/ordinary/end/cleanup failures preserve that leak and withhold proof.
/// `interfaces_clean` is Root's independent name/index cleanup, never app input.
pub fn evaluate_route_fixture(
    spec: &RouteFixtureSpec,
    measurements: [Option<&RouteMeasurement>; 5],
    interfaces_clean: bool,
) -> ScopedRouteResult {
    use crate::ControlFailure::*;
    let mut result = ScopedRouteResult::default();
    if !spec.valid() {
        result.problem(InvalidSpec);
        return result;
    }
    for check in RouteCheck::required() {
        let [negative, start, worker, end, cleanup] = measurements.map(|m| observation(m, check));
        let absent = |v: &RouteObservation| {
            v.error.is_none()
                && !v.routes.iter().any(|r| {
                    spec.exposes(r, false, check.surface) || spec.exposes(r, true, check.surface)
                })
        };
        let positive = |v: &RouteObservation| {
            v.error.is_none()
                && [false, true].into_iter().all(|ordinary| {
                    v.routes
                        .iter()
                        .any(|r| spec.positive(r, ordinary, check.surface))
                })
        };
        if negative.is_none_or(|v| !absent(v)) {
            result.problem(MissingNegativeControl);
            if negative.is_none_or(|v| v.error.is_some()) {
                result.problem(ProbeUnavailable);
            }
            continue;
        }
        if start.is_none_or(|v| !positive(v)) {
            result.problem(MissingPositiveControl);
            if start.is_none_or(|v| v.error.is_some()) {
                result.problem(ProbeUnavailable);
            }
            continue;
        }
        let leaked = worker.is_some_and(|v| {
            v.routes
                .iter()
                .any(|r| spec.exposes(r, false, check.surface))
        });
        if leaked {
            result.leaked.push(check);
        }
        let worker_available = worker.is_some_and(|v| v.error.is_none());
        if !worker_available {
            result.problem(ProbeUnavailable);
        }
        let ordinary_ok = worker.is_some_and(|v| {
            v.routes
                .iter()
                .any(|r| spec.positive(r, true, check.surface))
        });
        if !ordinary_ok {
            result.problem(OrdinaryObjectsChanged);
        }
        let end_ok = end.is_some_and(positive);
        if !end_ok {
            result.problem(FixtureChanged);
            if end.is_none_or(|v| v.error.is_some()) {
                result.problem(ProbeUnavailable);
            }
        }
        let cleaned = interfaces_clean && cleanup.is_some_and(absent);
        if !cleaned {
            result.problem(CleanupUnconfirmed);
            if cleanup.is_none_or(|v| v.error.is_some()) {
                result.problem(ProbeUnavailable);
            }
        }
        if !leaked && worker_available && ordinary_ok && end_ok && cleaned {
            result.proven.push(check);
        }
    }
    result
}
/// Protected proof also needs positive visibility of both objects from this
/// worker's authenticated RAW interval. Root alone cannot supply that baseline.
pub fn evaluate_paired_route_fixture(
    spec: &RouteFixtureSpec,
    measurements: [Option<&RouteMeasurement>; 5],
    raw: Option<&RouteMeasurement>,
    interfaces_clean: bool,
) -> ScopedRouteResult {
    let mut result = evaluate_route_fixture(spec, measurements, interfaces_clean);
    let qualified: Vec<_> = RouteCheck::required()
        .into_iter()
        .filter(|check| {
            observation(raw, *check).is_some_and(|v| {
                v.error.is_none()
                    && [false, true].into_iter().all(|ordinary| {
                        v.routes
                            .iter()
                            .any(|r| spec.positive(r, ordinary, check.surface))
                    })
            })
        })
        .collect();
    result.proven.retain(|c| qualified.contains(c));
    if qualified.len() != RouteCheck::required().len() {
        result.problem(crate::ControlFailure::MissingPositiveControl);
    }

    result
}

#[cfg(test)]
mod fixture_tests {
    use super::*;
    fn spec() -> RouteFixtureSpec {
        RouteFixtureSpec {
            session: 1,
            vpn: crate::InterfaceId {
                name: "tun000000004001".into(),
                index: 7,
            },
            ordinary: crate::InterfaceId {
                name: "vhr000000008001".into(),
                index: 8,
            },
        }
    }
    fn rows(mode: u8) -> RouteMeasurement {
        RouteMeasurement {
            observations: RouteCheck::required()
                .into_iter()
                .map(|check| {
                    let routes = if mode == 0 {
                        Vec::new()
                    } else {
                        [false, true]
                            .into_iter()
                            .filter(|ordinary| mode == 1 || *ordinary)
                            .map(|ordinary| ObservedRoute {
                                destination: if check.surface.address_bytes() == 4 {
                                    vec![198, 18, if ordinary { 128 } else { 64 }, 1]
                                } else {
                                    vec![
                                        0xfd,
                                        0x76,
                                        0x70,
                                        0x6e,
                                        0x68,
                                        0x69,
                                        0x64,
                                        0x65,
                                        0,
                                        0,
                                        0,
                                        0,
                                        if ordinary { 128 } else { 64 },
                                        1,
                                        0,
                                        1,
                                    ]
                                },
                                prefix: if check.surface.address_bytes() == 4 {
                                    32
                                } else {
                                    128
                                },
                                source: None,
                                source_prefix: 0,
                                output_index: check.surface.netlink().then_some(if ordinary {
                                    8
                                } else {
                                    7
                                }),
                                output_name: (!check.surface.netlink()).then(|| {
                                    if ordinary {
                                        "vhr000000008001"
                                    } else {
                                        "tun000000004001"
                                    }
                                    .into()
                                }),
                                table: check.surface.netlink().then_some(254),
                                protocol: check.surface.netlink().then_some(242),
                                scope: check.surface.netlink().then_some(253),
                                kind: check.surface.netlink().then_some(1),
                                metric: if ordinary { 54322 } else { 54321 },
                                gateway: None,
                                flags: 0,
                            })
                            .collect()
                    };
                    RouteObservation {
                        check,
                        routes,
                        error: None,
                    }
                })
                .collect(),
        }
    }
    #[test]
    fn all_actors_families_and_sources_need_actual_app_raw_and_closed_interfaces() {
        let (negative, start, worker) = (rows(0), rows(1), rows(2));
        let values = [
            Some(&negative),
            Some(&start),
            Some(&worker),
            Some(&start),
            Some(&negative),
        ];
        assert!(evaluate_paired_route_fixture(&spec(), values, Some(&start), true).complete());
        let missing = evaluate_paired_route_fixture(&spec(), values, None, true);
        assert!(missing.proven.is_empty());
        assert!(
            missing
                .controls
                .contains(&crate::ControlFailure::MissingPositiveControl)
        );
        let held = evaluate_paired_route_fixture(&spec(), values, Some(&start), false);
        assert!(held.proven.is_empty());
        assert!(
            held.controls
                .contains(&crate::ControlFailure::CleanupUnconfirmed)
        );
        assert!(!evaluate_route_fixture(&spec(), [Some(&negative); 5], true).complete());
    }
    #[test]
    fn qualified_worker_leaks_survive_partial_read_and_unavailable_end_cleanup() {
        let (negative, start) = (rows(0), rows(1));
        let mut worker = start.clone();
        for v in &mut worker.observations {
            v.error = Some(ProbeError::Timeout);
            v.routes.truncate(1);
        }
        let result = evaluate_paired_route_fixture(
            &spec(),
            [Some(&negative), Some(&start), Some(&worker), None, None],
            Some(&start),
            false,
        );
        assert_eq!(result.leaked.len(), 8);
        assert!(result.proven.is_empty());
        for control in [
            crate::ControlFailure::ProbeUnavailable,
            crate::ControlFailure::FixtureChanged,
            crate::ControlFailure::CleanupUnconfirmed,
            crate::ControlFailure::OrdinaryObjectsChanged,
        ] {
            assert!(result.controls.contains(&control));
        }
    }
    #[test]
    fn scrubbing_destination_does_not_hide_exposed_interface_identity() {
        let (negative, start) = (rows(0), rows(1));
        let mut worker = start.clone();
        for observation in &mut worker.observations {
            observation.routes[0].destination.fill(0);
        }
        let result = evaluate_paired_route_fixture(
            &spec(),
            [
                Some(&negative),
                Some(&start),
                Some(&worker),
                Some(&start),
                Some(&negative),
            ],
            Some(&start),
            true,
        );
        assert_eq!(result.leaked.len(), 8);
        assert!(result.proven.is_empty());
        assert!(!spec().positive_measurement(&worker));
    }
    #[test]
    fn missing_positive_stimulus_never_qualifies_an_apparent_leak() {
        let (negative, start) = (rows(0), rows(1));
        let result = evaluate_route_fixture(
            &spec(),
            [
                Some(&negative),
                Some(&negative),
                Some(&start),
                Some(&start),
                Some(&negative),
            ],
            true,
        );
        assert!(result.leaked.is_empty());
        assert!(result.proven.is_empty());
        assert!(
            result
                .controls
                .contains(&crate::ControlFailure::MissingPositiveControl)
        );
    }
}
