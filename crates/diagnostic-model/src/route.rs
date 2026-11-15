//! Literal route observations and evaluation with owned positive, ordinary and
//! cleanup controls.
use crate::{NativeReadActor, ObservedRule, ProbeError, ProcReadDenial, RuleRole};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_ROUTES: usize = 2048;
/// Outbound observations describe only Core-issued objects, without rewriting
/// their fields. Reserve room for the other native vectors in the 1 MiB RPC.
pub const MAX_SCOPED_ROUTES: usize = 16;
pub const SCOPED_ROUTE_JSON_BUDGET: usize = 128 * 1024;
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteSurface {
    NetlinkIpv4,
    NetlinkIpv6,
    ProcIpv4,
    ProcIpv6,
    RuleIpv4,
    RuleIpv6,
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
            Self::NetlinkIpv4 | Self::ProcIpv4 | Self::RuleIpv4 => 4,
            Self::NetlinkIpv6 | Self::ProcIpv6 | Self::RuleIpv6 => 16,
        }
    }
    pub fn netlink(self) -> bool {
        matches!(self, Self::NetlinkIpv4 | Self::NetlinkIpv6)
    }
    pub fn proc_route(self) -> bool {
        matches!(self, Self::ProcIpv4 | Self::ProcIpv6)
    }
    pub fn rule(self) -> bool {
        matches!(self, Self::RuleIpv4 | Self::RuleIpv6)
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
    pub fn rules() -> impl Iterator<Item = Self> {
        NativeReadActor::ALL.into_iter().flat_map(|actor| {
            [RouteSurface::RuleIpv4, RouteSurface::RuleIpv6]
                .into_iter()
                .map(move |surface| Self { actor, surface })
        })
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<ObservedRule>,
    pub error: Option<ProbeError>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteMeasurement {
    pub observations: Vec<RouteObservation>,
}
impl RouteMeasurement {
    pub fn unavailable(error: ProbeError) -> Self {
        Self {
            observations: RouteCheck::required()
                .into_iter()
                .map(|check| RouteObservation {
                    check,
                    routes: Vec::new(),
                    rules: Vec::new(),
                    error: Some(error.clone()),
                })
                .collect(),
        }
    }

    /// Local transport projection, never a cleaned network API response or
    /// verdict. Every exposure tag is retained; failures and overflow remain
    /// explicit. Only Core-issued fixture rows enter the worker report.
    pub fn scoped(&self, spec: &RouteFixtureSpec) -> Self {
        if !spec.valid() || !self.bounded() {
            return Self::unavailable(ProbeError::Malformed);
        }
        Self {
            observations: self
                .observations
                .iter()
                .map(|value| {
                    let mut rows = value.routes.iter().filter(|row| {
                        spec.exposes(row, false, value.check.surface)
                            || spec.exposes(row, true, value.check.surface)
                    });
                    let routes = rows.by_ref().take(MAX_SCOPED_ROUTES).cloned().collect();
                    let rules = value.rules.iter().filter(|row| {
                        RuleRole::ALL
                            .into_iter()
                            .any(|role| role.exposes(spec, row))
                    });
                    let mut rules = rules;
                    let retained_rules = rules.by_ref().take(MAX_SCOPED_ROUTES).cloned().collect();
                    let overflow = rows.next().is_some() || rules.next().is_some();
                    RouteObservation {
                        check: value.check,
                        routes,
                        rules: retained_rules,
                        error: value
                            .error
                            .clone()
                            .or(overflow.then_some(ProbeError::TooLarge)),
                    }
                })
                .collect(),
        }
    }
    pub fn bounded(&self) -> bool {
        let mut seen = BTreeSet::new();
        self.observations.len() <= RouteCheck::required().len() + RouteCheck::rules().count()
            && self.observations.iter().all(|v| {
                seen.insert(v.check)
                    && v.routes.len() <= MAX_ROUTES
                    && v.routes.iter().all(|r| r.valid(v.check.surface))
                    && v.rules.len() <= MAX_ROUTES
                    && v.rules.iter().all(ObservedRule::valid)
                    && if v.check.surface.rule() {
                        v.routes.is_empty()
                    } else {
                        v.rules.is_empty()
                    }
            })
    }

    pub fn bounded_scoped(&self) -> bool {
        self.bounded()
            && self
                .observations
                .iter()
                .all(|v| v.routes.len() <= MAX_SCOPED_ROUTES && v.rules.len() <= MAX_SCOPED_ROUTES)
    }

    pub fn scoped_for(&self, spec: &RouteFixtureSpec) -> bool {
        spec.valid()
            && self.bounded_scoped()
            && self.observations.iter().all(|value| {
                (!value.check.surface.rule() || spec.rule_uid.is_some())
                    && value.rules.iter().all(|row| {
                        RuleRole::ALL
                            .into_iter()
                            .any(|role| role.exposes(spec, row))
                    })
                    && value.routes.iter().all(|row| {
                        spec.exposes(row, false, value.check.surface)
                            || spec.exposes(row, true, value.check.surface)
                    })
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
            rules: vec![],
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_uid: Option<u32>,
}
impl RouteFixtureSpec {
    fn checks(&self) -> impl Iterator<Item = RouteCheck> {
        RouteCheck::required()
            .into_iter()
            .chain(RouteCheck::rules().filter(|_| self.rule_uid.is_some()))
    }

    pub fn valid(&self) -> bool {
        self.session != 0
            && self
                .rule_uid
                .is_none_or(|uid| (10_000..u32::MAX).contains(&uid))
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcRouteAccessDenial {
    pub check: RouteCheck,
    pub raw: ProcReadDenial,
    pub protected: ProcReadDenial,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopedRouteResult {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub access_denials: Vec<ProcRouteAccessDenial>,
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
fn positive(spec: &RouteFixtureSpec, value: &RouteObservation, ordinary: bool) -> bool {
    if value.check.surface.rule() {
        RuleRole::ALL
            .into_iter()
            .filter(|role| (*role == RuleRole::Ordinary) == ordinary)
            .all(|role| value.rules.iter().any(|row| role.matches(spec, row, false)))
    } else {
        value
            .routes
            .iter()
            .any(|row| spec.positive(row, ordinary, value.check.surface))
    }
}

fn exposes(spec: &RouteFixtureSpec, value: &RouteObservation, ordinary: bool) -> bool {
    if value.check.surface.rule() {
        RuleRole::ALL
            .into_iter()
            .filter(|role| (*role == RuleRole::Ordinary) == ordinary)
            .any(|role| value.rules.iter().any(|row| role.exposes(spec, row)))
    } else {
        value
            .routes
            .iter()
            .any(|row| spec.exposes(row, ordinary, value.check.surface))
    }
}

fn paired_positive(spec: &RouteFixtureSpec, value: &RouteObservation) -> bool {
    value.error.is_none() && positive(spec, value, false) && positive(spec, value, true)
}
/// Root controls qualify each vector before a worker row can establish a leak.
/// Later worker/ordinary/end/cleanup failures preserve that leak and withhold proof.
/// `interfaces_clean` is Root's independent name/index cleanup, never app input.
pub fn evaluate_route_fixture(
    spec: &RouteFixtureSpec,
    measurements: [Option<&RouteMeasurement>; 5],
    interfaces_clean: bool,
) -> ScopedRouteResult {
    evaluate_route_controls(spec, measurements, None, interfaces_clean)
}

fn permission_denial(value: Option<&RouteObservation>) -> Option<ProcReadDenial> {
    let value = value?;
    if !value.check.surface.proc_route() || !value.routes.is_empty() {
        return None;
    }
    match value.error {
        // Linux EPERM and EACCES. Rows retained from a partial read can still
        // establish an exposure, so only a denied sample with no rows qualifies.
        Some(ProbeError::ProcPermissionDenied(denial)) if matches!(denial.errno, 1 | 13) => {
            Some(denial)
        }
        _ => None,
    }
}

fn evaluate_route_controls(
    spec: &RouteFixtureSpec,
    measurements: [Option<&RouteMeasurement>; 5],
    app_raw: Option<&RouteMeasurement>,
    interfaces_clean: bool,
) -> ScopedRouteResult {
    use crate::ControlFailure::*;
    let mut result = ScopedRouteResult::default();
    if !spec.valid() {
        result.problem(InvalidSpec);
        return result;
    }
    for check in spec.checks() {
        let [negative, start, worker, end, cleanup] = measurements.map(|m| observation(m, check));
        let absent = |v: &RouteObservation| {
            v.error.is_none() && !exposes(spec, v, false) && !exposes(spec, v, true)
        };
        let root_positive = |v: &RouteObservation| paired_positive(spec, v);
        if negative.is_none_or(|v| !absent(v)) {
            result.problem(MissingNegativeControl);
            if negative.is_none_or(|v| v.error.is_some()) {
                result.problem(ProbeUnavailable);
            }
            continue;
        }
        if start.is_none_or(|v| !root_positive(v)) {
            result.problem(MissingPositiveControl);
            if start.is_none_or(|v| v.error.is_some()) {
                result.problem(ProbeUnavailable);
            }
            continue;
        }
        let end_ok = end.is_some_and(root_positive);
        let cleaned = interfaces_clean && cleanup.is_some_and(absent);
        if end_ok
            && cleaned
            && let Some(raw) = permission_denial(observation(app_raw, check))
            && let Some(protected) = permission_denial(worker)
        {
            result.access_denials.push(ProcRouteAccessDenial {
                check,
                raw,
                protected,
            });
            continue;
        }
        let leaked = worker.is_some_and(|v| exposes(spec, v, false));
        if leaked {
            result.leaked.push(check);
        }
        let worker_available = worker.is_some_and(|v| v.error.is_none());
        if !worker_available {
            result.problem(ProbeUnavailable);
        }
        let ordinary_ok = worker.is_some_and(|v| positive(spec, v, true));
        if !ordinary_ok {
            result.problem(OrdinaryObjectsChanged);
        }
        if !end_ok {
            result.problem(FixtureChanged);
            if end.is_none_or(|v| v.error.is_some()) {
                result.problem(ProbeUnavailable);
            }
        }
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
    let mut result = evaluate_route_controls(spec, measurements, raw, interfaces_clean);
    let qualified: Vec<_> = spec
        .checks()
        .filter(|check| observation(raw, *check).is_some_and(|v| paired_positive(spec, v)))
        .collect();
    result.proven.retain(|c| qualified.contains(c));
    if qualified.len() + result.access_denials.len() != spec.checks().count() {
        result.problem(crate::ControlFailure::MissingPositiveControl);
    }

    result
}

#[cfg(test)]
mod fixture_tests {
    use super::*;
    fn spec() -> RouteFixtureSpec {
        RouteFixtureSpec {
            rule_uid: None,
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
                        rules: Vec::new(),
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
    fn proc_denial_needs_both_worker_phases_and_live_root_controls() {
        use crate::ProcReadStage::*;
        let root_absent = rows(0);
        let root_visible = rows(1);
        let denied = |stage| {
            let mut sample = rows(2);
            for entry in &mut sample.observations {
                if entry.check.surface.proc_route() {
                    entry.routes.clear();
                    entry.error = Some(ProbeError::ProcPermissionDenied(ProcReadDenial {
                        stage,
                        errno: 13,
                    }));
                }
            }
            sample
        };
        let mut raw = denied(Open);
        for entry in &mut raw.observations {
            if entry.check.surface.netlink() {
                entry.routes.clone_from(
                    &observation(Some(&root_visible), entry.check)
                        .unwrap()
                        .routes,
                );
            }
        }
        let protected = denied(Read);
        let samples = [
            Some(&root_absent),
            Some(&root_visible),
            Some(&protected),
            Some(&root_visible),
            Some(&root_absent),
        ];
        let result = evaluate_paired_route_fixture(&spec(), samples, Some(&raw), true);
        assert_eq!(result.access_denials.len(), 4);
        assert!(
            result
                .access_denials
                .iter()
                .all(|v| v.raw.stage == Open && v.protected.stage == Read)
        );
        assert_eq!(result.proven.len(), 4);
        assert!(result.leaked.is_empty() && result.controls.is_empty());
        assert!(
            evaluate_paired_route_fixture(&spec(), samples, None, true)
                .access_denials
                .is_empty()
        );
        assert!(
            evaluate_paired_route_fixture(&spec(), samples, Some(&raw), false)
                .access_denials
                .is_empty()
        );
        let mut late_error = protected;
        let leaking = late_error
            .observations
            .iter_mut()
            .find(|v| v.check.surface.proc_route())
            .unwrap();
        leaking.routes = observation(Some(&root_visible), leaking.check)
            .unwrap()
            .routes
            .clone();
        let leaking_check = leaking.check;
        let result = evaluate_paired_route_fixture(
            &spec(),
            [
                Some(&root_absent),
                Some(&root_visible),
                Some(&late_error),
                Some(&root_visible),
                Some(&root_absent),
            ],
            Some(&raw),
            true,
        );
        assert_eq!(result.access_denials.len(), 3);
        assert_eq!(result.leaked, vec![leaking_check]);
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
    #[test]
    fn transport_projection_keeps_scrubbed_identity_leaks_and_late_errors() {
        let mut source = rows(1);
        for value in &mut source.observations {
            value.routes[0].destination.fill(0); // interface identity still leaks
            let mut unrelated = value.routes[1].clone();
            unrelated.output_index = Some(19);
            unrelated.output_name = Some("eth0".into());
            unrelated.destination.fill(17);
            value.routes.push(unrelated);
            value.error = Some(ProbeError::Timeout);
        }
        let projected = source.scoped(&spec());
        for value in &projected.observations {
            assert_eq!(value.routes.len(), 2);
            assert_eq!(
                value.routes[0].destination,
                vec![0; value.check.surface.address_bytes()]
            );
            assert_eq!(value.error, Some(ProbeError::Timeout));
        }
        let negative = rows(0);
        let start = rows(1);
        let result = evaluate_route_fixture(
            &spec(),
            [Some(&negative), Some(&start), Some(&projected), None, None],
            false,
        );
        assert_eq!(result.leaked.len(), 8);
        assert!(result.proven.is_empty());
    }
    #[test]
    fn maximum_scoped_route_payload_has_an_explicit_transport_reservation() {
        let mut spec = spec();
        spec.vpn.index = i32::MAX as u32;
        spec.ordinary.index = i32::MAX as u32 - 1;
        let mut source = rows(1);
        for value in &mut source.observations {
            let length = value.check.surface.address_bytes();
            let mut row = value.routes[0].clone();
            row.destination = vec![255; length];
            row.source = Some(vec![255; length]);
            row.source_prefix = (length * 8) as u8;
            row.output_index = Some(spec.vpn.index);
            row.output_name = Some("\"".repeat(15));
            row.table = Some(u32::MAX);
            row.protocol = Some(u8::MAX);
            row.scope = Some(u8::MAX);
            row.kind = Some(u8::MAX);
            row.metric = u32::MAX;
            row.gateway = Some(vec![255; length]);
            row.flags = u32::MAX;
            value.routes = vec![row; MAX_SCOPED_ROUTES + 1];
        }
        assert!(source.bounded());
        assert!(!source.scoped_for(&spec));
        let bounded = source.scoped(&spec);
        assert!(bounded.scoped_for(&spec));
        assert!(
            bounded
                .observations
                .iter()
                .all(|v| v.routes.len() == MAX_SCOPED_ROUTES
                    && v.error == Some(ProbeError::TooLarge))
        );
        assert!(serde_json::to_vec(&bounded).unwrap().len() < SCOPED_ROUTE_JSON_BUDGET);
        assert!(bounded.observations.iter().all(|v| v.error.is_some()));
    }
}
