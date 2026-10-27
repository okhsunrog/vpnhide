//! The root observation loop: `NETLINK_ROUTE` + framework VPN classification.
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

use crate::framework_vpn;
use crate::routing_compose;
use crate::server::CoreState;
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;
use tokio::time::Instant;
use vpnhide_checks::is_vpn_interface_name;

const DEBOUNCE: Duration = Duration::from_millis(250);
const RECV_BUF: usize = 64 * 1024;
// Framework registration can settle after the last kernel notification, and app data
// (hence its UID) can appear without a network event. Reconcile these sources in core.
const RECONCILE_INTERVAL: Duration = Duration::from_secs(5);
const FRAMEWORK_RECONCILE_INTERVAL: Duration = Duration::from_secs(30);

// --- netlink ABI (stable kernel numbers; defined here to avoid per-target libc gaps) ---
const RTM_NEWLINK: u16 = 16;
const RTM_DELLINK: u16 = 17;
const RTM_GETLINK: u16 = 18;
const RTM_NEWROUTE: u16 = 24;
const RTM_DELROUTE: u16 = 25;
const RTM_NEWRULE: u16 = 32;
const RTM_DELRULE: u16 = 33;
const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;
const NLM_F_DUMP_INTR: u16 = 0x10;
const NLM_F_REQUEST: u16 = 0x001;
const NLM_F_DUMP: u16 = 0x100 | 0x200; // ROOT | MATCH
const RTNLGRP_LINK: u32 = 1;
const ROUTING_GROUPS: [u32; 5] = [RTNLGRP_LINK, 7, 8, 11, 19]; // link, v4 route/rule, v6 route/rule
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

fn for_each_message(buf: &[u8], mut f: impl FnMut(u16, u16, u32, &[u8])) {
    let mut offset = 0usize;
    while buf.len().saturating_sub(offset) >= NLMSG_HDRLEN {
        let hdr = &buf[offset..];
        let len = read_unaligned::<u32>(&hdr[0..4]).unwrap() as usize;
        let msg_type = read_unaligned::<u16>(&hdr[4..6]).unwrap();
        let flags = read_unaligned::<u16>(&hdr[6..8]).unwrap();
        let sequence = read_unaligned::<u32>(&hdr[8..12]).unwrap();
        if len < NLMSG_HDRLEN || offset + len > buf.len() {
            break;
        }
        f(
            msg_type,
            flags,
            sequence,
            &buf[offset + NLMSG_HDRLEN..offset + len],
        );
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

/// A raw `NETLINK_ROUTE` socket subscribed to link, route and UID-rule events.
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
        for group in ROUTING_GROUPS {
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

/// Feed a recv batch through the link cache. A route/rule event invalidates UID
/// membership even when the tunnel remains up (split-routing changes).
fn consume(batch: &[u8], state: &mut RouteState) -> bool {
    let before = state.up.clone();
    let mut routing_changed = false;
    for_each_message(batch, |msg_type, _, _, payload| {
        routing_changed |= matches!(
            msg_type,
            RTM_NEWROUTE | RTM_DELROUTE | RTM_NEWRULE | RTM_DELRULE
        );
        if msg_type == RTM_NEWLINK || msg_type == RTM_DELLINK {
            if let Some((name, up)) = parse_link(payload) {
                state.apply(msg_type, name, up);
            }
        }
    });
    routing_changed || state.up != before
}

fn next_generation(state: &Arc<CoreState>) -> u64 {
    state
        .publisher
        .current()
        .snapshot
        .routing
        .generation
        .saturating_add(1)
}

/// Compose netlink + framework and publish when the observation changed.
async fn publish(state: &Arc<CoreState>, route: &RouteState) {
    let previous = state.publisher.current().snapshot.routing.clone();
    let last = previous.clone();
    let up = route.up.clone();
    // One observation worker at a time; blocking framework/netlink reads never run on
    // the async socket server's executor. No replacement worker races an overdue one.
    let result = tokio::task::spawn_blocking(move || {
        let (framework, uid) = framework_vpn::sample(routing_compose::observe_uid_from_env());
        let mut next = routing_compose::compose(&last, last.generation, &up, framework, None);
        next.uid = uid;
        next
    })
    .await;
    let Ok(mut next) = result else {
        publish_unknown(state, "observation_worker_failed");
        return;
    };
    if previous != next {
        next.generation = previous.generation.saturating_add(1);
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

async fn refresh_backend(state: &Arc<CoreState>) {
    use crate::{activation, backend};
    use std::path::Path;
    // Only this source owns live/installed evidence. Boot decision and delivery
    // receipts remain with their owner. A busy KPM read becomes Unknown rather
    // than preserving an old active identity as current forever.
    let before = state.publisher.current().snapshot.backend;
    let ctl = activation::kmod_ctl_path_from_env()
        .unwrap_or_else(|| Path::new("/proc/vpnhide_ctl").to_path_buf());
    let modules = backend::kernel_modules_from_env();
    let kpm = backend::kpm_lock_from_env();
    let observed = tokio::task::spawn_blocking(move || {
        backend::detect_managed(
            &ctl,
            modules
                .as_deref()
                .unwrap_or_else(|| Path::new(backend::MODULES_ROOT)),
            kpm.as_deref(),
        )
    })
    .await;
    let Ok(observed) = observed else {
        return;
    };
    state.publisher.update_if_changed(|snapshot| {
        // A delivery/boot transition after the read started supersedes this sample.
        if snapshot.backend.boot != before.boot
            || snapshot.backend.selected != before.selected
            || snapshot.backend.applied != before.applied
        {
            return false;
        }
        merge_backend_observation(&mut snapshot.backend, observed)
    });
}

fn merge_backend_observation(
    current: &mut vpnhide_core_rpc::NativeBackendStatus,
    observed: vpnhide_core_rpc::NativeBackendStatus,
) -> bool {
    let changed = current.active != observed.active
        || current.installed != observed.installed
        || current.observed_freshness != observed.observed_freshness;
    current.active = observed.active;
    current.installed = observed.installed;
    current.observed_freshness = observed.observed_freshness;
    changed
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
                if consume_dump(&buf[..n], state)? {
                    return Ok(());
                }
            }
            Ok(Err(e)) => return Err(e),
            Err(_would_block) => continue,
        }
    }
}

fn consume_dump(batch: &[u8], state: &mut RouteState) -> io::Result<bool> {
    let mut done = false;
    let mut error = None;
    for_each_message(batch, |kind, flags, sequence, payload| {
        // Sequence 1 belongs to our dump; sequence 0 is interleaved multicast.
        if sequence != 0 && sequence != 1 {
            return;
        }
        if sequence == 1 && flags & NLM_F_DUMP_INTR != 0 {
            error = Some(io::Error::other("netlink dump interrupted"));
        }
        match kind {
            RTM_NEWLINK | RTM_DELLINK => {
                if let Some((name, up)) = parse_link(payload) {
                    state.apply(kind, name, up);
                }
            }
            NLMSG_DONE if sequence == 1 => done = true,
            NLMSG_ERROR if sequence == 1 => {
                if let Some(code) = read_unaligned::<i32>(payload).filter(|code| *code != 0) {
                    error = Some(io::Error::from_raw_os_error(code.saturating_neg()));
                }
            }
            _ => {}
        }
    });
    error.map_or(Ok(done), Err)
}

/// A deadline far enough out that a `pending`-less debounce arm never fires (it is disabled
/// by its precondition; this only keeps the expression valid — see the select! note below).
fn far_future() -> Instant {
    Instant::now() + Duration::from_secs(3600)
}

/// Run the observation loop, feeding VPN presence into `state`'s publisher, until an
/// unrecoverable error. Requires a running Tokio runtime.
pub async fn observe(state: Arc<CoreState>) -> io::Result<()> {
    // SAFETY: NetlinkRoute owns the fd and exposes the same descriptor until Drop.
    let async_fd =
        unsafe { AsyncFd::register_with_interest(NetlinkRoute::open()?, Interest::READABLE) }
            .map_err(|error| std::io::Error::other(error.to_string()))?;
    let mut buf = vec![0u8; RECV_BUF];
    let mut route = RouteState::default();

    bounded_resync(&async_fd, &mut buf, &mut route).await?;
    publish(&state, &route).await;
    refresh_backend(&state).await;

    let mut pending: Option<Instant> = None;
    let mut backend_deadline = Instant::now() + Duration::from_secs(30);
    let mut reconcile_deadline = Instant::now() + FRAMEWORK_RECONCILE_INTERVAL;
    let mut uid_deadline = Instant::now() + RECONCILE_INTERVAL;
    let mut focus_uid = routing_compose::observe_uid_from_env();
    let mut settling: Option<(Instant, u8)> = None;
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
                    bounded_resync(&async_fd, &mut buf, &mut route).await?;
                    changed = true;
                }
                if changed {
                    pending.get_or_insert_with(|| Instant::now() + DEBOUNCE);
                }
            }
            _ = tokio::time::sleep_until(pending.unwrap_or_else(far_future)), if pending.is_some() => {
                publish(&state, &route).await;
                pending = None;
                // Android can classify the VPN after its final kernel event.
                // Two bounded follow-ups cover that setup window without a
                // permanent five-second dumpsys loop on an idle device.
                settling = Some((Instant::now() + Duration::from_millis(750), 2));
            }
            _ = tokio::time::sleep_until(settling.map_or_else(far_future, |(at, _)| at)), if settling.is_some() => {
                publish(&state, &route).await;
                let left = settling.map_or(0, |(_, left)| left.saturating_sub(1));
                settling = (left > 0).then(|| (Instant::now() + Duration::from_millis(1500), left));
            }
            _ = tokio::time::sleep_until(reconcile_deadline) => {
                bounded_resync(&async_fd, &mut buf, &mut route).await?;
                publish(&state, &route).await;
                reconcile_deadline = Instant::now() + FRAMEWORK_RECONCILE_INTERVAL;
            }
            _ = tokio::time::sleep_until(uid_deadline) => {
                let uid = routing_compose::observe_uid_from_env();
                if uid != focus_uid {
                    focus_uid = uid;
                    publish(&state, &route).await;
                }
                uid_deadline = Instant::now() + RECONCILE_INTERVAL;
            }
            _ = tokio::time::sleep_until(backend_deadline) => {
                refresh_backend(&state).await;
                backend_deadline = Instant::now() + Duration::from_secs(30);
            }
        }
    }
}

async fn bounded_resync(
    socket: &AsyncFd<NetlinkRoute>,
    buf: &mut [u8],
    route: &mut RouteState,
) -> io::Result<()> {
    tokio::time::timeout(Duration::from_secs(3), resync(socket, buf, route))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "netlink dump timed out"))?
}

/// Re-open a failed event source, retaining last-good display data while explicitly
/// withdrawing current evidence. A source failure must not leave stale routing Current.
pub async fn supervise(state: Arc<CoreState>) {
    loop {
        if let Err(error) = observe(state.clone()).await {
            publish_unknown(&state, &format!("observer:{error}"));
            eprintln!("core: observation unavailable: {error}");
        }
        tokio::time::sleep(RECONCILE_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_observation_preserves_boot_and_delivery_evidence() {
        use vpnhide_core_rpc::{
            NativeBackend, NativeBackendStatus, NativeBootOutcome, NativeBootStatus,
            ObservationFreshness, SelectionMode, ZygiskTargets,
        };
        let boot = NativeBootStatus {
            boot_id: "boot-before-observation".into(),
            decided_epoch: 7,
            decided_at_ms: 1,
            updated_at_ms: 2,
            outcome: NativeBootOutcome::Selected {
                backend: NativeBackend::Kpm,
                mode: SelectionMode::Loaded,
                filesystem_hiding: Some(true),
            },
            installed: vec![NativeBackend::Kpm],
            live: vec![NativeBackend::Kpm],
            staged: vec![],
            kmod: None,
            zygisk_targets: ZygiskTargets::Retired,
            ports: None,
            obsolete_modules: vec![],
        };
        let mut current = NativeBackendStatus {
            selected: Some(NativeBackend::Kpm),
            applied: Some(NativeBackend::Kpm),
            active: Some(NativeBackend::Kpm),
            boot: Some(boot.clone()),
            ..Default::default()
        };
        let observed = NativeBackendStatus {
            installed: vec![NativeBackend::Kpm],
            observed_freshness: ObservationFreshness::Unknown,
            ..Default::default()
        };
        assert!(merge_backend_observation(&mut current, observed.clone()));
        assert_eq!(current.boot, Some(boot));
        assert_eq!(current.selected, Some(NativeBackend::Kpm));
        assert_eq!(current.applied, Some(NativeBackend::Kpm));
        assert_eq!(current.active, None);
        assert_eq!(current.observed_freshness, ObservationFreshness::Unknown);
        assert!(!merge_backend_observation(&mut current, observed));
    }

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
        // Routes leave link presence unchanged but must trigger a membership read.
        assert!(consume(&nlmsg(RTM_NEWROUTE, &[0u8; 12]), &mut state));
        assert_eq!(state.up.iter().cloned().collect::<Vec<_>>(), vec!["tun0"]);
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

    #[test]
    fn rule_only_changes_trigger_uid_membership_without_a_link_transition() {
        let mut state = RouteState::default();
        for kind in [RTM_NEWROUTE, RTM_DELROUTE, RTM_NEWRULE, RTM_DELRULE] {
            assert!(consume(&nlmsg(kind, &[0; 12]), &mut state));
            assert!(state.up.is_empty());
        }
    }

    #[test]
    fn dump_keeps_deletions_and_rejects_interrupted_or_foreign_completion() {
        let mut state = RouteState::default();
        state.apply(RTM_NEWLINK, "tun0".into(), true);
        assert!(!consume_dump(&nlmsg(RTM_DELLINK, &link_body("tun0", false)), &mut state).unwrap());
        assert!(state.up.is_empty());
        assert!(!consume_dump(&nlmsg(NLMSG_DONE, &[]), &mut state).unwrap());
        let mut done = nlmsg(NLMSG_DONE, &[]);
        done[8..12].copy_from_slice(&1u32.to_ne_bytes());
        assert!(consume_dump(&done, &mut state).unwrap());
        done[6..8].copy_from_slice(&NLM_F_DUMP_INTR.to_ne_bytes());
        assert!(consume_dump(&done, &mut state).is_err());
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn unchanged_resync_does_not_invalidate_app_measurements() {
        let _guard = crate::ports::test_env_lock();
        let keys = [
            framework_vpn::FRAMEWORK_IFACES_ENV,
            framework_vpn::FRAMEWORK_MODE_ENV,
            routing_compose::OBSERVE_UID_ENV,
        ];
        let previous = keys.map(|key| (key, std::env::var_os(key)));
        unsafe {
            std::env::set_var(framework_vpn::FRAMEWORK_IFACES_ENV, "");
            std::env::remove_var(framework_vpn::FRAMEWORK_MODE_ENV);
            std::env::set_var(routing_compose::OBSERVE_UID_ENV, "10042");
        }
        let state = CoreState::new(crate::Publisher::new(1, Default::default()));
        publish(&state, &RouteState::default()).await;
        let first = state.publisher.current();
        publish(&state, &RouteState::default()).await;
        let second = state.publisher.current();
        for (key, value) in previous {
            unsafe {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
        assert_eq!(first.revision, second.revision);
        assert_eq!(
            first.snapshot.routing.generation,
            second.snapshot.routing.generation
        );
        assert_eq!(
            second.snapshot.routing.uid.unwrap().state,
            vpnhide_core_rpc::UidVpnRouteState::VpnOff
        );
    }
}
