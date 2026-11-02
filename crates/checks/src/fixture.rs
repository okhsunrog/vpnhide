//! Exact-object native observations for Core-issued fixtures. No interface,
//! address, route or UID policy is created/changed by this unprivileged adapter.

use std::collections::BTreeSet;
use std::ffi::CString;
use std::io;
use std::mem::zeroed;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};
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

fn u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_ne_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}
fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_ne_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

#[derive(Default)]
struct LinkDump {
    entries: std::collections::BTreeMap<u32, String>,
    done: bool,
}

impl LinkDump {
    fn consume(&mut self, batch: &[u8]) -> Result<bool, ProbeError> {
        if self.done || batch.is_empty() {
            return Err(ProbeError::Malformed);
        }
        let mut offset = 0;
        while offset < batch.len() {
            let size = u32_at(batch, offset).ok_or(ProbeError::Malformed)? as usize;
            if size < 16 || size > batch.len() - offset || u32_at(batch, offset + 8) != Some(1) {
                return Err(ProbeError::Malformed);
            }
            let aligned = (size + 3) & !3;
            if aligned > batch.len() - offset {
                return Err(ProbeError::Malformed);
            }
            let frame = &batch[offset..offset + size];
            if u16_at(frame, 6).ok_or(ProbeError::Malformed)? & 0x10 != 0 {
                return Err(ProbeError::Malformed);
            }
            match u16_at(frame, 4).ok_or(ProbeError::Malformed)? {
                3 => {
                    if !matches!(size, 16 | 20)
                        || (size == 20 && u32_at(frame, 16) != Some(0))
                        || offset + aligned != batch.len()
                    {
                        return Err(ProbeError::Malformed);
                    }
                    self.done = true;
                }
                2 => {
                    return Err(ProbeError::Errno(
                        u32_at(frame, 16)
                            .map(|v| i32::from_ne_bytes(v.to_ne_bytes()).saturating_neg())
                            .filter(|e| *e > 0)
                            .unwrap_or(libc::EIO),
                    ));
                }
                16 => {
                    let index = u32_at(frame, 20)
                        .filter(|i| *i > 0 && *i <= i32::MAX as u32)
                        .ok_or(ProbeError::Malformed)?;
                    let mut at = 32;
                    let mut name = None;
                    while at < frame.len() {
                        let length = u16_at(frame, at).ok_or(ProbeError::Malformed)? as usize;
                        if length < 4 || length > frame.len() - at {
                            return Err(ProbeError::Malformed);
                        }
                        if u16_at(frame, at + 2) == Some(3) {
                            if name.is_some() {
                                return Err(ProbeError::Malformed);
                            }
                            let value = &frame[at + 4..at + length];
                            let end = value
                                .iter()
                                .position(|b| *b == 0)
                                .filter(|p| *p > 0 && *p < libc::IFNAMSIZ)
                                .ok_or(ProbeError::Malformed)?;
                            if end + 1 != value.len() {
                                return Err(ProbeError::Malformed);
                            }
                            name = Some(
                                std::str::from_utf8(&value[..end])
                                    .map_err(|_| ProbeError::Malformed)?
                                    .to_owned(),
                            );
                        }
                        at += (length + 3) & !3;
                    }
                    if at != frame.len() {
                        return Err(ProbeError::Malformed);
                    }
                    let name = name.ok_or(ProbeError::Malformed)?;
                    if self.entries.contains_key(&index)
                        || self.entries.values().any(|n| *n == name)
                    {
                        return Err(ProbeError::Malformed);
                    }
                    self.entries.insert(index, name);
                    if self.entries.len() > MAX_INTERFACES {
                        return Err(ProbeError::TooLarge);
                    }
                }
                _ => return Err(ProbeError::Malformed),
            }
            offset += aligned;
        }
        Ok(self.done)
    }
}

fn links() -> Result<Vec<ObservedInterface>, ProbeError> {
    let fd = socket(
        libc::AF_NETLINK,
        libc::SOCK_RAW | libc::SOCK_NONBLOCK,
        libc::NETLINK_ROUTE,
    )?;
    let mut address: libc::sockaddr_nl = unsafe { zeroed() };
    address.nl_family = libc::AF_NETLINK as libc::sa_family_t;
    if unsafe {
        libc::bind(
            fd.as_raw_fd(),
            (&address as *const libc::sockaddr_nl).cast(),
            size_of::<libc::sockaddr_nl>() as _,
        )
    } != 0
    {
        return Err(errno());
    }
    let mut request = [0u8; 32];
    request[..4].copy_from_slice(&32u32.to_ne_bytes());
    request[4..6].copy_from_slice(&18u16.to_ne_bytes());
    request[6..8].copy_from_slice(&0x301u16.to_ne_bytes());
    request[8..12].copy_from_slice(&1u32.to_ne_bytes());
    if unsafe {
        libc::sendto(
            fd.as_raw_fd(),
            request.as_ptr().cast(),
            request.len(),
            0,
            (&address as *const libc::sockaddr_nl).cast(),
            size_of::<libc::sockaddr_nl>() as _,
        )
    } != request.len() as isize
    {
        return Err(errno());
    }
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut dump = LinkDump::default();
    let mut total = 0usize;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or(ProbeError::Timeout)?;
        let mut poll = libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ready =
            unsafe { libc::poll(&mut poll, 1, remaining.as_millis().clamp(1, 1000) as i32) };
        if ready < 0 {
            return Err(errno());
        }
        if ready == 0 {
            return Err(ProbeError::Timeout);
        }
        let mut sender: libc::sockaddr_nl = unsafe { zeroed() };
        let mut iov = libc::iovec {
            iov_base: buffer.as_mut_ptr().cast(),
            iov_len: buffer.len(),
        };
        let mut message: libc::msghdr = unsafe { zeroed() };
        message.msg_name = (&mut sender as *mut libc::sockaddr_nl).cast();
        message.msg_namelen = size_of::<libc::sockaddr_nl>() as _;
        message.msg_iov = &mut iov;
        message.msg_iovlen = 1;
        let size = unsafe { libc::recvmsg(fd.as_raw_fd(), &mut message, 0) };
        if size < 0 {
            return Err(errno());
        }
        if sender.nl_pid != 0
            || sender.nl_family != libc::AF_NETLINK as libc::sa_family_t
            || message.msg_namelen as usize != size_of::<libc::sockaddr_nl>()
            || message.msg_flags & libc::MSG_TRUNC != 0
        {
            return Err(ProbeError::Malformed);
        }
        total += size as usize;
        if total > 1024 * 1024 {
            return Err(ProbeError::TooLarge);
        }
        if dump.consume(&buffer[..size as usize])? {
            return Ok(dump
                .entries
                .into_iter()
                .map(|(index, name)| ObservedInterface {
                    name,
                    index: Some(index),
                    ifaddrs: None,
                })
                .collect());
        }
    }
}

pub fn observe(name: &str) -> NativeMeasurement {
    let sample = |result: Result<Vec<ObservedInterface>, ProbeError>| match result {
        Ok(value) => Sample::Value(value),
        Err(e) => Sample::Error(e),
    };
    NativeMeasurement {
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
