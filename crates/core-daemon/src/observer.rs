//! The root observation loop: `NETLINK_ROUTE` + framework VPN classification (§8).
//!
//! Netlink answers "did routing/link state change" and supplies name-match **candidates**.
//! Framework evidence (`TRANSPORT_VPN` via dumpsys / env stub) classifies VPN. Freshness is
//! Current only with both; otherwise Unknown with last-good retained for display.
//!
//! Events are change-notifications, not a log. Debounce coalesces bursts; ENOBUFS → mark
//! Unknown then resync. Backend identity is refreshed on a bounded interval (not only at
//! startup/apply).

use std::collections::BTreeSet;
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::Interest;
use tokio::io::unix::AsyncFd;
use tokio::time::Instant;
use vpnhide_checks::is_vpn_interface_name;
use crate::framework_vpn;
use crate::routing_compose;
use crate::server::CoreState;

const DEBOUNCE: Duration = Duration::from_millis(250);
const RECV_BUF: usize = 64 * 1024;

// --- netlink ABI (stable kernel numbers; defined here to avoid per-target libc gaps) ---
const RTM_NEWLINK: u16 = 16;
const RTM_DELLINK: u16 = 17;
const RTM_GETLINK: u16 = 18;
const NLMSG_DONE: u16 = 3;
const NLM_F_REQUEST: u16 = 0x001;
const NLM_F_DUMP: u16 = 0x100 | 0x200; // ROOT | MATCH
const RTNLGRP_LINK: u32 = 1;
const IFLA_IFNAME: u16 = 3;
const IFF_UP: u32 = 0x1;
const NLMSG_ALIGNTO: usize = 4;
const RTA_ALIGNTO: usize = 4;
const NLMSG_HDRLEN: usize = 16; // align(sizeof(nlmsghdr)=16, 4)

const fn align(len: usize, to: usize) -> usize {
    (len + to - 1) & !(to - 1)
}

fn read_unaligned<T: Copy>(bytes: &[u8]) -> Option<T> {
    if bytes.len() < size_of::<T>() {
        return None;
    }
    // SAFETY: length checked; read_unaligned tolerates any alignment.
    Some(unsafe { std::ptr::read_unaligned(bytes.as_ptr() as *const T) })
}

fn for_each_message(buf: &[u8], mut f: impl FnMut(u16, &[u8])) {
    let mut offset = 0usize;
    while buf.len() - offset >= NLMSG_HDRLEN {
        let hdr = &buf[offset..];
        let len = read_unaligned::<u32>(&hdr[0..4]).unwrap() as usize;
        let msg_type = read_unaligned::<u16>(&hdr[4..6]).unwrap();
        if len < NLMSG_HDRLEN || offset + len > buf.len() {
            break;
        }
        f(msg_type, &buf[offset + NLMSG_HDRLEN..offset + len]);
        offset += align(len, NLMSG_ALIGNTO);
    }
}

fn find_attr(payload: &[u8], body_offset: usize, want: u16) -> Option<&[u8]> {
    let mut offset = align(body_offset, RTA_ALIGNTO);
    while payload.len().saturating_sub(offset) >= 4 {
        let rta = &payload[offset..];
        let rta_len = read_unaligned::<u16>(&rta[0..2]).unwrap() as usize;
        let rta_type = read_unaligned::<u16>(&rta[2..4]).unwrap();
        if rta_len < 4 || offset + rta_len > payload.len() {
            break;
        }
        if rta_type == want {
            return Some(&payload[offset + 4..offset + rta_len]);
        }
        offset += align(rta_len, RTA_ALIGNTO);
    }
    None
}

/// (name, administratively up) from an RTM_*LINK message. The index is not tracked here —
/// presence keys on the name, which is what the VPN-candidate set is expressed in.
fn parse_link(payload: &[u8]) -> Option<(String, bool)> {
    // struct ifinfomsg { u8 family; u8 pad; u16 type; i32 index; u32 flags; u32 change; }
    let flags = read_unaligned::<u32>(payload.get(8..12)?)?;
    let name_bytes = find_attr(payload, 16, IFLA_IFNAME)?;
    let name = name_bytes
        .split(|&b| b == 0)
        .next()
        .and_then(|b| std::str::from_utf8(b).ok())?
        .to_owned();
    (!name.is_empty()).then_some((name, flags & IFF_UP != 0))
}

/// A raw `NETLINK_ROUTE` socket in non-blocking mode, subscribed to link events.
struct NetlinkRoute {
    fd: RawFd,
}

impl NetlinkRoute {
    fn open() -> io::Result<Self> {
        // SAFETY: socket(2) with constant args.
        let fd = unsafe {
            libc::socket(
                libc::AF_NETLINK,
                libc::SOCK_RAW | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
                libc::NETLINK_ROUTE,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let sock = NetlinkRoute { fd };

        let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        addr.nl_family = libc::AF_NETLINK as u16;
        let rc = unsafe {
            libc::bind(
                fd,
                &addr as *const _ as *const libc::sockaddr,
                size_of::<libc::sockaddr_nl>() as libc::socklen_t,
            )
        };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        let group = RTNLGRP_LINK;
        let rc = unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_NETLINK,
                libc::NETLINK_ADD_MEMBERSHIP,
                &group as *const _ as *const libc::c_void,
                size_of::<u32>() as libc::socklen_t,
            )
        };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(sock)
    }

    fn request_link_dump(&self) -> io::Result<()> {
        let mut req = [0u8; 32]; // nlmsghdr(16) + ifinfomsg(16), family AF_UNSPEC
        let total = req.len() as u32;
        req[0..4].copy_from_slice(&total.to_ne_bytes());
        req[4..6].copy_from_slice(&RTM_GETLINK.to_ne_bytes());
        req[6..8].copy_from_slice(&(NLM_F_REQUEST | NLM_F_DUMP).to_ne_bytes());
        req[8..12].copy_from_slice(&1u32.to_ne_bytes());
        let sent =
            unsafe { libc::send(self.fd, req.as_ptr() as *const libc::c_void, req.len(), 0) };
        if sent < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// One non-blocking recv. `WouldBlock` when drained; `ENOBUFS` on overflow.
    fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        let n = unsafe { libc::recv(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0) };
        if n < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(n as usize)
        }
    }
}

impl AsRawFd for NetlinkRoute {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}

impl Drop for NetlinkRoute {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd) };
    }
}

/// The set of VPN-candidate interfaces currently up.
#[derive(Default)]
struct RouteState {
    up: BTreeSet<String>,
}

impl RouteState {
    fn apply(&mut self, msg_type: u16, name: String, up: bool) {
        if !is_vpn_interface_name(&name) {
            return;
        }
        if msg_type == RTM_DELLINK || !up {
            self.up.remove(&name);
        } else {
            self.up.insert(name);
        }
    }
}

/// Feed one recv batch through the route state. Returns true if the VPN set changed.
fn consume(batch: &[u8], state: &mut RouteState) -> bool {
    let before = state.up.clone();
    for_each_message(batch, |msg_type, payload| {
        if msg_type == RTM_NEWLINK || msg_type == RTM_DELLINK {
            if let Some((name, up)) = parse_link(payload) {
                state.apply(msg_type, name, up);
            }
        }
    });
    state.up != before
}

fn next_generation(state: &Arc<CoreState>) -> u64 {
    state.publisher.current().snapshot.routing.generation.saturating_add(1)
}

/// Compose netlink + framework and publish when the observation changed.
fn publish(state: &Arc<CoreState>, route: &RouteState) {
    let previous = state.publisher.current().snapshot.routing.clone();
    let framework = framework_vpn::classify();
    let next = routing_compose::compose(
        &previous,
        next_generation(state),
        &route.up,
        framework,
        routing_compose::observe_uid_from_env(),
    );
    if previous != next {
        state
            .publisher
            .update(move |snapshot| snapshot.routing = next);
    }
}

fn publish_unknown(state: &Arc<CoreState>, detail: &str) {
    let previous = state.publisher.current().snapshot.routing.clone();
    let next = routing_compose::mark_unknown(&previous, next_generation(state), detail);
    if previous != next {
        state
            .publisher
            .update(move |snapshot| snapshot.routing = next);
    }
}

fn refresh_backend(state: &Arc<CoreState>) {
    use std::path::Path;
    use crate::{activation, backend};
    // Lifecycle refresh must **not** take the KPM management lock — that races
    // Native apply (EAGAIN). Kernel ctl + installed dirs are lock-free; KPM
    // active identity is refreshed at startup/apply via detect_managed.
    let ctl = activation::kmod_ctl_path_from_env()
        .unwrap_or_else(|| Path::new("/proc/vpnhide_ctl").to_path_buf());
    let modules = backend::kernel_modules_from_env();
    let detected = backend::detect(
        &ctl,
        modules
            .as_deref()
            .unwrap_or_else(|| Path::new(backend::MODULES_ROOT)),
    );
    state.publisher.update(|snapshot| {
        let mut next = detected;
        next.selected = snapshot.backend.selected;
        next.applied = snapshot.backend.applied;
        // Keep prior KPM-active evidence if lock-free detect cannot see it.
        if next.active.is_none() && snapshot.backend.active == Some(vpnhide_core_rpc::NativeBackend::Kpm)
        {
            next.active = snapshot.backend.active;
        }
        if snapshot.backend != next {
            snapshot.backend = next;
        }
    });
}

/// Rebuild the route state from a fresh kernel dump (startup and after an ENOBUFS overflow).
async fn resync(
    async_fd: &AsyncFd<NetlinkRoute>,
    buf: &mut [u8],
    state: &mut RouteState,
) -> io::Result<()> {
    *state = RouteState::default();
    async_fd.get_ref().request_link_dump()?;
    loop {
        let mut guard = async_fd.readable().await?;
        match guard.try_io(|inner| inner.get_ref().recv(buf)) {
            Ok(Ok(n)) => {
                let mut done = false;
                for_each_message(&buf[..n], |msg_type, payload| match msg_type {
                    RTM_NEWLINK => {
                        if let Some((name, up)) = parse_link(payload) {
                            state.apply(RTM_NEWLINK, name, up);
                        }
                    }
                    NLMSG_DONE => done = true,
                    _ => {}
                });
                if done {
                    return Ok(());
                }
            }
            Ok(Err(e)) => return Err(e),
            Err(_would_block) => continue,
        }
    }
}

/// A deadline far enough out that a `pending`-less debounce arm never fires (it is disabled
/// by its precondition; this only keeps the expression valid — see the select! note below).
fn far_future() -> Instant {
    Instant::now() + Duration::from_secs(3600)
}

/// Run the observation loop, feeding VPN presence into `state`'s publisher, until an
/// unrecoverable error. Requires a running Tokio runtime.
pub async fn observe(state: Arc<CoreState>) -> io::Result<()> {
    let async_fd = AsyncFd::with_interest(NetlinkRoute::open()?, Interest::READABLE)?;
    let mut buf = vec![0u8; RECV_BUF];
    let mut route = RouteState::default();

    resync(&async_fd, &mut buf, &mut route).await?;
    publish(&state, &route);
    refresh_backend(&state);

    let mut pending: Option<Instant> = None;
    let mut backend_deadline = Instant::now() + Duration::from_secs(30);
    loop {
        tokio::select! {
            readable = async_fd.readable() => {
                let mut guard = readable?;
                let mut changed = false;
                let mut overflow = false;
                loop {
                    match guard.try_io(|inner| inner.get_ref().recv(&mut buf)) {
                        Ok(Ok(n)) => changed |= consume(&buf[..n], &mut route),
                        Ok(Err(e)) if e.raw_os_error() == Some(libc::ENOBUFS) => {
                            overflow = true;
                            break;
                        }
                        Ok(Err(e)) => {
                            publish_unknown(&state, &format!("netlink_recv:{e}"));
                            return Err(e);
                        }
                        Err(_would_block) => break,
                    }
                }
                drop(guard);
                if overflow {
                    eprintln!("core: netlink overflow — marking Unknown then resyncing");
                    publish_unknown(&state, "enobufs");
                    resync(&async_fd, &mut buf, &mut route).await?;
                    changed = true;
                }
                if changed {
                    pending = Some(Instant::now() + DEBOUNCE);
                }
            }
            _ = tokio::time::sleep_until(pending.unwrap_or_else(far_future)), if pending.is_some() => {
                publish(&state, &route);
                pending = None;
            }
            _ = tokio::time::sleep_until(backend_deadline) => {
                refresh_backend(&state);
                backend_deadline = Instant::now() + Duration::from_secs(30);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nlmsg(msg_type: u16, body: &[u8]) -> Vec<u8> {
        let len = (NLMSG_HDRLEN + body.len()) as u32;
        let mut m = Vec::new();
        m.extend_from_slice(&len.to_ne_bytes());
        m.extend_from_slice(&msg_type.to_ne_bytes());
        m.extend_from_slice(&0u16.to_ne_bytes());
        m.extend_from_slice(&0u32.to_ne_bytes());
        m.extend_from_slice(&0u32.to_ne_bytes());
        m.extend_from_slice(body);
        while m.len() % NLMSG_ALIGNTO != 0 {
            m.push(0);
        }
        m
    }

    fn link_body(name: &str, up: bool) -> Vec<u8> {
        let mut b = vec![0u8; 16];
        b[8..12].copy_from_slice(&(if up { IFF_UP } else { 0 }).to_ne_bytes());
        let rta_len = (4 + name.len() + 1) as u16;
        b.extend_from_slice(&rta_len.to_ne_bytes());
        b.extend_from_slice(&IFLA_IFNAME.to_ne_bytes());
        b.extend_from_slice(name.as_bytes());
        b.push(0);
        while b.len() % RTA_ALIGNTO != 0 {
            b.push(0);
        }
        b
    }

    #[test]
    fn a_vpn_link_up_then_down_flips_the_set() {
        let mut state = RouteState::default();
        assert!(consume(
            &nlmsg(RTM_NEWLINK, &link_body("tun0", true)),
            &mut state
        ));
        assert_eq!(state.up.iter().cloned().collect::<Vec<_>>(), vec!["tun0"]);
        // A route message (RTM_NEWROUTE = 24), i.e. non-link, does not change presence.
        assert!(!consume(&nlmsg(24, &[0u8; 12]), &mut state));
        assert!(consume(
            &nlmsg(RTM_DELLINK, &link_body("tun0", true)),
            &mut state
        ));
        assert!(state.up.is_empty());
    }

    #[test]
    fn a_non_vpn_link_never_enters_the_set() {
        let mut state = RouteState::default();
        assert!(!consume(
            &nlmsg(RTM_NEWLINK, &link_body("wlan0", true)),
            &mut state
        ));
        assert!(state.up.is_empty());
    }
}
