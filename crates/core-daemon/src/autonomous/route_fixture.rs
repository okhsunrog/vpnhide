//! Engineering-only owned route prototype. Not admitted by the production run
//! coordinator yet; required route/rule checks remain unverified there.
//! Existing DOWN fixtures are consumed into a distinct UP owner, never assessed
//! through the old DOWN contract. Nonpersistent TUN descriptor lifetime owns all
//! addresses/routes; no default route, global rule, packet IO or delete-by-name.
use super::native_fixture::{OracleUpTun, TunFixture};
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
pub fn observe_cleanup(spec: &RouteFixtureSpec) -> (RouteMeasurement, bool) {
    (
        vpnhide_checks::route::observe(),
        absent(&spec.vpn) && absent(&spec.ordinary),
    )
}
pub struct RouteCompletion {
    pub end: RouteMeasurement,
    pub cleanup: RouteMeasurement,
    pub interfaces_clean: bool,
    pub result: ScopedRouteResult,
}
impl RouteFixture {
    pub fn create(session: u64) -> io::Result<Self> {
        Self::create_inner(session, false, &mut None)
    }
    pub fn failed_setup_for_oracle(session: u64) -> io::Result<RouteFixtureSpec> {
        let mut reached = None;
        match Self::create_inner(session, true, &mut reached) {
            Err(error)
                if error
                    .get_ref()
                    .is_some_and(|e| e.is::<InjectedSetupFault>()) =>
            {
                reached.ok_or_else(|| invalid("fault lacks created identities"))
            }
            Err(error) => Err(error),
            Ok(_) => Err(invalid("setup fault did not fire")),
        }
    }
    fn create_inner(
        session: u64,
        fault: bool,
        reached: &mut Option<RouteFixtureSpec>,
    ) -> io::Result<Self> {
        let placeholder = |name| InterfaceId { name, index: 0 };
        let mut spec = RouteFixtureSpec {
            session,
            vpn: placeholder(String::new()),
            ordinary: placeholder(String::new()),
        };
        spec.vpn.name = super::native_fixture::fixture_name(spec.role_session(false))?;
        spec.ordinary.name = format!(
            "vhr{}",
            &super::native_fixture::fixture_name(spec.role_session(true))?[3..]
        );
        if vpnhide_checks::is_vpn_interface_name(&spec.ordinary.name) {
            return Err(invalid(
                "ordinary route interface would match VPN vocabulary",
            ));
        }
        let negative = vpnhide_checks::route::observe();
        if !spec.negative(&negative) {
            return Err(invalid(format!(
                "route negative control unavailable/collision: {negative:?}"
            )));
        }
        let vpn = TunFixture::create(spec.role_session(false))?.into_up_for_oracle(false)?;
        let ordinary = TunFixture::create(spec.role_session(true))?.into_up_for_oracle(true)?;
        spec.vpn = vpn.interface.clone();
        spec.ordinary = ordinary.interface.clone();
        if !spec.valid() {
            return Err(invalid("route fixture identity invalid"));
        }
        *reached = Some(spec.clone());
        for ordinary_role in [false, true] {
            for ipv6 in [false, true] {
                install(&spec, ordinary_role, ipv6)?;
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
            spec,
            negative,
            start,
            owners: Some([vpn, ordinary]),
        })
    }
    pub fn duplicate_for_oracle(&self, ordinary: bool) -> io::Result<std::fs::File> {
        self.owners
            .as_ref()
            .ok_or_else(|| invalid("route owners closed"))?[usize::from(ordinary)]
        .duplicate_for_oracle()
    }
    fn measure_cleanup(&mut self) -> RouteCleanup {
        let end = vpnhide_checks::route::observe();
        drop(self.owners.take());
        let (cleanup, interfaces_clean) = observe_cleanup(&self.spec);
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
