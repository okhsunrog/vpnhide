//! Engineering-only owned route prototype. Not admitted by the production run
//! coordinator yet; required route/rule checks remain unverified there.
//! Existing DOWN fixtures are consumed into a distinct UP owner, never assessed
//! through the old DOWN contract. Nonpersistent TUN descriptor lifetime owns all
//! addresses/routes; no default route, global rule, packet IO or delete-by-name.
mod identity;
use super::native_fixture::{OracleUpTun, TunFixture};
pub use identity::RouteCleanupPlan;
use std::{
    io,
    mem::zeroed,
    os::fd::AsRawFd,
    time::{Duration, Instant},
};
use vpnhide_diagnostic_model::{
    InterfaceId, NativeReadActor, ProbeError, RouteFixtureSpec, RouteMeasurement, RouteSurface,
    ScopedRouteResult,
};

fn invalid(message: impl ToString) -> io::Error {
    io::Error::other(message.to_string())
}
fn request(spec: &RouteFixtureSpec, ordinary: bool, ipv6: bool, sequence: u32) -> Vec<u8> {
    let surface = if ipv6 {
        RouteSurface::NetlinkIpv6
    } else {
        RouteSurface::NetlinkIpv4
    };
    let interface = if ordinary { &spec.ordinary } else { &spec.vpn };
    let mut bytes = vec![0; 16];
    bytes[4..6].copy_from_slice(&24u16.to_ne_bytes()); // RTM_NEWROUTE
    bytes[6..8].copy_from_slice(&0x605u16.to_ne_bytes()); // REQUEST|ACK|CREATE|EXCL
    bytes[8..12].copy_from_slice(&sequence.to_ne_bytes());
    bytes.extend_from_slice(&[
        if ipv6 {
            libc::AF_INET6 as u8
        } else {
            libc::AF_INET as u8
        },
        if ipv6 { 128 } else { 32 },
        0,
        0,
        254,
        242,
        253,
        1,
        0,
        0,
        0,
        0,
    ]);
    let mut attribute = |kind: u16, value: &[u8]| {
        bytes.extend_from_slice(&((value.len() + 4) as u16).to_ne_bytes());
        bytes.extend_from_slice(&kind.to_ne_bytes());
        bytes.extend_from_slice(value);
        bytes.resize((bytes.len() + 3) & !3, 0);
    };
    attribute(1, &spec.destination(ordinary, surface));
    attribute(4, &interface.index.to_ne_bytes());
    attribute(6, &RouteFixtureSpec::metric(ordinary).to_ne_bytes());
    let length = bytes.len() as u32;
    bytes[..4].copy_from_slice(&length.to_ne_bytes());
    bytes
}
fn install(spec: &RouteFixtureSpec, ordinary: bool, ipv6: bool) -> io::Result<()> {
    let fd = vpnhide_checks::netlink_read::bound_socket(NativeReadActor::Syscall)
        .map_err(|e| invalid(format!("route socket: {e:?}")))?;
    let bytes = request(spec, ordinary, ipv6, 1);
    let mut target: libc::sockaddr_nl = unsafe { zeroed() };
    target.nl_family = libc::AF_NETLINK as _;
    let sent = unsafe {
        libc::sendto(
            fd.as_raw_fd(),
            bytes.as_ptr().cast(),
            bytes.len(),
            0,
            (&target as *const libc::sockaddr_nl).cast(),
            size_of::<libc::sockaddr_nl>() as _,
        )
    };
    if sent != bytes.len() as isize {
        return Err(io::Error::last_os_error());
    }
    let reply = vpnhide_checks::netlink_read::receive(
        &fd,
        NativeReadActor::Syscall,
        Instant::now() + Duration::from_secs(1),
    )
    .map_err(|e| invalid(format!("route ACK read: {e:?}")))?;
    vpnhide_checks::netlink_wire::acknowledgement(&reply, &bytes)
        .map_err(|e| invalid(format!("route ACK: {e:?}")))
}
fn absent(interface: &InterfaceId) -> bool {
    if !matches!(
        vpnhide_checks::fixture::index(&interface.name),
        Err(ProbeError::Errno(libc::ENODEV | libc::ENXIO))
    ) {
        return false;
    }
    let mut name = [0 as libc::c_char; libc::IFNAMSIZ];
    unsafe { libc::if_indextoname(interface.index, name.as_mut_ptr()) }.is_null()
        && matches!(
            io::Error::last_os_error().raw_os_error(),
            Some(libc::ENODEV | libc::ENXIO)
        )
}
pub struct RouteFixture {
    pub spec: RouteFixtureSpec,
    pub negative: RouteMeasurement,
    pub start: RouteMeasurement,
    owners: Option<[OracleUpTun; 2]>,
    cleanup_plan: RouteCleanupPlan,
}
pub struct RouteCleanup {
    pub end: RouteMeasurement,
    pub cleanup: RouteMeasurement,
    pub interfaces_clean: bool,
}
#[derive(Debug)]
struct InjectedSetupFault;
impl std::fmt::Display for InjectedSetupFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("injected first-route setup fault")
    }
}
impl std::error::Error for InjectedSetupFault {}
pub fn observe_cleanup(plan: &RouteCleanupPlan) -> (RouteMeasurement, bool) {
    plan.observe_cleanup()
}
pub struct RouteCreateFailure {
    pub error: io::Error,
    pub cleanup_plan: Option<Box<RouteCleanupPlan>>,
    pub resource_created: bool,
    pub cleanup_confirmed: bool,
}

pub struct RouteCompletion {
    pub end: RouteMeasurement,
    pub cleanup: RouteMeasurement,
    pub interfaces_clean: bool,
    pub result: ScopedRouteResult,
}
impl RouteFixture {
    pub fn create(session: u64) -> io::Result<Self> {
        Self::create_detailed(session).map_err(|f| f.error)
    }
    pub fn create_detailed(session: u64) -> Result<Self, RouteCreateFailure> {
        Self::create_tracked(session, false)
    }
    fn create_tracked(session: u64, fault: bool) -> Result<Self, RouteCreateFailure> {
        let placeholder = |name| InterfaceId { name, index: 0 };
        let mut spec = RouteFixtureSpec {
            session,
            vpn: placeholder(String::new()),
            ordinary: placeholder(String::new()),
        };
        let prepared = (|| -> io::Result<RouteCleanupPlan> {
            spec.vpn.name = super::native_fixture::fixture_name(spec.role_session(false))?;
            spec.ordinary.name = format!(
                "vhr{}",
                &super::native_fixture::fixture_name(spec.role_session(true))?[3..]
            );
            RouteCleanupPlan::new(spec)
        })();
        let mut plan = prepared.map_err(|error| RouteCreateFailure {
            error,
            cleanup_plan: None,
            resource_created: false,
            cleanup_confirmed: true,
        })?;
        Self::create_inner(&mut plan, fault).map_err(|error| {
            let created = plan.created;
            let cleaned = if created {
                plan.observe_cleanup().1
            } else {
                true
            };
            RouteCreateFailure {
                error,
                cleanup_plan: Some(Box::new(plan)),
                resource_created: created,
                cleanup_confirmed: cleaned,
            }
        })
    }
    pub fn failed_setup_for_oracle(session: u64) -> io::Result<RouteCleanupPlan> {
        match Self::create_tracked(session, true) {
            Err(failure)
                if failure
                    .error
                    .get_ref()
                    .is_some_and(|e| e.is::<InjectedSetupFault>()) =>
            {
                failure
                    .cleanup_plan
                    .map(|plan| *plan)
                    .ok_or_else(|| invalid("fault lacks cleanup identity"))
            }
            Err(failure) => Err(failure.error),
            Ok(_) => Err(invalid("setup fault did not fire")),
        }
    }
    fn allocate_role(plan: &mut RouteCleanupPlan, ordinary: bool) -> io::Result<OracleUpTun> {
        if !plan.current() {
            return Err(invalid("route creation namespace/boot changed"));
        }
        let down = match TunFixture::create_detailed(plan.spec.role_session(ordinary)) {
            Ok(v) => v,
            Err(failure) => {
                plan.created |= failure.resource_created;
                if let Some((spec, _)) = failure.cleanup {
                    plan.record(spec.interface, ordinary);
                } else if failure.resource_created {
                    plan.unidentified = true;
                }
                return Err(failure.error);
            }
        };
        plan.record(down.spec.interface.clone(), ordinary);
        let up = down.into_up_for_oracle(ordinary)?;
        plan.record(up.interface.clone(), ordinary);
        Ok(up)
    }
    fn create_inner(plan: &mut RouteCleanupPlan, fault: bool) -> io::Result<Self> {
        let spec = &plan.spec;
        if vpnhide_checks::is_vpn_interface_name(&spec.ordinary.name) {
            return Err(invalid(
                "ordinary route interface would match VPN vocabulary",
            ));
        }
        for name in [
            &spec.vpn.name,
            &spec.ordinary.name,
            &super::native_fixture::fixture_name(spec.role_session(true))?,
        ] {
            if !matches!(
                vpnhide_checks::fixture::index(name),
                Err(ProbeError::Errno(libc::ENODEV | libc::ENXIO))
            ) {
                return Err(invalid(
                    "planned route interface name occupied or unavailable",
                ));
            }
        }
        let negative = vpnhide_checks::route::observe();
        if !spec.negative(&negative) {
            return Err(invalid(format!(
                "route negative control unavailable/collision: {negative:?}"
            )));
        }
        let vpn = Self::allocate_role(plan, false)?;
        let ordinary = Self::allocate_role(plan, true)?;
        let spec = &plan.spec;
        if !spec.valid() || !plan.current() {
            return Err(invalid("route fixture identity/environment changed"));
        }
        for ordinary_role in [false, true] {
            for ipv6 in [false, true] {
                install(spec, ordinary_role, ipv6)?;
                if fault {
                    return Err(io::Error::other(InjectedSetupFault));
                }
            }
        }
        let start = vpnhide_checks::route::observe();
        if !spec.positive_measurement(&start) {
            return Err(invalid(format!(
                "route positive control missing: {start:?}"
            )));
        }
        Ok(Self {
            spec: spec.clone(),
            negative,
            start,
            owners: Some([vpn, ordinary]),
            cleanup_plan: plan.clone(),
        })
    }
    pub fn cleanup_plan(&self) -> RouteCleanupPlan {
        self.cleanup_plan.clone()
    }
    pub fn duplicate_for_oracle(&self, ordinary: bool) -> io::Result<std::fs::File> {
        self.owners
            .as_ref()
            .ok_or_else(|| invalid("route owners closed"))?[usize::from(ordinary)]
        .duplicate_for_oracle()
    }
    fn measure_cleanup(&mut self) -> RouteCleanup {
        let end = if self.cleanup_plan.current() {
            vpnhide_checks::route::observe()
        } else {
            identity::unavailable()
        };
        drop(self.owners.take());
        let (cleanup, interfaces_clean) = observe_cleanup(&self.cleanup_plan);
        RouteCleanup {
            end,
            cleanup,
            interfaces_clean,
        }
    }
    pub fn close_for_oracle(mut self) -> RouteCleanup {
        self.measure_cleanup()
    }
    pub fn finish(
        mut self,
        worker: &RouteMeasurement,
        raw: Option<&RouteMeasurement>,
        protected: bool,
    ) -> RouteCompletion {
        let RouteCleanup {
            end,
            cleanup,
            interfaces_clean,
        } = self.measure_cleanup();
        let measurements = [
            Some(&self.negative),
            Some(&self.start),
            Some(worker),
            Some(&end),
            Some(&cleanup),
        ];
        let result = if protected {
            vpnhide_diagnostic_model::evaluate_paired_route_fixture(
                &self.spec,
                measurements,
                raw,
                interfaces_clean,
            )
        } else {
            vpnhide_diagnostic_model::evaluate_route_fixture(
                &self.spec,
                measurements,
                interfaces_clean,
            )
        };
        RouteCompletion {
            end,
            cleanup,
            interfaces_clean,
            result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn route_write_request_is_exclusive_host_only_and_binds_the_owned_output_index() {
        let spec = RouteFixtureSpec {
            session: 1,
            vpn: InterfaceId {
                name: "tun000000004001".into(),
                index: 7,
            },
            ordinary: InterfaceId {
                name: "vhr000000008001".into(),
                index: 8,
            },
        };
        for ordinary in [false, true] {
            for ipv6 in [false, true] {
                let bytes = request(&spec, ordinary, ipv6, 19);
                let frames = vpnhide_checks::netlink_wire::messages(&bytes).unwrap();
                assert_eq!(frames.len(), 1);
                let frame = &frames[0];
                assert_eq!(frame.kind, 24);
                assert_eq!(frame.flags, 0x605);
                assert_eq!(frame.sequence, 19);
                assert_eq!(
                    &frame.body[1..8],
                    &[if ipv6 { 128 } else { 32 }, 0, 0, 254, 242, 253, 1]
                );
                let attrs = vpnhide_checks::netlink_wire::attributes(&frame.body[12..]).unwrap();
                assert_eq!(attrs.iter().map(|v| v.kind).collect::<Vec<_>>(), [1, 4, 6]);
                assert_eq!(
                    vpnhide_checks::netlink_wire::u32_at(attrs[1].value, 0).unwrap(),
                    if ordinary { 8 } else { 7 }
                );
            }
        }
    }
}
