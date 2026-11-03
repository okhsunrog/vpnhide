//! Root-only owned-TUN address setup. No global switch, link-up, packet IO,
//! prefix route, or retry against an unrelated/replaced interface.
use std::{
    mem::zeroed,
    os::fd::AsRawFd,
    time::{Duration, Instant},
};
use vpnhide_diagnostic_model::{
    AddressActor, NativeFixtureSpec, NativeMeasurement, ProbeError, Sample, fixture_addresses,
};

fn request(spec: &NativeFixtureSpec, ipv6: bool, sequence: u32) -> Vec<u8> {
    let (v4, v6) = fixture_addresses(spec.session);
    let address = if ipv6 { v6.as_slice() } else { v4.as_slice() };
    let flags = 0x200u32 | if ipv6 { 2 } else { 0 }; // NOPREFIXROUTE | NODAD
    let mut bytes = vec![0; 16];
    bytes[4..6].copy_from_slice(&20u16.to_ne_bytes()); // RTM_NEWADDR
    bytes[6..8].copy_from_slice(&0x605u16.to_ne_bytes()); // REQUEST | ACK | CREATE | EXCL
    bytes[8..12].copy_from_slice(&sequence.to_ne_bytes());
    bytes.extend_from_slice(&[
        if ipv6 {
            libc::AF_INET6 as u8
        } else {
            libc::AF_INET as u8
        },
        if ipv6 { 128 } else { 32 },
        flags as u8,
        0,
    ]);
    bytes.extend_from_slice(&spec.interface.index.to_ne_bytes());
    let mut attribute = |kind: u16, value: &[u8]| {
        bytes.extend_from_slice(&((4 + value.len()) as u16).to_ne_bytes());
        bytes.extend_from_slice(&kind.to_ne_bytes());
        bytes.extend_from_slice(value);
        bytes.resize((bytes.len() + 3) & !3, 0);
    };
    attribute(1, address); // IFA_ADDRESS (peer for point-to-point IPv4)
    if !ipv6 {
        attribute(2, address);
    } // IFA_LOCAL
    attribute(8, &flags.to_ne_bytes()); // full-width IFA_FLAGS
    let length = bytes.len() as u32;
    bytes[..4].copy_from_slice(&length.to_ne_bytes());
    bytes
}

pub(super) fn install(
    spec: &NativeFixtureSpec,
    negative: &NativeMeasurement,
    _fault: super::SetupFault,
) -> Result<(), ProbeError> {
    if !spec.valid() {
        return Err(ProbeError::Malformed);
    }
    let observations = negative.addresses.as_ref().ok_or(ProbeError::Unsupported)?;
    if !observations.bounded() {
        return Err(ProbeError::Malformed);
    }
    let baseline = &observations
        .actors
        .iter()
        .find(|v| v.actor == AddressActor::Syscall)
        .ok_or(ProbeError::Unsupported)?
        .netlink;
    let entries = match baseline {
        Sample::Value(v) => v,
        Sample::Error(e) => return Err(e.clone()),
    };
    if !entries
        .iter()
        .any(|a| a.name.as_deref() == Some("lo") && a.address == [127, 0, 0, 1])
    {
        return Err(ProbeError::Unsupported);
    }
    let (v4, v6) = fixture_addresses(spec.session);
    if entries.iter().any(|a| a.address == v4 || a.address == v6) {
        return Err(ProbeError::Errno(libc::EADDRINUSE));
    }
    // Descriptor lifetime owns both addresses, including an uncertain ACK. A
    // failed later family leaves an explicitly partial stimulus for evaluation.
    let fd = vpnhide_checks::netlink_read::bound_socket(AddressActor::Syscall)?;
    for (ipv6, sequence) in [(false, 1), (true, 2)] {
        if vpnhide_checks::fixture::index(&spec.interface.name) != Ok(spec.interface.index) {
            return Err(ProbeError::Malformed);
        }
        let bytes = request(spec, ipv6, sequence);
        let mut destination: libc::sockaddr_nl = unsafe { zeroed() };
        destination.nl_family = libc::AF_NETLINK as _;
        let sent = unsafe {
            libc::sendto(
                fd.as_raw_fd(),
                bytes.as_ptr().cast(),
                bytes.len(),
                0,
                (&destination as *const libc::sockaddr_nl).cast(),
                size_of::<libc::sockaddr_nl>() as _,
            )
        };
        if sent != bytes.len() as isize {
            return Err(ProbeError::Errno(
                std::io::Error::last_os_error()
                    .raw_os_error()
                    .filter(|e| *e > 0)
                    .unwrap_or(libc::EIO),
            ));
        }
        let reply = vpnhide_checks::netlink_read::receive(
            &fd,
            AddressActor::Syscall,
            Instant::now() + Duration::from_secs(1),
        )?;
        vpnhide_checks::netlink_wire::acknowledgement(&reply, &bytes)?;
        #[cfg(feature = "fixture-oracle")]
        if !ipv6 && _fault == super::SetupFault::AfterIpv4 {
            return Err(ProbeError::Unsupported);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpnhide_diagnostic_model::{InterfaceId, OrdinaryControl};

    #[test]
    fn exclusive_host_addresses_never_request_link_up_or_a_prefix_route() {
        let spec = NativeFixtureSpec {
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
        };
        for (ipv6, family, prefix, flags, expected_attributes) in [
            (
                false,
                2,
                32,
                0x200u32,
                vec![
                    (1, vec![198, 18, 0, 1]),
                    (2, vec![198, 18, 0, 1]),
                    (8, 0x200u32.to_ne_bytes().to_vec()),
                ],
            ),
            (
                true,
                10,
                128,
                0x202u32,
                vec![
                    (
                        1,
                        vec![
                            0xfd, 0x76, 0x70, 0x6e, 0x68, 0x69, 0x64, 0x65, 0, 0, 0, 0, 0, 1, 0, 1,
                        ],
                    ),
                    (8, 0x202u32.to_ne_bytes().to_vec()),
                ],
            ),
        ] {
            let wire = request(&spec, ipv6, 91);
            assert_eq!(
                u32::from_ne_bytes(wire[..4].try_into().unwrap()) as usize,
                wire.len()
            );
            assert_eq!(&wire[4..6], &20u16.to_ne_bytes());
            assert_eq!(&wire[6..8], &0x605u16.to_ne_bytes());
            assert_eq!(&wire[8..12], &91u32.to_ne_bytes());
            assert_eq!(&wire[12..16], &[0; 4]);
            assert_eq!(&wire[16..20], &[family, prefix, flags as u8, 0]);
            assert_eq!(&wire[20..24], &42u32.to_ne_bytes());
            let attributes = vpnhide_checks::netlink_wire::attributes(&wire[24..]).unwrap();
            assert_eq!(
                attributes
                    .iter()
                    .map(|a| (a.kind, a.value.to_vec()))
                    .collect::<Vec<_>>(),
                expected_attributes
            );
        }
    }
}
