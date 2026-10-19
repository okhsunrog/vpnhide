//! THROWAWAY SPIKE — core-2.0 §12 step 1a (observation value).
//!
//! Question this answers on a real device: can a uid-0 `NETLINK_ROUTE` listener
//! catch every VPN up/down edge promptly, without missing edges, and does the
//! recomputed "is this uid routed through the VPN" flip correctly — using the
//! same shared primitive the app already ships (`vpnhide_checks`)?
//!
//! What it does NOT try to be: production. It derives the VPN-interface set by
//! kernel name-matching (`is_vpn_interface_name`), NOT from the framework's
//! authoritative `TRANSPORT_VPN` classification. That is deliberate: per the
//! architecture note (§8), netlink can trigger and measure membership but cannot
//! classify a VPN, so a real observer cross-checks the framework. Here we only
//! measure whether netlink is a reliable *trigger*.
//!
//! Usage (on device, as root):
//!   netlink-spike --uid <uid> [--raw]
//! Then toggle the VPN on/off several times and watch the `STATE` lines. Also try
//! a Wi-Fi<->cellular handover (should NOT emit a vpn edge) and a route storm.

use std::collections::{BTreeSet, HashMap};
use std::time::Instant;

use vpnhide_checks::{is_vpn_interface_name, self_routed_for_interfaces};

// ---- netlink ABI constants (defined here, not via libc, to avoid per-target
// coverage gaps; these are stable kernel numbers). ----
mod nl {
    pub const RTM_NEWLINK: u16 = 16;
    pub const RTM_DELLINK: u16 = 17;
    pub const RTM_GETLINK: u16 = 18;
    pub const RTM_NEWROUTE: u16 = 24;
    pub const RTM_DELROUTE: u16 = 25;
    pub const RTM_NEWRULE: u16 = 32;
    pub const RTM_DELRULE: u16 = 33;

    pub const NLMSG_ERROR: u16 = 2;
    pub const NLMSG_DONE: u16 = 3;

    pub const NLM_F_REQUEST: u16 = 0x001;
    pub const NLM_F_ROOT: u16 = 0x100;
    pub const NLM_F_MATCH: u16 = 0x200;
    pub const NLM_F_DUMP: u16 = NLM_F_ROOT | NLM_F_MATCH;

    // RTNLGRP_* group *numbers* (for NETLINK_ADD_MEMBERSHIP), not the legacy mask bits.
    pub const RTNLGRP_LINK: u32 = 1;
    pub const RTNLGRP_IPV4_ROUTE: u32 = 7;
    pub const RTNLGRP_IPV6_ROUTE: u32 = 11;
    pub const RTNLGRP_IPV4_RULE: u32 = 19;
    pub const RTNLGRP_IPV6_RULE: u32 = 20;

    pub const IFLA_IFNAME: u16 = 3;
    pub const RTA_OIF: u16 = 4;

    pub const IFF_UP: u32 = 0x1;

    pub const NLMSG_ALIGNTO: usize = 4;
    pub const RTA_ALIGNTO: usize = 4;

    pub const fn align(len: usize, to: usize) -> usize {
        (len + to - 1) & !(to - 1)
    }
    // sizeof(struct nlmsghdr) == 16, aligned to 4 == 16.
    pub const NLMSG_HDRLEN: usize = 16;
}

/// Copy a `Copy` struct out of a byte slice without assuming alignment.
fn read_unaligned<T: Copy>(bytes: &[u8]) -> Option<T> {
    if bytes.len() < size_of::<T>() {
        return None;
    }
    // SAFETY: length checked; read_unaligned tolerates any alignment.
    Some(unsafe { std::ptr::read_unaligned(bytes.as_ptr() as *const T) })
}

/// Iterate `(nlmsg_type, payload)` over a netlink batch. `payload` is the bytes
/// after the 16-byte header, truncated to this message's declared length.
fn for_each_message(buf: &[u8], mut f: impl FnMut(u16, &[u8])) {
    let mut offset = 0usize;
    while buf.len() - offset >= nl::NLMSG_HDRLEN {
        let hdr = &buf[offset..];
        // struct nlmsghdr { u32 len; u16 type; u16 flags; u32 seq; u32 pid; }
        let len = read_unaligned::<u32>(&hdr[0..4]).unwrap() as usize;
        let msg_type = read_unaligned::<u16>(&hdr[4..6]).unwrap();
        if len < nl::NLMSG_HDRLEN || offset + len > buf.len() {
            break;
        }
        let payload = &buf[offset + nl::NLMSG_HDRLEN..offset + len];
        f(msg_type, payload);
        offset += nl::align(len, nl::NLMSG_ALIGNTO);
    }
}

/// Find one rtattr's data by type within a message body that starts with a
/// fixed header of `body_offset` bytes (ifinfomsg=16, rtmsg=12).
fn find_attr(payload: &[u8], body_offset: usize, want: u16) -> Option<&[u8]> {
    let mut offset = nl::align(body_offset, nl::RTA_ALIGNTO);
    while payload.len().saturating_sub(offset) >= 4 {
        let rta = &payload[offset..];
        // struct rtattr { u16 len; u16 type; }
        let rta_len = read_unaligned::<u16>(&rta[0..2]).unwrap() as usize;
        let rta_type = read_unaligned::<u16>(&rta[2..4]).unwrap();
        if rta_len < 4 || offset + rta_len > payload.len() {
            break;
        }
        if rta_type == want {
            return Some(&payload[offset + 4..offset + rta_len]);
        }
        offset += nl::align(rta_len, nl::RTA_ALIGNTO);
    }
    None
}

/// (ifindex, name, administratively up) from an RTM_*LINK message.
fn parse_link(payload: &[u8]) -> Option<(u32, String, bool)> {
    // struct ifinfomsg { u8 family; u8 pad; u16 type; i32 index; u32 flags; u32 change; }
    let index = read_unaligned::<i32>(payload.get(4..8)?)? as u32;
    let flags = read_unaligned::<u32>(payload.get(8..12)?)?;
    let name_bytes = find_attr(payload, 16, nl::IFLA_IFNAME)?;
    let name = name_bytes
        .split(|&b| b == 0)
        .next()
        .and_then(|b| std::str::from_utf8(b).ok())?
        .to_owned();
    if name.is_empty() {
        return None;
    }
    Some((index, name, flags & nl::IFF_UP != 0))
}

/// Out-interface index from an RTM_*ROUTE message, if it carries RTA_OIF.
fn parse_route_oif(payload: &[u8]) -> Option<u32> {
    let data = find_attr(payload, 12, nl::RTA_OIF)?;
    read_unaligned::<u32>(data)
}

struct RouteSocket {
    fd: i32,
}

impl RouteSocket {
    fn open(rcvbuf: Option<i32>) -> std::io::Result<Self> {
        // SAFETY: plain socket(2) with constant args.
        let fd = unsafe {
            libc::socket(
                libc::AF_NETLINK,
                libc::SOCK_RAW | libc::SOCK_CLOEXEC,
                libc::NETLINK_ROUTE,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let sock = RouteSocket { fd };

        // Spike knob: shrink the receive buffer below the normal floor (root can, via
        // SO_RCVBUFFORCE) so that even one VPN transition's ~135-message burst overflows
        // it — the deterministic way to exercise the ENOBUFS -> resync path without a
        // real route storm. Not something a production observer would ever do.
        if let Some(bytes) = rcvbuf {
            const SO_RCVBUFFORCE: libc::c_int = 33;
            let set = |opt: libc::c_int| unsafe {
                libc::setsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    opt,
                    &bytes as *const _ as *const libc::c_void,
                    size_of::<i32>() as libc::socklen_t,
                )
            };
            // FORCE bypasses the kernel floor but needs CAP_NET_ADMIN, which a KSU
            // uid-0 shell may lack (EPERM). Fall back to plain SO_RCVBUF, which any
            // caller may set but the kernel clamps to its floor (~a few KB) — still
            // small enough that a VPN transition's burst overflows it under --slow.
            if set(SO_RCVBUFFORCE) < 0 {
                if set(libc::SO_RCVBUF) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                eprintln!(
                    "note: SO_RCVBUFFORCE denied (no CAP_NET_ADMIN); used SO_RCVBUF (kernel-floor clamped)"
                );
            }
        }

        let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        addr.nl_family = libc::AF_NETLINK as u16;
        // nl_groups=0: subscribe explicitly below so we can request >32 groups.
        let rc = unsafe {
            libc::bind(
                fd,
                &addr as *const _ as *const libc::sockaddr,
                size_of::<libc::sockaddr_nl>() as libc::socklen_t,
            )
        };
        if rc < 0 {
            return Err(std::io::Error::last_os_error());
        }

        for group in [
            nl::RTNLGRP_LINK,
            nl::RTNLGRP_IPV4_ROUTE,
            nl::RTNLGRP_IPV6_ROUTE,
            nl::RTNLGRP_IPV4_RULE,
            nl::RTNLGRP_IPV6_RULE,
        ] {
            sock.add_membership(group)?;
        }
        Ok(sock)
    }

    fn add_membership(&self, group: u32) -> std::io::Result<()> {
        let rc = unsafe {
            libc::setsockopt(
                self.fd,
                libc::SOL_NETLINK,
                libc::NETLINK_ADD_MEMBERSHIP,
                &group as *const _ as *const libc::c_void,
                size_of::<u32>() as libc::socklen_t,
            )
        };
        if rc < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// Ask the kernel to dump every link, to (re)build the index->name map.
    fn request_link_dump(&self) -> std::io::Result<()> {
        // nlmsghdr(16) + ifinfomsg(16), family AF_UNSPEC.
        let mut req = [0u8; 32];
        let total = req.len() as u32;
        req[0..4].copy_from_slice(&total.to_ne_bytes());
        req[4..6].copy_from_slice(&nl::RTM_GETLINK.to_ne_bytes());
        req[6..8].copy_from_slice(&(nl::NLM_F_REQUEST | nl::NLM_F_DUMP).to_ne_bytes());
        req[8..12].copy_from_slice(&1u32.to_ne_bytes()); // seq
        // pid=0, ifinfomsg all-zero (family AF_UNSPEC) -> dump all.
        let sent =
            unsafe { libc::send(self.fd, req.as_ptr() as *const libc::c_void, req.len(), 0) };
        if sent < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// One recv. Returns the batch, or Err whose kind is ENOBUFS on overflow.
    fn recv(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = unsafe { libc::recv(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0) };
        if n < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(n as usize)
    }

    /// Block until readable or `timeout_ms` (-1 = forever). true if readable.
    fn poll_readable(&self, timeout_ms: i32) -> std::io::Result<bool> {
        let mut pfd = libc::pollfd {
            fd: self.fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let rc = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
        if rc < 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EINTR) {
                return Ok(false);
            }
            return Err(err);
        }
        Ok(rc > 0 && pfd.revents & libc::POLLIN != 0)
    }
}

impl Drop for RouteSocket {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd) };
    }
}

const DEBOUNCE_MS: u128 = 250;
const RECV_BUF: usize = 64 * 1024;

struct Observer {
    uid: u32,
    raw: bool,
    start: Instant,
    links: HashMap<u32, String>,
    vpn_up: BTreeSet<String>,
    last_vpn: BTreeSet<String>,
    last_routed: Option<bool>,
    events_since_flush: u32,
    last_edge_at: Instant,
    event_at: Instant,
}

impl Observer {
    fn new(uid: u32, raw: bool) -> Self {
        let now = Instant::now();
        Observer {
            uid,
            raw,
            start: now,
            links: HashMap::new(),
            vpn_up: BTreeSet::new(),
            last_vpn: BTreeSet::new(),
            last_routed: None,
            events_since_flush: 0,
            last_edge_at: now,
            event_at: now,
        }
    }

    fn ms(&self) -> u128 {
        self.start.elapsed().as_millis()
    }

    /// Apply a link event to the index->name map and the vpn-up set.
    fn on_link(&mut self, msg_type: u16, payload: &[u8]) {
        let Some((index, name, up)) = parse_link(payload) else {
            return;
        };
        if self.raw {
            println!(
                "[+{:>7}ms] MSG {} idx={index} {name} up={up}",
                self.ms(),
                if msg_type == nl::RTM_DELLINK {
                    "DELLINK"
                } else {
                    "NEWLINK"
                }
            );
        }
        let removed = msg_type == nl::RTM_DELLINK || !up;
        if removed {
            self.links.remove(&index);
            if is_vpn_interface_name(&name) {
                self.vpn_up.remove(&name);
            }
        } else {
            self.links.insert(index, name.clone());
            if is_vpn_interface_name(&name) {
                self.vpn_up.insert(name);
            }
        }
    }

    /// A route/rule event may change this uid's membership. Returns true if it
    /// touches the VPN and is worth a recompute.
    fn on_route_or_rule(&mut self, msg_type: u16, payload: &[u8]) -> bool {
        match msg_type {
            nl::RTM_NEWROUTE | nl::RTM_DELROUTE => {
                let oif_name = parse_route_oif(payload).and_then(|i| self.links.get(&i));
                let touches_vpn = oif_name.is_some_and(|n| is_vpn_interface_name(n));
                if self.raw {
                    println!(
                        "[+{:>7}ms] MSG {} oif={}",
                        self.ms(),
                        if msg_type == nl::RTM_DELROUTE {
                            "DELROUTE"
                        } else {
                            "NEWROUTE"
                        },
                        oif_name.map(String::as_str).unwrap_or("?")
                    );
                }
                touches_vpn
            }
            nl::RTM_NEWRULE | nl::RTM_DELRULE => {
                if self.raw {
                    println!(
                        "[+{:>7}ms] MSG {}",
                        self.ms(),
                        if msg_type == nl::RTM_DELRULE {
                            "DELRULE"
                        } else {
                            "NEWRULE"
                        }
                    );
                }
                // A policy-rule change (Android's per-uid VPN steering) always warrants a recheck.
                true
            }
            _ => false,
        }
    }

    /// Recompute membership and print one STATE line, tagged with what changed.
    fn flush(&mut self, reason: &str) {
        let ifaces: Vec<String> = self.vpn_up.iter().cloned().collect();
        let (routed, detail) = if ifaces.is_empty() {
            (Some(false), "no_vpn".to_owned())
        } else {
            self_routed_for_interfaces(self.uid, Some(&ifaces))
        };
        let vpn_changed = self.vpn_up != self.last_vpn;
        let routed_changed = routed != self.last_routed;
        let gap = self.last_edge_at.elapsed().as_millis();
        println!(
            "[+{:>7}ms] STATE {reason} vpn=[{}] routed={} detail={detail} Δvpn={} Δrouted={} evts={} gap={gap}ms",
            self.ms(),
            ifaces.join(","),
            routed
                .map(|b| b.to_string())
                .unwrap_or_else(|| "unknown".to_owned()),
            if vpn_changed { "Y" } else { "n" },
            if routed_changed { "Y" } else { "n" },
            self.events_since_flush,
        );
        self.last_vpn = self.vpn_up.clone();
        self.last_routed = routed;
        self.events_since_flush = 0;
        self.last_edge_at = Instant::now();
    }
}

struct Config {
    uid: u32,
    raw: bool,
    rcvbuf: Option<i32>,
    slow_ms: u64,
}

fn parse_args() -> Option<Config> {
    let mut uid: Option<u32> = None;
    let mut raw = false;
    let mut rcvbuf = None;
    let mut slow_ms = 0u64;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--uid" => uid = args.next().and_then(|v| v.parse().ok()),
            "--raw" => raw = true,
            "--rcvbuf" => rcvbuf = args.next().and_then(|v| v.parse().ok()),
            "--slow" => slow_ms = args.next().and_then(|v| v.parse().ok())?,
            _ => return None,
        }
    }
    uid.map(|uid| Config {
        uid,
        raw,
        rcvbuf,
        slow_ms,
    })
}

fn main() {
    let Some(cfg) = parse_args() else {
        eprintln!("usage: netlink-spike --uid <uid> [--raw] [--rcvbuf <bytes>] [--slow <ms>]");
        std::process::exit(2);
    };
    let Config {
        uid,
        raw,
        rcvbuf,
        slow_ms,
    } = cfg;

    let sock = match RouteSocket::open(rcvbuf) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("netlink open failed (root? enforcing SELinux denial?): {e}");
            std::process::exit(1);
        }
    };

    let mut obs = Observer::new(uid, raw);
    println!(
        "netlink-spike: uid={uid} — toggle the VPN and watch STATE lines. \
         A Wi-Fi<->cellular handover should show Δvpn=n. Ctrl-C to stop."
    );

    // Initial resync + snapshot.
    resync(&sock, &mut obs);
    obs.flush("init");

    let mut buf = vec![0u8; RECV_BUF];
    let mut dirty = false;
    loop {
        let timeout = if dirty {
            let remaining = DEBOUNCE_MS.saturating_sub(obs.since_last_event());
            remaining.min(1000) as i32
        } else {
            -1
        };
        let readable = match sock.poll_readable(timeout) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("poll failed: {e}");
                break;
            }
        };
        if readable {
            match sock.recv(&mut buf) {
                Ok(n) => {
                    let hits = consume_batch(&mut obs, &buf[..n]);
                    if hits > 0 {
                        dirty = true;
                        obs.note_events(hits);
                    }
                    // Spike knob: simulate a slow consumer so a small rcvbuf backs up
                    // and overflows during a burst, forcing the ENOBUFS path.
                    if slow_ms > 0 {
                        std::thread::sleep(std::time::Duration::from_millis(slow_ms));
                    }
                }
                Err(e) if e.raw_os_error() == Some(libc::ENOBUFS) => {
                    println!("[+{:>7}ms] OVERFLOW — events dropped, resyncing", obs.ms());
                    resync(&sock, &mut obs);
                    dirty = true;
                    obs.note_events(1);
                }
                Err(e) => {
                    eprintln!("recv failed: {e}");
                    break;
                }
            }
        } else if dirty && obs.debounce_elapsed() {
            obs.flush("edge");
            dirty = false;
        }
    }
}

// ---- small helpers that need Observer internals but read better free-standing ----

impl Observer {
    fn note_events(&mut self, hits: usize) {
        self.events_since_flush += hits as u32;
        self.event_at = Instant::now();
    }
    fn since_last_event(&self) -> u128 {
        self.event_at.elapsed().as_millis()
    }
    fn debounce_elapsed(&self) -> bool {
        self.event_at.elapsed().as_millis() >= DEBOUNCE_MS
    }
}

/// Feed one recv batch through the observer. Returns the number of relevant
/// messages (0 = nothing that could change VPN presence or membership).
fn consume_batch(obs: &mut Observer, batch: &[u8]) -> usize {
    let mut hits = 0usize;
    for_each_message(batch, |msg_type, payload| match msg_type {
        nl::RTM_NEWLINK | nl::RTM_DELLINK => {
            let before = obs.vpn_up.clone();
            obs.on_link(msg_type, payload);
            if obs.vpn_up != before {
                hits += 1;
            }
        }
        nl::RTM_NEWROUTE | nl::RTM_DELROUTE | nl::RTM_NEWRULE | nl::RTM_DELRULE => {
            if obs.on_route_or_rule(msg_type, payload) {
                hits += 1;
            }
        }
        nl::NLMSG_ERROR => {
            eprintln!("[+{:>7}ms] netlink NLMSG_ERROR", obs.ms());
        }
        _ => {}
    });
    hits
}

/// Rebuild the link map from a fresh kernel dump (startup and after overflow).
fn resync(sock: &RouteSocket, obs: &mut Observer) {
    obs.links.clear();
    obs.vpn_up.clear();
    if let Err(e) = sock.request_link_dump() {
        eprintln!("link dump request failed: {e}");
        return;
    }
    let mut buf = vec![0u8; RECV_BUF];
    let mut done = false;
    while !done {
        let n = match sock.recv(&mut buf) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("dump recv failed: {e}");
                return;
            }
        };
        for_each_message(&buf[..n], |msg_type, payload| match msg_type {
            nl::RTM_NEWLINK => obs.on_link(nl::RTM_NEWLINK, payload),
            nl::NLMSG_DONE => done = true,
            _ => {}
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // struct nlmsghdr { u32 len; u16 type; u16 flags; u32 seq; u32 pid; }
    fn nlmsg(msg_type: u16, body: &[u8]) -> Vec<u8> {
        let len = (nl::NLMSG_HDRLEN + body.len()) as u32;
        let mut m = Vec::new();
        m.extend_from_slice(&len.to_ne_bytes());
        m.extend_from_slice(&msg_type.to_ne_bytes());
        m.extend_from_slice(&0u16.to_ne_bytes());
        m.extend_from_slice(&0u32.to_ne_bytes());
        m.extend_from_slice(&0u32.to_ne_bytes());
        m.extend_from_slice(body);
        while m.len() % nl::NLMSG_ALIGNTO != 0 {
            m.push(0);
        }
        m
    }

    fn rtattr(rta_type: u16, data: &[u8]) -> Vec<u8> {
        let len = (4 + data.len()) as u16;
        let mut a = Vec::new();
        a.extend_from_slice(&len.to_ne_bytes());
        a.extend_from_slice(&rta_type.to_ne_bytes());
        a.extend_from_slice(data);
        while a.len() % nl::RTA_ALIGNTO != 0 {
            a.push(0);
        }
        a
    }

    fn link_body(index: u32, name: &str, up: bool) -> Vec<u8> {
        let mut b = vec![0u8; 16]; // ifinfomsg
        b[4..8].copy_from_slice(&(index as i32).to_ne_bytes());
        let flags = if up { nl::IFF_UP } else { 0 };
        b[8..12].copy_from_slice(&flags.to_ne_bytes());
        let mut name_z = name.as_bytes().to_vec();
        name_z.push(0);
        b.extend_from_slice(&rtattr(nl::IFLA_IFNAME, &name_z));
        b
    }

    #[test]
    fn parses_link_name_index_and_up_flag() {
        let body = link_body(7, "tun0", true);
        let (idx, name, up) = parse_link(&body).unwrap();
        assert_eq!((idx, name.as_str(), up), (7, "tun0", true));
    }

    #[test]
    fn parses_route_out_interface_index() {
        let mut body = vec![0u8; 12]; // rtmsg
        body.extend_from_slice(&rtattr(nl::RTA_OIF, &42u32.to_ne_bytes()));
        assert_eq!(parse_route_oif(&body), Some(42));
    }

    #[test]
    fn iterates_multiple_messages_in_one_batch() {
        let mut batch = nlmsg(nl::RTM_NEWLINK, &link_body(1, "lo", true));
        batch.extend(nlmsg(nl::RTM_NEWLINK, &link_body(7, "tun0", true)));
        let mut names = Vec::new();
        for_each_message(&batch, |t, p| {
            if t == nl::RTM_NEWLINK {
                names.push(parse_link(p).unwrap().1);
            }
        });
        assert_eq!(names, vec!["lo".to_owned(), "tun0".to_owned()]);
    }

    #[test]
    fn vpn_up_set_tracks_add_and_remove() {
        let mut obs = Observer::new(10234, false);
        obs.on_link(nl::RTM_NEWLINK, &link_body(7, "tun0", true));
        assert!(obs.vpn_up.contains("tun0"));
        obs.on_link(nl::RTM_DELLINK, &link_body(7, "tun0", true));
        assert!(obs.vpn_up.is_empty());
    }

    #[test]
    fn non_vpn_link_does_not_enter_vpn_set() {
        let mut obs = Observer::new(10234, false);
        obs.on_link(nl::RTM_NEWLINK, &link_body(2, "wlan0", true));
        assert!(obs.vpn_up.is_empty());
    }
}
