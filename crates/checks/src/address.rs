//! Actual bounded app/root address observations. No addresses or policy are
//! installed here; direct syscall actors never proxy through libc ioctl/recvmsg.
use std::{
    collections::BTreeSet,
    io,
    mem::zeroed,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
};
use vpnhide_diagnostic_model::{
    AddressActorMeasurement, AddressIoctl, AddressIoctlObservation, AddressMeasurement,
    IfconfObservation, MAX_ADDRESSES, NativeReadActor, ObservedAddress, ProbeError, Sample,
};

fn error() -> ProbeError {
    ProbeError::Errno(
        io::Error::last_os_error()
            .raw_os_error()
            .filter(|e| *e > 0)
            .unwrap_or(libc::EIO),
    )
}
fn sample<T>(value: Result<T, ProbeError>) -> Sample<T> {
    match value {
        Ok(v) => Sample::Value(v),
        Err(e) => Sample::Error(e),
    }
}
fn socket(actor: NativeReadActor) -> Result<OwnedFd, ProbeError> {
    let kind = libc::SOCK_DGRAM | libc::SOCK_CLOEXEC;
    let fd = unsafe {
        match actor {
            NativeReadActor::Libc => libc::socket(libc::AF_INET, kind, 0) as libc::c_long,
            NativeReadActor::Syscall => libc::syscall(libc::SYS_socket, libc::AF_INET, kind, 0),
        }
    };
    if fd < 0 {
        return Err(error());
    }
    // Successful socket descriptors are c_int even when syscall returns c_long.
    Ok(unsafe { OwnedFd::from_raw_fd(fd as libc::c_int) })
}
fn ioctl(
    actor: NativeReadActor,
    fd: i32,
    command: libc::c_ulong,
    request: *mut libc::c_void,
) -> Result<(), ProbeError> {
    let result = unsafe {
        match actor {
            NativeReadActor::Libc => libc::ioctl(fd, command as _, request) as libc::c_long,
            NativeReadActor::Syscall => libc::syscall(libc::SYS_ioctl, fd, command, request),
        }
    };
    if result < 0 {
        Err(error())
    } else if result == 0 {
        Ok(())
    } else {
        Err(ProbeError::Malformed)
    }
}
fn request(name: &str) -> Result<libc::ifreq, ProbeError> {
    if name.is_empty()
        || name.len() >= libc::IFNAMSIZ
        || !name.is_ascii()
        || name.as_bytes().contains(&0)
    {
        return Err(ProbeError::Malformed);
    }
    let mut req: libc::ifreq = unsafe { zeroed() };
    for (target, byte) in req.ifr_name.iter_mut().zip(name.bytes()) {
        *target = byte as libc::c_char;
    }
    req.ifr_ifru.ifru_addr.sa_family = libc::AF_INET as _;
    Ok(req)
}
fn ipv4(address: libc::sockaddr) -> Result<Vec<u8>, ProbeError> {
    if address.sa_family != libc::AF_INET as libc::sa_family_t {
        return Err(ProbeError::Malformed);
    }
    Ok(address.sa_data[2..6]
        .iter()
        .map(|b| b.to_ne_bytes()[0])
        .collect())
}
fn ioctl_address(
    name: &str,
    kind: AddressIoctl,
    actor: NativeReadActor,
) -> Result<Vec<u8>, ProbeError> {
    let mut req = request(name)?;
    let fd = socket(actor)?;
    let command = match kind {
        AddressIoctl::Address => libc::SIOCGIFADDR,
        AddressIoctl::Netmask => libc::SIOCGIFNETMASK,
        AddressIoctl::Broadcast => libc::SIOCGIFBRDADDR,
        AddressIoctl::Destination => libc::SIOCGIFDSTADDR,
    };
    ioctl(
        actor,
        fd.as_raw_fd(),
        command,
        (&mut req as *mut libc::ifreq).cast(),
    )?;
    ipv4(unsafe { req.ifr_ifru.ifru_addr })
}
fn name(bytes: &[libc::c_char]) -> Result<String, ProbeError> {
    let bytes: Vec<_> = bytes.iter().map(|b| b.to_ne_bytes()[0]).collect();
    let length = bytes
        .iter()
        .position(|b| *b == 0)
        .ok_or(ProbeError::Malformed)?;
    Ok(crate::netlink_wire::interface_name(&bytes[..length + 1])?.into())
}
fn ifconf_size(fd: &OwnedFd, actor: NativeReadActor) -> Result<u32, ProbeError> {
    let mut req: libc::ifconf = unsafe { zeroed() };
    ioctl(
        actor,
        fd.as_raw_fd(),
        libc::SIOCGIFCONF,
        (&mut req as *mut libc::ifconf).cast(),
    )?;
    u32::try_from(req.ifc_len).map_err(|_| ProbeError::Malformed)
}
fn ifconf(actor: NativeReadActor) -> Result<IfconfObservation, ProbeError> {
    let fd = socket(actor)?;
    let record_bytes = size_of::<libc::ifreq>();
    let before = ifconf_size(&fd, actor)?;
    if before as usize > MAX_ADDRESSES * record_bytes {
        return Err(ProbeError::TooLarge);
    }
    // Typed backing storage has ifreq alignment on both Android ABIs.
    let mut records: Vec<libc::ifreq> = std::iter::repeat_with(|| unsafe { zeroed() })
        .take(MAX_ADDRESSES + 1)
        .collect();
    let capacity = records.len() * record_bytes;
    let mut req: libc::ifconf = unsafe { zeroed() };
    req.ifc_len = capacity as i32;
    req.ifc_ifcu.ifcu_buf = records.as_mut_ptr().cast();
    ioctl(
        actor,
        fd.as_raw_fd(),
        libc::SIOCGIFCONF,
        (&mut req as *mut libc::ifconf).cast(),
    )?;
    let used = usize::try_from(req.ifc_len).map_err(|_| ProbeError::Malformed)?;
    if used > capacity || !used.is_multiple_of(record_bytes) {
        return Err(ProbeError::Malformed);
    }
    if used / record_bytes > MAX_ADDRESSES {
        return Err(ProbeError::TooLarge);
    }
    let after = ifconf_size(&fd, actor)?;
    let entries = records[..used / record_bytes]
        .iter()
        .map(|r| {
            let name = name(&r.ifr_name)?;
            Ok(ObservedAddress {
                index: crate::fixture::index(&name).ok(),
                name: Some(name),
                address: ipv4(unsafe { r.ifr_ifru.ifru_addr })?,
                prefix: None,
            })
        })
        .collect::<Result<Vec<_>, ProbeError>>()?;
    Ok(IfconfObservation {
        entries,
        record_bytes: record_bytes as u32,
        used_bytes: used as u32,
        size_before: before,
        size_after: after,
    })
}

fn prefix(mask: &[u8]) -> Result<u8, ProbeError> {
    let mut count = 0;
    let mut zero = false;
    for byte in mask {
        for bit in (0..8).rev() {
            if byte & (1 << bit) != 0 {
                if zero {
                    return Err(ProbeError::Malformed);
                }
                count += 1;
            } else {
                zero = true;
            }
        }
    }
    Ok(count)
}
unsafe fn address_bytes(address: *const libc::sockaddr) -> Result<Option<Vec<u8>>, ProbeError> {
    if address.is_null() {
        return Err(ProbeError::Malformed);
    }
    match unsafe { (*address).sa_family as i32 } {
        libc::AF_INET => Ok(Some(unsafe {
            (*address.cast::<libc::sockaddr_in>())
                .sin_addr
                .s_addr
                .to_ne_bytes()
                .to_vec()
        })),
        libc::AF_INET6 => Ok(Some(unsafe {
            (*address.cast::<libc::sockaddr_in6>())
                .sin6_addr
                .s6_addr
                .to_vec()
        })),
        _ => Ok(None),
    }
}
fn ifaddrs() -> Result<Vec<ObservedAddress>, ProbeError> {
    let mut list = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return Err(error());
    }
    struct List(*mut libc::ifaddrs);
    impl Drop for List {
        fn drop(&mut self) {
            unsafe { libc::freeifaddrs(self.0) }
        }
    }
    let _owner = List(list);
    let mut visited = BTreeSet::new();
    let mut entries = BTreeSet::new();
    let mut at = list;
    while !at.is_null() {
        if visited.len() >= MAX_ADDRESSES * 4 || !visited.insert(at as usize) {
            return Err(ProbeError::TooLarge);
        }
        let item = unsafe { &*at };
        // Addressless link-layer entries remain in the separate interface probe.
        if !item.ifa_addr.is_null()
            && let Some(address) = unsafe { address_bytes(item.ifa_addr) }?
        {
            let label = if item.ifa_name.is_null() {
                None
            } else {
                let length = unsafe { libc::strnlen(item.ifa_name, libc::IFNAMSIZ) };
                if length >= libc::IFNAMSIZ {
                    return Err(ProbeError::Malformed);
                }
                if length == 0 {
                    None
                } else {
                    Some(
                        crate::netlink_wire::interface_name(unsafe {
                            std::slice::from_raw_parts(item.ifa_name.cast(), length + 1)
                        })?
                        .to_owned(),
                    )
                }
            };
            let mask = if item.ifa_netmask.is_null() {
                None
            } else {
                unsafe { address_bytes(item.ifa_netmask) }?.filter(|m| m.len() == address.len())
            };
            let value = ObservedAddress {
                index: label.as_ref().and_then(|n| crate::fixture::index(n).ok()),
                name: label,
                address,
                prefix: mask.as_ref().map(|v| prefix(v)).transpose()?,
            };
            if !entries.insert(value) {
                return Err(ProbeError::Malformed);
            }
            if entries.len() > MAX_ADDRESSES {
                return Err(ProbeError::TooLarge);
            }
        }
        at = item.ifa_next;
    }
    Ok(entries.into_iter().collect())
}

fn index_name(index: u32) -> Option<String> {
    let mut bytes = [0 as libc::c_char; libc::IFNAMSIZ];
    if unsafe { libc::if_indextoname(index, bytes.as_mut_ptr()) }.is_null() {
        None
    } else {
        name(&bytes).ok()
    }
}
struct AddressDump {
    wire: crate::netlink_wire::Dump,
    entries: BTreeSet<ObservedAddress>,
}
impl Default for AddressDump {
    fn default() -> Self {
        Self {
            wire: crate::netlink_wire::Dump::new(1, 20),
            entries: BTreeSet::new(),
        }
    }
}
impl AddressDump {
    fn consume(&mut self, bytes: &[u8]) -> Result<bool, ProbeError> {
        let entries = &mut self.entries;
        self.wire.consume(bytes, |body| {
            if body.len() < 8 {
                return Err(ProbeError::Malformed);
            }
            let length = match body[0] as i32 {
                libc::AF_INET => 4,
                libc::AF_INET6 => 16,
                _ => return Ok(()),
            };
            let index = crate::netlink_wire::u32_at(body, 4)?;
            if index == 0 || index > i32::MAX as u32 || body[1] as usize > length * 8 {
                return Err(ProbeError::Malformed);
            }
            let (mut local, mut address, mut name) = (None, None, None);
            for attribute in crate::netlink_wire::attributes(&body[8..])? {
                match attribute.kind {
                    1 | 2 => {
                        if attribute.value.len() != length {
                            return Err(ProbeError::Malformed);
                        }
                        let field = if attribute.kind == 1 {
                            &mut address
                        } else {
                            &mut local
                        };
                        if field.replace(attribute.value.to_vec()).is_some() {
                            return Err(ProbeError::Malformed);
                        }
                    }
                    3 => {
                        let label =
                            crate::netlink_wire::interface_name(attribute.value)?.to_owned();
                        if name.replace(label).is_some() {
                            return Err(ProbeError::Malformed);
                        }
                    }
                    _ => {}
                }
            }
            let address = local.or(address).ok_or(ProbeError::Malformed)?;
            let value = ObservedAddress {
                name: name.or_else(|| index_name(index)),
                index: Some(index),
                address,
                prefix: Some(body[1]),
            };
            if !entries.insert(value) {
                return Err(ProbeError::Malformed);
            }
            if entries.len() > MAX_ADDRESSES {
                return Err(ProbeError::TooLarge);
            }
            Ok(())
        })
    }
}
fn netlink(actor: NativeReadActor) -> Result<Vec<ObservedAddress>, ProbeError> {
    let mut dump = AddressDump::default();
    crate::netlink_read::dump(crate::netlink_read::DumpKind::Addresses, actor, |bytes| {
        dump.consume(bytes)
    })?;
    Ok(dump.entries.into_iter().collect())
}

pub fn observe(name: &str) -> AddressMeasurement {
    AddressMeasurement {
        actors: NativeReadActor::ALL
            .into_iter()
            .map(|actor| AddressActorMeasurement {
                actor,
                ioctls: AddressIoctl::ALL
                    .into_iter()
                    .map(|kind| AddressIoctlObservation {
                        kind,
                        fixture: sample(ioctl_address(name, kind, actor)),
                        ordinary: sample(ioctl_address("lo", kind, actor)),
                    })
                    .collect(),
                ifconf: sample(ifconf(actor)),
                netlink: sample(netlink(actor)),
            })
            .collect(),
        ifaddrs: sample(ifaddrs()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(kind: u16, body: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0; 16];
        bytes[4..6].copy_from_slice(&kind.to_ne_bytes());
        bytes[8..12].copy_from_slice(&1u32.to_ne_bytes());
        bytes.extend_from_slice(body);
        let length = bytes.len() as u32;
        bytes[..4].copy_from_slice(&length.to_ne_bytes());
        bytes.resize((bytes.len() + 3) & !3, 0);
        bytes
    }
    fn attribute(kind: u16, value: &[u8]) -> Vec<u8> {
        let mut bytes = ((value.len() + 4) as u16).to_ne_bytes().to_vec();
        bytes.extend_from_slice(&kind.to_ne_bytes());
        bytes.extend_from_slice(value);
        bytes.resize((bytes.len() + 3) & !3, 0);
        bytes
    }
    #[test]
    fn address_dump_rejects_bad_lengths_prefixes_and_duplicate_local_fields() {
        let mut body = vec![libc::AF_INET as u8, 32, 0, 0];
        body.extend_from_slice(&42u32.to_ne_bytes());
        body.extend(attribute(2, &[198, 18, 0, 1]));
        body.extend(attribute(3, b"tun000000000001\0"));
        let mut dump = AddressDump::default();
        assert!(!dump.consume(&frame(20, &body)).unwrap());
        assert!(dump.consume(&frame(3, &[])).unwrap());
        assert_eq!(dump.entries.iter().next().unwrap().address, [198, 18, 0, 1]);
        for bad in [
            {
                let mut b = body.clone();
                b[1] = 33;
                b
            },
            {
                let mut b = body.clone();
                b[4..8].fill(0);
                b
            },
            {
                let mut b = body.clone();
                b.extend(attribute(2, &[198, 18, 0, 1]));
                b
            },
        ] {
            assert!(AddressDump::default().consume(&frame(20, &bad)).is_err());
        }
        assert_eq!(prefix(&[255, 255, 128, 0]), Ok(17));
        assert!(prefix(&[255, 0, 128, 0]).is_err());
    }
    #[test]
    fn real_host_loopback_is_observed_by_libc_and_direct_syscalls() {
        let observation = observe("tunabcdef012345");
        assert!(observation.bounded());
        assert!(observation.ifaddrs.value().is_some_and(|entries| {
            entries
                .iter()
                .any(|a| a.name.as_deref() == Some("lo") && a.address == [127, 0, 0, 1])
        }));
        for actor in &observation.actors {
            assert_eq!(
                actor
                    .ioctls
                    .iter()
                    .find(|v| v.kind == AddressIoctl::Address)
                    .unwrap()
                    .ordinary
                    .value()
                    .map(Vec::as_slice),
                Some([127, 0, 0, 1].as_slice())
            );
            assert!(actor.ifconf.value().is_some_and(|v| {
                v.entries
                    .iter()
                    .any(|a| a.name.as_deref() == Some("lo") && a.address == [127, 0, 0, 1])
            }));
            assert!(actor.netlink.value().is_some_and(|v| {
                v.iter()
                    .any(|a| a.name.as_deref() == Some("lo") && a.address == [127, 0, 0, 1])
            }));
        }
    }
}
