//! Exact-object native observations for Core-issued fixtures. No interface,
//! address, route or UID policy is created/changed by this unprivileged adapter.

use std::collections::BTreeSet;
use std::ffi::CString;
use std::io;
use std::mem::zeroed;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use vpnhide_diagnostic_model::{
    IfaddrMetadata, MAX_INTERFACES, NativeMeasurement, ObservedInterface, ProbeError, Sample,
};

fn errno() -> ProbeError {
    ProbeError::Errno(
        io::Error::last_os_error()
            .raw_os_error()
            .filter(|e| *e > 0)
            .unwrap_or(libc::EIO),
    )
}

fn socket(family: i32, kind: i32, protocol: i32) -> Result<OwnedFd, ProbeError> {
    let raw = unsafe { libc::socket(family, kind | libc::SOCK_CLOEXEC, protocol) };
    if raw < 0 {
        return Err(errno());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

pub fn interface_ioctl(name: &str, mtu: bool) -> Sample<u32> {
    if name.is_empty()
        || name.len() >= libc::IFNAMSIZ
        || !name.is_ascii()
        || name.as_bytes().contains(&0)
    {
        return Sample::Error(ProbeError::Malformed);
    }
    let fd = match socket(libc::AF_INET, libc::SOCK_DGRAM, 0) {
        Ok(fd) => fd,
        Err(e) => return Sample::Error(e),
    };
    let mut request: libc::ifreq = unsafe { zeroed() };
    for (target, byte) in request.ifr_name.iter_mut().zip(name.bytes()) {
        *target = byte as libc::c_char;
    }
    let command = if mtu {
        libc::SIOCGIFMTU
    } else {
        libc::SIOCGIFFLAGS
    };
    if unsafe { libc::ioctl(fd.as_raw_fd(), command as _, &mut request) } != 0 {
        return Sample::Error(errno());
    }
    if mtu {
        let value = unsafe { request.ifr_ifru.ifru_mtu };
        match u32::try_from(value) {
            Ok(value) if value > 0 => Sample::Value(value),
            _ => Sample::Error(ProbeError::Malformed),
        }
    } else {
        Sample::Value(u32::from(u16::from_ne_bytes(
            unsafe { request.ifr_ifru.ifru_flags }.to_ne_bytes(),
        )))
    }
}

fn ifaddrs() -> Result<Vec<ObservedInterface>, ProbeError> {
    let mut raw = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut raw) } != 0 {
        return Err(errno());
    }
    struct List(*mut libc::ifaddrs);
    impl Drop for List {
        fn drop(&mut self) {
            unsafe {
                libc::freeifaddrs(self.0);
            }
        }
    }
    let _owner = List(raw);
    let mut visited = BTreeSet::new();
    let mut entries = std::collections::BTreeMap::new();
    let mut current = raw;
    while !current.is_null() {
        if visited.len() >= MAX_INTERFACES * 8 || !visited.insert(current as usize) {
            return Err(ProbeError::TooLarge);
        }
        let entry = unsafe { &*current };
        if entry.ifa_name.is_null() {
            return Err(ProbeError::Malformed);
        }
        let length = unsafe { libc::strnlen(entry.ifa_name, libc::IFNAMSIZ) };
        if length == 0 || length >= libc::IFNAMSIZ {
            return Err(ProbeError::Malformed);
        }
        let name = std::str::from_utf8(unsafe {
            std::slice::from_raw_parts(entry.ifa_name.cast::<u8>(), length)
        })
        .map_err(|_| ProbeError::Malformed)?;
        let index = if !entry.ifa_addr.is_null()
            && unsafe { (*entry.ifa_addr).sa_family as i32 } == libc::AF_PACKET
        {
            let link = unsafe { &*entry.ifa_addr.cast::<libc::sockaddr_ll>() };
            u32::try_from(link.sll_ifindex)
                .ok()
                .filter(|v| *v > 0)
                .map(Some)
                .ok_or(ProbeError::Malformed)?
        } else {
            // A DOWN TUN may have no link-layer address. Its enumerated name is
            // still evidence, even if another hook conceals the by-name index.
            index(name).ok()
        };
        let observed = entries
            .entry(name.to_owned())
            .or_insert_with(|| ObservedInterface {
                name: name.into(),
                index,
                ifaddrs: Some(IfaddrMetadata::default()),
            });
        match (observed.index, index) {
            (Some(old), Some(new)) if old != new => return Err(ProbeError::Malformed),
            (None, Some(_)) => observed.index = index,
            _ => {}
        }
        let metadata = observed.ifaddrs.as_mut().expect("ifaddrs source record");
        if entry.ifa_addr.is_null() {
            metadata.has_null_address = true;
        } else {
            let family = unsafe { (*entry.ifa_addr).sa_family };
            if !metadata.families.contains(&family) {
                metadata.families.push(family);
                metadata.families.sort_unstable();
            }
        }
        if !metadata.flags.contains(&entry.ifa_flags) {
            metadata.flags.push(entry.ifa_flags);
            metadata.flags.sort_unstable();
        }
        if entries.len() > MAX_INTERFACES {
            return Err(ProbeError::TooLarge);
        }
        current = entry.ifa_next;
    }
    Ok(entries.into_values().collect())
}

struct LinkDump {
    entries: std::collections::BTreeMap<u32, String>,
    wire: crate::netlink_wire::Dump,
}
impl Default for LinkDump {
    fn default() -> Self {
        Self {
            entries: Default::default(),
            wire: crate::netlink_wire::Dump::new(1, 16),
        }
    }
}
impl LinkDump {
    fn consume(&mut self, batch: &[u8]) -> Result<bool, ProbeError> {
        let entries = &mut self.entries;
        self.wire.consume(batch, |body| {
            if body.len() < 16 {
                return Err(ProbeError::Malformed);
            }
            let index = crate::netlink_wire::u32_at(body, 4)?;
            if index == 0 || index > i32::MAX as u32 {
                return Err(ProbeError::Malformed);
            }
            let mut name = None;
            for attr in crate::netlink_wire::attributes(&body[16..])? {
                if attr.kind == 3 {
                    if name.is_some() {
                        return Err(ProbeError::Malformed);
                    }
                    name = Some(crate::netlink_wire::interface_name(attr.value)?.to_owned());
                }
            }
            let name = name.ok_or(ProbeError::Malformed)?;
            if entries.contains_key(&index) || entries.values().any(|n| *n == name) {
                return Err(ProbeError::Malformed);
            }
            entries.insert(index, name);
            if entries.len() > MAX_INTERFACES {
                return Err(ProbeError::TooLarge);
            }
            Ok(())
        })
    }
}

fn links() -> Result<Vec<ObservedInterface>, ProbeError> {
    let mut dump = LinkDump::default();
    crate::netlink_read::dump(
        crate::netlink_read::DumpKind::Links,
        vpnhide_diagnostic_model::NativeReadActor::Libc,
        |batch| dump.consume(batch),
    )?;
    Ok(dump
        .entries
        .into_iter()
        .map(|(index, name)| ObservedInterface {
            name,
            index: Some(index),
            ifaddrs: None,
        })
        .collect())
}

pub fn observe(name: &str) -> NativeMeasurement {
    let sample = |result: Result<Vec<ObservedInterface>, ProbeError>| match result {
        Ok(value) => Sample::Value(value),
        Err(e) => Sample::Error(e),
    };
    NativeMeasurement {
        filesystem: Some(crate::filesystem::observe(name)),
        sys_interfaces: Some(crate::sys_interfaces::observe()),
        proc_interfaces: Some(crate::proc_interfaces::observe()),
        addresses: Some(crate::address::observe(name)),
        binding: Some(crate::binding::observe(name)),
        flags: interface_ioctl(name, false),
        mtu: interface_ioctl(name, true),
        ifaddrs: sample(ifaddrs()),
        links: sample(links()),
        ordinary_flags: interface_ioctl("lo", false),
        ordinary_mtu: interface_ioctl("lo", true),
    }
}

pub fn index(name: &str) -> Result<u32, ProbeError> {
    if name.is_empty() || name.len() >= libc::IFNAMSIZ || !name.is_ascii() {
        return Err(ProbeError::Malformed);
    }
    let name = CString::new(name).map_err(|_| ProbeError::Malformed)?;
    let index = unsafe { libc::if_nametoindex(name.as_ptr()) };
    if index == 0 { Err(errno()) } else { Ok(index) }
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
    fn link(index: u32, name: &str) -> Vec<u8> {
        let mut body = vec![0; 16];
        body[4..8].copy_from_slice(&index.to_ne_bytes());
        let length = 4 + name.len() + 1;
        body.extend_from_slice(&(length as u16).to_ne_bytes());
        body.extend_from_slice(&3u16.to_ne_bytes());
        body.extend_from_slice(name.as_bytes());
        body.push(0);
        body.resize((body.len() + 3) & !3, 0);
        frame(16, &body)
    }
    fn done() -> Vec<u8> {
        frame(3, &0u32.to_ne_bytes())
    }

    #[test]
    fn exact_dump_keeps_down_addressless_link_and_requires_done() {
        let mut dump = LinkDump::default();
        assert!(!dump.consume(&link(1, "lo")).unwrap());
        assert!(!dump.consume(&link(42, "tun000000000001")).unwrap());
        assert!(dump.consume(&done()).unwrap());
        assert_eq!(
            dump.entries.get(&42).map(String::as_str),
            Some("tun000000000001")
        );
        assert!(dump.consume(&done()).is_err());
    }

    #[test]
    fn done_cannot_hide_trailing_messages_or_errors() {
        for trailing in [link(42, "tun000000000001"), done(), vec![1]] {
            let batch = [link(1, "lo"), done(), trailing].concat();
            assert_eq!(
                LinkDump::default().consume(&batch),
                Err(ProbeError::Malformed)
            );
        }
        assert!(
            LinkDump::default()
                .consume(&frame(3, &(-12i32).to_ne_bytes()))
                .is_err()
        );
    }

    #[test]
    fn malformed_and_conflicting_identity_frames_never_become_empty_success() {
        for other in [link(2, "lo"), link(1, "eth0"), link(1, "lo")] {
            assert!(
                LinkDump::default()
                    .consume(&[link(1, "lo"), other, done()].concat())
                    .is_err()
            );
        }
        let valid = link(1, "lo");
        for size in 0..valid.len() {
            assert!(LinkDump::default().consume(&valid[..size]).is_err());
        }
        let mut bad_attr = valid.clone();
        bad_attr[32..34].copy_from_slice(&3u16.to_ne_bytes());
        let mut wrong_seq = valid.clone();
        wrong_seq[8..12].copy_from_slice(&2u32.to_ne_bytes());
        let mut interrupted = valid.clone();
        interrupted[6..8].copy_from_slice(&0x10u16.to_ne_bytes());
        for bad in [
            bad_attr,
            wrong_seq,
            interrupted,
            link(0, "lo"),
            link(u32::MAX, "lo"),
            link(3, "bad\0tail"),
        ] {
            assert!(LinkDump::default().consume(&bad).is_err());
        }
    }
}
