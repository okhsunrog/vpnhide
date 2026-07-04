mod generated;

use std::ffi::CStr;
use std::io::ErrorKind;

use crate::generated::iface_lists::matches_vpn;

// ── Probe outcome types — serialized to JSON on both transports ───────
// In-process (app view) via the JNI export below; root ground-truth via the
// `vhprobe` bin. Same code, same JSON schema on both sides.

#[derive(serde::Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    /// Probe ran and saw nothing VPN-shaped — the VPN is hidden from this
    /// surface (by a backend hook, or simply nothing to leak here). The caller
    /// distinguishes those two via the root ground-truth differential.
    Pass,
    /// Probe surfaced VPN-shaped data the kmod / zygisk should have hidden.
    Fail,
    /// The probe was denied by SELinux (EACCES/EPERM) before it could read.
    /// Distinct from Pass: the app saw no VPN, but SELinux blocked the read —
    /// not a backend hook — so it is not evidence the backend works.
    SelinuxBlocked,
    /// App has no network permission, so the probe couldn't run at all.
    /// Reported separately so the UI can tell the user to enable network
    /// access before trusting the results.
    NetworkBlocked,
}

#[derive(serde::Serialize, Debug, Clone)]
pub struct CheckOutput {
    pub status: CheckStatus,
    pub detail: String,
}

impl CheckOutput {
    fn pass(detail: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::Pass,
            detail: detail.into(),
        }
    }

    fn fail(detail: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::Fail,
            detail: detail.into(),
        }
    }

    fn selinux_blocked(detail: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::SelinuxBlocked,
            detail: detail.into(),
        }
    }

    fn network_blocked(detail: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::NetworkBlocked,
            detail: detail.into(),
        }
    }
}

fn is_vpn_iface(name: &str) -> bool {
    matches_vpn(name.as_bytes())
}

/// 8-byte-aligned byte buffer. Several probes reinterpret the raw bytes the
/// kernel writes back (`ifreq` from SIOCGIFCONF, `nlmsghdr`/`rtattr` from a
/// netlink dump) as typed structs. A plain `[u8; N]` is only 1-aligned, so
/// forming a reference/slice of those types over it is undefined behaviour even
/// when it happens to work. Over-aligning the backing storage to 8 — at least
/// the alignment of every struct read out of these buffers — makes every such
/// reference well-formed.
#[repr(align(8))]
struct AlignedBytes<const N: usize>([u8; N]);

impl<const N: usize> AlignedBytes<N> {
    fn zeroed() -> Self {
        Self([0u8; N])
    }
}

fn is_selinux_denial(e: &std::io::Error) -> bool {
    e.kind() == ErrorKind::PermissionDenied
}

// ── helpers ──────────────────────────────────────────────────────────

fn cstr_to_str(ptr: *const libc::c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

fn last_os_error() -> String {
    std::io::Error::last_os_error().to_string()
}

fn last_os_errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

fn join_list(v: &[String]) -> String {
    v.join(", ")
}

fn format_iface_result(all: &[String], vpn: &[String], context: &str) -> CheckOutput {
    if vpn.is_empty() {
        CheckOutput::pass(format!("{context} [{list}], no VPN", list = join_list(all)))
    } else {
        CheckOutput::fail(format!(
            "VPN interfaces [{vpn}] in [{all}]",
            vpn = join_list(vpn),
            all = join_list(all),
        ))
    }
}

// ── structs missing from libc crate on Android ───────────────────────

#[repr(C)]
struct Ifinfomsg {
    ifi_family: u8,
    _pad: u8,
    ifi_type: u16,
    ifi_index: i32,
    ifi_flags: u32,
    ifi_change: u32,
}

#[repr(C)]
struct Rtmsg {
    rtm_family: u8,
    rtm_dst_len: u8,
    rtm_src_len: u8,
    rtm_tos: u8,
    rtm_table: u8,
    rtm_protocol: u8,
    rtm_scope: u8,
    rtm_type: u8,
    rtm_flags: u32,
}

#[repr(C)]
struct Rtattr {
    rta_len: u16,
    rta_type: u16,
}

const IFLA_IFNAME: u16 = 3;
const RTA_OIF: u16 = 4;

// ── check implementations ────────────────────────────────────────────

/// Open an IPv4 datagram socket and pass it to `f`, then close it.
/// Returns `CheckOutput::network_blocked(...)` if `socket()` returns
/// ECONNREFUSED (no NETWORK permission), `CheckOutput::fail(...)` for
/// any other socket() failure, otherwise the result of `f(fd)`.
unsafe fn with_inet_dgram_socket(f: impl FnOnce(libc::c_int) -> CheckOutput) -> CheckOutput {
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
    if fd < 0 {
        let err = last_os_errno();
        if err == libc::ECONNREFUSED {
            return CheckOutput::network_blocked(
                "socket() returned ECONNREFUSED — network access disabled for this app",
            );
        }
        return CheckOutput::fail(format!("cannot create socket: {}", last_os_error()));
    }
    let out = f(fd);
    unsafe { libc::close(fd) };
    out
}

fn check_ioctl_siocgifflags() -> CheckOutput {
    unsafe {
        with_inet_dgram_socket(|fd| {
            let mut ifr: libc::ifreq = std::mem::zeroed();
            let name = b"tun0\0";
            ifr.ifr_name[..name.len()].copy_from_slice(&name.map(|b| b as libc::c_char));

            let ret = libc::ioctl(fd, libc::SIOCGIFFLAGS as _, &ifr);
            let err = last_os_errno();

            if ret < 0 {
                if err == libc::ENODEV {
                    CheckOutput::pass(
                        "ioctl(tun0, SIOCGIFFLAGS) returned ENODEV — interface not visible",
                    )
                } else if err == libc::ENXIO {
                    CheckOutput::pass(
                        "ioctl(tun0, SIOCGIFFLAGS) returned ENXIO — interface not visible",
                    )
                } else {
                    CheckOutput::fail(format!("ioctl returned error {err} ({})", last_os_error()))
                }
            } else {
                let flags = ifr.ifr_ifru.ifru_flags as u32;
                CheckOutput::fail(format!(
                    "tun0 is visible! flags=0x{flags:x} (IFF_UP={}, IFF_RUNNING={})",
                    u8::from(flags & libc::IFF_UP as u32 != 0),
                    u8::from(flags & libc::IFF_RUNNING as u32 != 0),
                ))
            }
        })
    }
}

fn check_ioctl_siocgifmtu() -> CheckOutput {
    unsafe {
        with_inet_dgram_socket(|fd| {
            let mut ifr: libc::ifreq = std::mem::zeroed();
            let name = b"tun0\0";
            ifr.ifr_name[..name.len()].copy_from_slice(&name.map(|b| b as libc::c_char));

            let ret = libc::ioctl(fd, libc::SIOCGIFMTU as _, &ifr);
            let err = last_os_errno();

            if ret < 0 {
                if err == libc::ENODEV || err == libc::ENXIO {
                    CheckOutput::pass(
                        "ioctl(tun0, SIOCGIFMTU) returned ENODEV — interface not visible",
                    )
                } else {
                    CheckOutput::fail(format!("ioctl returned error {err} ({})", last_os_error()))
                }
            } else {
                let mtu = ifr.ifr_ifru.ifru_mtu;
                CheckOutput::fail(format!("tun0 is visible! MTU={mtu}"))
            }
        })
    }
}

fn check_ioctl_siocgifconf() -> CheckOutput {
    unsafe {
        with_inet_dgram_socket(|fd| {
            let mut buf = AlignedBytes::<4096>::zeroed();
            let mut ifc: libc::ifconf = std::mem::zeroed();
            ifc.ifc_len = buf.0.len() as libc::c_int;
            ifc.ifc_ifcu.ifcu_buf = buf.0.as_mut_ptr().cast();

            if libc::ioctl(fd, libc::SIOCGIFCONF as _, &mut ifc) < 0 {
                let e = last_os_error();
                return CheckOutput::fail(format!("ioctl error: {e}"));
            }

            let count = ifc.ifc_len as usize / std::mem::size_of::<libc::ifreq>();
            let reqs = std::slice::from_raw_parts(buf.0.as_ptr() as *const libc::ifreq, count);

            let mut all = Vec::new();
            let mut vpn = Vec::new();
            for req in reqs {
                let name = cstr_to_str(req.ifr_name.as_ptr());
                if is_vpn_iface(&name) {
                    vpn.push(name.clone());
                }
                all.push(name);
            }

            format_iface_result(&all, &vpn, &format!("{count} interfaces visible:"))
        })
    }
}

fn check_getifaddrs() -> CheckOutput {
    unsafe {
        let mut addrs: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut addrs) != 0 {
            return CheckOutput::fail(format!("getifaddrs error: {}", last_os_error()));
        }

        let mut all: Vec<String> = Vec::new();
        let mut vpn: Vec<String> = Vec::new();
        let mut ifa = addrs;
        while !ifa.is_null() {
            let entry = &*ifa;
            if !entry.ifa_name.is_null() {
                let name = cstr_to_str(entry.ifa_name);
                if !all.contains(&name) {
                    all.push(name.clone());
                }
                if is_vpn_iface(&name) && !vpn.contains(&name) {
                    vpn.push(name);
                }
            }
            ifa = entry.ifa_next;
        }
        libc::freeifaddrs(addrs);

        format_iface_result(&all, &vpn, &format!("{} unique interfaces:", all.len()))
    }
}

fn check_proc_file(path: &str) -> CheckOutput {
    match std::fs::read_to_string(path) {
        Err(e) => {
            if is_selinux_denial(&e) {
                return CheckOutput::selinux_blocked(format!(
                    "access denied by SELinux ({e}) — app cannot read {path}"
                ));
            }
            CheckOutput::fail(format!("cannot open {path}: {e}"))
        }
        Ok(content) => {
            let mut total = 0;
            let mut vpn_lines = Vec::new();
            for line in content.lines() {
                if line.is_empty() {
                    continue;
                }
                total += 1;
                if line.split_ascii_whitespace().any(is_vpn_iface) {
                    vpn_lines.push(line[..line.len().min(80)].to_string());
                }
            }
            if vpn_lines.is_empty() {
                CheckOutput::pass(format!("{total} lines in {path}, no VPN entries"))
            } else {
                let details: String = vpn_lines.iter().map(|l| format!("\n  {l}")).collect();
                CheckOutput::fail(format!("{} VPN lines in {path}:{details}", vpn_lines.len()))
            }
        }
    }
}

/// Upper bound on recvmsg iterations for a single netlink dump. A real
/// RTM_GETLINK / RTM_GETROUTE dump completes in a few 32 KiB reads; a stream
/// that exceeds this is looping (a kernel iface filter re-sending without
/// NLMSG_DONE — issue #61), so we stop instead of growing the result `Vec`
/// without bound until Scudo aborts the process with an OOM map failure.
const MAX_NETLINK_RECV_ITERS: usize = 256;

/// Wrapper around recvmsg for netlink sockets. Uses recvmsg (not recv/recvfrom)
/// so that zygisk's recvmsg hook can filter the response.
unsafe fn netlink_recv(fd: i32, buf: &mut [u8]) -> isize {
    unsafe {
        let mut iov = libc::iovec {
            iov_base: buf.as_mut_ptr().cast(),
            iov_len: buf.len(),
        };
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        libc::recvmsg(fd, &mut msg, 0)
    }
}

/// Open a bound NETLINK_ROUTE socket.
///
/// `Err` is short-circuit control flow: callers `return` it as the probe
/// outcome verbatim. The wrapped `CheckOutput` may carry any status —
/// SELinux denials map to `Pass` (the kernel hid the interface, exactly
/// what we want), real failures map to `Fail`.
fn open_netlink() -> Result<i32, CheckOutput> {
    unsafe {
        let fd = libc::socket(libc::AF_NETLINK, libc::SOCK_RAW, libc::NETLINK_ROUTE);
        if fd < 0 {
            let e = std::io::Error::last_os_error();
            return Err(if is_selinux_denial(&e) {
                CheckOutput::selinux_blocked(format!("netlink socket denied by SELinux ({e})"))
            } else {
                CheckOutput::fail(format!("cannot create netlink socket: {e}"))
            });
        }

        let mut sa: libc::sockaddr_nl = std::mem::zeroed();
        sa.nl_family = libc::AF_NETLINK as u16;
        let sa_len = std::mem::size_of_val(&sa) as libc::socklen_t;
        if libc::bind(fd, std::ptr::from_ref(&sa).cast(), sa_len) < 0 {
            let e = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(if is_selinux_denial(&e) {
                CheckOutput::selinux_blocked(format!(
                    "netlink bind denied by SELinux ({e}) — app cannot enumerate interfaces"
                ))
            } else {
                CheckOutput::fail(format!("bind error: {e}"))
            });
        }

        // Receive timeout: a kernel-side interface filter can, on some kernels,
        // re-send a dump without ever emitting NLMSG_DONE (issue #61, observed
        // on android14-6.1). Without a timeout the next blocking recvmsg hangs
        // the diagnostics thread forever; with it the read returns EAGAIN and
        // the loop exits. Best-effort — the per-call iteration cap is the hard
        // backstop, so a setsockopt failure is non-fatal.
        let tv = libc::timeval {
            tv_sec: 2,
            tv_usec: 0,
        };
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            std::ptr::from_ref(&tv).cast(),
            std::mem::size_of::<libc::timeval>() as libc::socklen_t,
        );
        Ok(fd)
    }
}

/// Parse netlink messages from a buffer, calling `on_msg` for each message.
/// Returns false if NLMSG_DONE or NLMSG_ERROR was seen.
///
/// # Safety
/// `buf` must contain valid netlink messages up to `len` bytes.
unsafe fn parse_netlink_msgs(
    buf: &[u8],
    len: usize,
    msg_type: u16,
    mut on_msg: impl FnMut(&[u8], usize, usize),
) -> bool {
    let mut offset = 0usize;
    let hdr_size = std::mem::size_of::<libc::nlmsghdr>();
    while offset + hdr_size <= len {
        let nh = unsafe { &*(buf.as_ptr().add(offset) as *const libc::nlmsghdr) };
        let msg_len = nh.nlmsg_len as usize;
        if msg_len < hdr_size || msg_len > len - offset {
            break;
        }
        if nh.nlmsg_type == libc::NLMSG_DONE as u16 || nh.nlmsg_type == libc::NLMSG_ERROR as u16 {
            return false;
        }
        if nh.nlmsg_type == msg_type {
            on_msg(buf, offset, msg_len);
        }
        offset += (msg_len + 3) & !3;
    }
    true // continue receiving
}

/// Iterate rtattr entries within a netlink message payload.
///
/// # Safety
/// `buf[start..end]` must contain valid rtattr entries.
unsafe fn for_each_rtattr(
    buf: &[u8],
    start: usize,
    end: usize,
    mut on_attr: impl FnMut(&Rtattr, &[u8]),
) {
    // Walk rtattrs in `buf[start..end]`. For each, hand the callback
    // the header AND a slice covering its payload — already bounds-
    // checked against `end`, so callbacks can never read past the
    // message. A truncated tail (rta_len < 4, or rta_len reaching
    // past `end`) ends the walk; netlink dumps end on padding, so
    // this is the normal exit too.
    let mut off = start;
    while off + 4 <= end {
        let rta = unsafe { &*(buf.as_ptr().add(off) as *const Rtattr) };
        let rta_len = rta.rta_len as usize;
        if rta_len < 4 || off + rta_len > end {
            break;
        }
        on_attr(rta, &buf[off + 4..off + rta_len]);
        off += (rta_len + 3) & !3;
    }
}

fn check_netlink_getlink() -> CheckOutput {
    let fd = match open_netlink() {
        Ok(fd) => fd,
        Err(out) => return out,
    };

    unsafe {
        #[repr(C)]
        struct Req {
            nlh: libc::nlmsghdr,
            ifm: Ifinfomsg,
        }
        let mut req: Req = std::mem::zeroed();
        req.nlh.nlmsg_len = std::mem::size_of::<Req>() as u32;
        req.nlh.nlmsg_type = libc::RTM_GETLINK;
        req.nlh.nlmsg_flags = (libc::NLM_F_REQUEST | libc::NLM_F_DUMP) as u16;
        req.nlh.nlmsg_seq = 1;

        if libc::send(
            fd,
            std::ptr::from_ref(&req).cast(),
            req.nlh.nlmsg_len as usize,
            0,
        ) < 0
        {
            let e = last_os_error();
            libc::close(fd);
            return CheckOutput::fail(format!("send error: {e}"));
        }

        let mut buf = AlignedBytes::<32768>::zeroed();
        let mut all = Vec::new();
        let mut vpn = Vec::new();
        let hdr_plus_ifinfo =
            std::mem::size_of::<libc::nlmsghdr>() + std::mem::size_of::<Ifinfomsg>();

        for _ in 0..MAX_NETLINK_RECV_ITERS {
            let len = netlink_recv(fd, &mut buf.0);
            if len <= 0 {
                break;
            }
            let cont = parse_netlink_msgs(
                &buf.0,
                len as usize,
                libc::RTM_NEWLINK,
                |b, offset, msg_len| {
                    let data_start = offset + hdr_plus_ifinfo;
                    let msg_end = offset + msg_len;
                    for_each_rtattr(b, data_start, msg_end, |rta, payload| {
                        if rta.rta_type == IFLA_IFNAME && !payload.is_empty() {
                            // IFLA_IFNAME is a NUL-terminated string;
                            // payload was bounds-checked by for_each_rtattr.
                            let name = cstr_to_str(payload.as_ptr() as *const libc::c_char);
                            if is_vpn_iface(&name) {
                                vpn.push(name.clone());
                            }
                            all.push(name);
                        }
                    });
                },
            );
            if !cont {
                break;
            }
        }
        libc::close(fd);

        format_iface_result(
            &all,
            &vpn,
            &format!("{} interfaces via netlink:", all.len()),
        )
    }
}

fn check_netlink_getroute() -> CheckOutput {
    let fd = match open_netlink() {
        Ok(fd) => fd,
        Err(out) => return out,
    };

    unsafe {
        #[repr(C)]
        struct Req {
            nlh: libc::nlmsghdr,
            rtm: Rtmsg,
        }
        let mut req: Req = std::mem::zeroed();
        req.nlh.nlmsg_len = std::mem::size_of::<Req>() as u32;
        req.nlh.nlmsg_type = libc::RTM_GETROUTE;
        req.nlh.nlmsg_flags = (libc::NLM_F_REQUEST | libc::NLM_F_DUMP) as u16;
        req.nlh.nlmsg_seq = 1;

        if libc::send(
            fd,
            std::ptr::from_ref(&req).cast(),
            req.nlh.nlmsg_len as usize,
            0,
        ) < 0
        {
            let e = last_os_error();
            libc::close(fd);
            return CheckOutput::fail(format!("send error: {e}"));
        }

        let mut buf = AlignedBytes::<32768>::zeroed();
        let mut vpn = Vec::new();
        let mut total = 0u32;
        let hdr_plus_rtmsg = std::mem::size_of::<libc::nlmsghdr>() + std::mem::size_of::<Rtmsg>();

        for _ in 0..MAX_NETLINK_RECV_ITERS {
            let len = netlink_recv(fd, &mut buf.0);
            if len <= 0 {
                break;
            }
            let cont = parse_netlink_msgs(
                &buf.0,
                len as usize,
                libc::RTM_NEWROUTE,
                |b, offset, msg_len| {
                    total += 1;
                    let data_start = offset + hdr_plus_rtmsg;
                    let msg_end = offset + msg_len;
                    for_each_rtattr(b, data_start, msg_end, |rta, payload| {
                        if rta.rta_type == RTA_OIF && payload.len() >= 4 {
                            let ifindex = i32::from_ne_bytes(payload[..4].try_into().unwrap());
                            let mut ifname_buf = [0u8; libc::IF_NAMESIZE];
                            let ptr = libc::if_indextoname(
                                ifindex as u32,
                                ifname_buf.as_mut_ptr().cast(),
                            );
                            if !ptr.is_null() {
                                let name = cstr_to_str(ptr);
                                if is_vpn_iface(&name) {
                                    vpn.push(name);
                                }
                            }
                        }
                    });
                },
            );
            if !cont {
                break;
            }
        }
        libc::close(fd);

        if vpn.is_empty() {
            CheckOutput::pass(format!("{total} routes, no VPN"))
        } else {
            CheckOutput::fail(format!("VPN routes via [{}]", join_list(&vpn)))
        }
    }
}

/// Send an RTM_GETRULE dump for `family` on an already-bound `fd` and invoke
/// `on_rule(buf, offset, msg_len)` for each RTM_NEWRULE message. The kernel's
/// rule dump is per-family, and Android installs the per-app VPN policy rules for
/// BOTH IPv4 and IPv6 — so callers dump `AF_INET` and `AF_INET6` on the same
/// socket (distinct seq) and merge, else an IPv6 rule set is invisible. Returns
/// the OS error string on a send failure; a short read / NLMSG_DONE ends the dump.
///
/// # Safety
/// `fd` must be a bound NETLINK_ROUTE socket; `buf` must be 8-aligned and large
/// enough for a dump chunk (the callers pass a 32 KiB `AlignedBytes`).
unsafe fn dump_fib_rules(
    fd: i32,
    family: u8,
    seq: u32,
    buf: &mut [u8],
    mut on_rule: impl FnMut(&[u8], usize, usize),
) -> Result<(), String> {
    unsafe {
        #[repr(C)]
        struct Req {
            nlh: libc::nlmsghdr,
            frh: Rtmsg,
        }
        let mut req: Req = std::mem::zeroed();
        req.nlh.nlmsg_len = std::mem::size_of::<Req>() as u32;
        req.nlh.nlmsg_type = libc::RTM_GETRULE;
        req.nlh.nlmsg_flags = (libc::NLM_F_REQUEST | libc::NLM_F_DUMP) as u16;
        req.nlh.nlmsg_seq = seq;
        req.frh.rtm_family = family;

        if libc::send(
            fd,
            std::ptr::from_ref(&req).cast(),
            req.nlh.nlmsg_len as usize,
            0,
        ) < 0
        {
            return Err(last_os_error());
        }

        for _ in 0..MAX_NETLINK_RECV_ITERS {
            let len = netlink_recv(fd, buf);
            if len <= 0 {
                break;
            }
            let cont = parse_netlink_msgs(buf, len as usize, libc::RTM_NEWRULE, &mut on_rule);
            if !cont {
                break;
            }
        }
        Ok(())
    }
}

/// The address families whose policy-rule dumps together cover an Android VPN's
/// per-app routing (v4 + v6). Iterated by both the diagnostic and the gate.
const RULE_FAMILIES: [u8; 2] = [libc::AF_INET as u8, libc::AF_INET6 as u8];

/// RTM_GETRULE — policy routing rules. On Android the VPN's per-app policy is
/// expressed as `ip rule` entries steering a UID range into the interface's own
/// routing table (`... uidrange 10236-10236 lookup tun0`), plus rules that name
/// the VPN interface directly (`iif/oif tun0`). Reading those rules reveals the
/// VPN even when /proc/net/route (main table only) shows nothing. The kernel
/// `fib_nl_fill_rule` hook trims exactly these from a target's dump; this probe
/// mirrors that predicate so a working backend reads as "hidden" and root
/// ground-truth (not a target → hook inert) reads as "leak".
fn check_netlink_getrule() -> CheckOutput {
    check_netlink_getrule_uid(unsafe { libc::getuid() })
}

/// RTM_GETRULE for a specific uid. The `vhprobe --uid` self-routing gate reuses
/// this to answer "is <uid> routed through the VPN?" — a matching policy rule
/// (or a VPN-named iif/oif) means yes.
fn check_netlink_getrule_uid(myuid: u32) -> CheckOutput {
    // rtattr types and standard table ids (linux/fib_rules.h, linux/rtnetlink.h).
    const FRA_IIFNAME: u16 = 3;
    const FRA_TABLE: u16 = 15;
    const FRA_OIFNAME: u16 = 17;
    const FRA_UID_RANGE: u16 = 20;
    const RT_TABLE_DEFAULT: u32 = 253;
    const RT_TABLE_MAIN: u32 = 254;
    const RT_TABLE_LOCAL: u32 = 255;

    let fd = match open_netlink() {
        Ok(fd) => fd,
        Err(out) => return out,
    };

    let mut leaks: Vec<String> = Vec::new();
    let mut total = 0u32;

    unsafe {
        let mut buf = AlignedBytes::<32768>::zeroed();
        let hdr_plus_rtmsg = std::mem::size_of::<libc::nlmsghdr>() + std::mem::size_of::<Rtmsg>();

        // fib_rule_hdr shares Rtmsg's 12-byte layout (family + 7 u8 + u32 flags).
        let mut on_rule = |b: &[u8], offset: usize, msg_len: usize| {
            total += 1;
            let frh = &*(b
                .as_ptr()
                .add(offset + std::mem::size_of::<libc::nlmsghdr>())
                as *const Rtmsg);
            // The low byte of the table id lives in the header; the full u32
            // arrives in FRA_TABLE (Android tun tables are > 255).
            let mut table = frh.rtm_table as u32;
            let mut uid_lo = 0u32;
            let mut uid_hi = 0u32;
            let mut has_uidrange = false;
            let mut iface_hit: Option<String> = None;
            for_each_rtattr(
                b,
                offset + hdr_plus_rtmsg,
                offset + msg_len,
                |rta, payload| match rta.rta_type {
                    FRA_IIFNAME | FRA_OIFNAME if !payload.is_empty() => {
                        let name = cstr_to_str(payload.as_ptr() as *const libc::c_char);
                        if is_vpn_iface(&name) {
                            iface_hit = Some(name);
                        }
                    }
                    FRA_TABLE if payload.len() >= 4 => {
                        table = u32::from_ne_bytes(payload[..4].try_into().unwrap());
                    }
                    FRA_UID_RANGE if payload.len() >= 8 => {
                        uid_lo = u32::from_ne_bytes(payload[..4].try_into().unwrap());
                        uid_hi = u32::from_ne_bytes(payload[4..8].try_into().unwrap());
                        has_uidrange = true;
                    }
                    _ => {}
                },
            );
            // Mirror fib_nl_fill_rule: a rule leaks the VPN if it names a VPN
            // interface, or steers this very UID into a non-standard table (the
            // per-app tun policy rule). The 0..u32::MAX range is the catch-all
            // default rule, not a VPN rule.
            if let Some(name) = iface_hit {
                leaks.push(format!("iface {name}"));
            } else if has_uidrange
                && uid_lo <= myuid
                && myuid <= uid_hi
                && !(uid_lo == 0 && uid_hi == u32::MAX)
                && table != RT_TABLE_MAIN
                && table != RT_TABLE_LOCAL
                && table != RT_TABLE_DEFAULT
                && table > 100
            {
                leaks.push(format!("uid {myuid} → table {table}"));
            }
        };

        // Dump v4 and v6 rules on the same socket (distinct seq) and merge.
        for (i, family) in RULE_FAMILIES.into_iter().enumerate() {
            if let Err(e) = dump_fib_rules(fd, family, (i + 1) as u32, &mut buf.0, &mut on_rule) {
                libc::close(fd);
                return CheckOutput::fail(format!("send error: {e}"));
            }
        }
        libc::close(fd);
    }

    if leaks.is_empty() {
        CheckOutput::pass(format!("{total} policy rules, none reveal VPN"))
    } else {
        CheckOutput::fail(format!("VPN policy rule(s): {}", join_list(&leaks)))
    }
}

fn check_sys_class_net() -> CheckOutput {
    match std::fs::read_dir("/sys/class/net") {
        Err(e) => {
            if is_selinux_denial(&e) {
                CheckOutput::selinux_blocked(format!("access denied by SELinux ({e})"))
            } else {
                CheckOutput::fail(format!("cannot open /sys/class/net: {e}"))
            }
        }
        Ok(entries) => {
            let mut all = Vec::new();
            let mut vpn = Vec::new();
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if is_vpn_iface(&name) {
                    vpn.push(name.clone());
                }
                all.push(name);
            }
            format_iface_result(&all, &vpn, &format!("[{}]:", all.len()))
        }
    }
}

// ── /proc/net/* wrappers: one uniffi export per path so the Kotlin side
//    keeps a thin `checkProcNetFoo(): CheckOutput` surface instead of
//    pushing path strings across the FFI. ──────────────────────────────

fn check_proc_net_route() -> CheckOutput {
    check_proc_file("/proc/net/route")
}

fn check_proc_net_if_inet6() -> CheckOutput {
    check_proc_file("/proc/net/if_inet6")
}

fn check_proc_net_ipv6_route() -> CheckOutput {
    check_proc_file("/proc/net/ipv6_route")
}

// NOTE: /proc/net/{tcp,tcp6,udp,udp6,fib_trie} are intentionally NOT probed
// here. Those files expose the VPN only as a hex *local address*, never as an
// interface name, so a name-matching probe could only ever report a green pass
// regardless of an actual leak. From inside the targeted (and thus self-hidden)
// process we have no reference tunnel address to decode and compare, so there is
// no honest check to run; the address-leak vector on these files is covered by
// zygisk's tcp/tcp6/udp socket-table filters, not by a self-diagnostic.

fn check_proc_net_dev() -> CheckOutput {
    check_proc_file("/proc/net/dev")
}

// ── Registry + JSON transport ─────────────────────────────────────────────
// One code path, two transports: the JNI export runs this in the app's own
// process (app view — real uid + SELinux domain + zygisk/kernel hooks); the
// `vhprobe` bin runs it as root (ground truth — uid 0 is not a hook target).
// Both emit the same JSON; the app classifies by comparing per-`id`.

#[derive(serde::Serialize)]
struct CheckJson {
    /// Stable id — matches NativeChecks.kt `NativeCheckSpec.id`.
    id: &'static str,
    status: CheckStatus,
    detail: String,
}

fn run_all() -> Vec<CheckJson> {
    fn j(id: &'static str, out: CheckOutput) -> CheckJson {
        CheckJson {
            id,
            status: out.status,
            detail: out.detail,
        }
    }
    vec![
        j("ioctl_flags", check_ioctl_siocgifflags()),
        j("ioctl_mtu", check_ioctl_siocgifmtu()),
        j("ioctl_conf", check_ioctl_siocgifconf()),
        j("getifaddrs", check_getifaddrs()),
        j("netlink_getlink", check_netlink_getlink()),
        j("netlink_getroute", check_netlink_getroute()),
        j("netlink_getrule", check_netlink_getrule()),
        j("proc_route", check_proc_net_route()),
        j("proc_ipv6_route", check_proc_net_ipv6_route()),
        j("proc_if_inet6", check_proc_net_if_inet6()),
        j("proc_dev", check_proc_net_dev()),
        j("sys_class_net", check_sys_class_net()),
    ]
}

/// Run every native probe in display order and serialize to a JSON array of
/// `{id, status, detail}`. Public so both the JNI export and the `vhprobe` bin
/// call the exact same code.
pub fn run_all_json() -> String {
    serde_json::to_string(&run_all()).unwrap_or_else(|_| "[]".to_string())
}

#[derive(serde::Serialize)]
struct SelfRouted {
    uid: u32,
    routed: bool,
    detail: String,
}

/// The self-in-tunnel gate: is `uid` routed through the VPN? Run as root (not a
/// hook target) so the answer is the unfiltered ground truth. Serialized as
/// `{uid, routed, detail}`.
///
/// This is NOT the diagnostic predicate (`check_netlink_getrule`, which mirrors
/// the kernel hook's broad "any non-standard table > 100" rule). Policy routing
/// steers every UID into *some* per-network table (wlan0, rmnet, tun0 — all
/// non-standard), so the broad predicate would call any online UID "routed".
/// The gate must pin the VPN table specifically: first learn which table id a
/// rule egresses to the VPN interface (`oif tun0`), then ask whether a uidrange
/// rule steers this UID into exactly that table.
pub fn self_routed_json(uid: u32) -> String {
    let (routed, detail) = uid_routed_through_vpn(uid);
    let sr = SelfRouted {
        uid,
        routed,
        detail,
    };
    serde_json::to_string(&sr).unwrap_or_else(|_| "{}".to_string())
}

struct GateRule {
    table: u32,
    uid_lo: u32,
    uid_hi: u32,
    has_uidrange: bool,
    oif_vpn: bool,
}

fn uid_routed_through_vpn(myuid: u32) -> (bool, String) {
    const FRA_TABLE: u16 = 15;
    const FRA_OIFNAME: u16 = 17;
    const FRA_UID_RANGE: u16 = 20;

    let fd = match open_netlink() {
        Ok(fd) => fd,
        Err(out) => return (false, out.detail),
    };

    let mut rules: Vec<GateRule> = Vec::new();

    unsafe {
        let mut buf = AlignedBytes::<32768>::zeroed();
        let hdr_plus_rtmsg = std::mem::size_of::<libc::nlmsghdr>() + std::mem::size_of::<Rtmsg>();

        let mut on_rule = |b: &[u8], offset: usize, msg_len: usize| {
            let frh = &*(b
                .as_ptr()
                .add(offset + std::mem::size_of::<libc::nlmsghdr>())
                as *const Rtmsg);
            let mut r = GateRule {
                table: frh.rtm_table as u32,
                uid_lo: 0,
                uid_hi: 0,
                has_uidrange: false,
                oif_vpn: false,
            };
            for_each_rtattr(
                b,
                offset + hdr_plus_rtmsg,
                offset + msg_len,
                |rta, payload| match rta.rta_type {
                    FRA_OIFNAME if !payload.is_empty() => {
                        let name = cstr_to_str(payload.as_ptr() as *const libc::c_char);
                        if is_vpn_iface(&name) {
                            r.oif_vpn = true;
                        }
                    }
                    FRA_TABLE if payload.len() >= 4 => {
                        r.table = u32::from_ne_bytes(payload[..4].try_into().unwrap());
                    }
                    FRA_UID_RANGE if payload.len() >= 8 => {
                        r.uid_lo = u32::from_ne_bytes(payload[..4].try_into().unwrap());
                        r.uid_hi = u32::from_ne_bytes(payload[4..8].try_into().unwrap());
                        r.has_uidrange = true;
                    }
                    _ => {}
                },
            );
            rules.push(r);
        };

        // v4 + v6: a VPN steers the uid into a tun table for both families; either
        // membership means routed, so merge both dumps before deciding.
        for (i, family) in RULE_FAMILIES.into_iter().enumerate() {
            if let Err(e) = dump_fib_rules(fd, family, (i + 1) as u32, &mut buf.0, &mut on_rule) {
                libc::close(fd);
                return (false, format!("send error: {e}"));
            }
        }
        libc::close(fd);
    }

    // Pass 1: the VPN egress table id(s) — tables a rule routes out via `oif tun0`.
    let vpn_tables: Vec<u32> = rules
        .iter()
        .filter(|r| r.oif_vpn)
        .map(|r| r.table)
        .collect();

    // Pass 2: is this UID steered into exactly a VPN table by a uidrange rule?
    // Exclude the universal 0..u32::MAX catch-all (not a per-app membership rule).
    let routed = rules.iter().any(|r| {
        r.has_uidrange
            && r.uid_lo <= myuid
            && myuid <= r.uid_hi
            && !(r.uid_lo == 0 && r.uid_hi == u32::MAX)
            && vpn_tables.contains(&r.table)
    });

    let detail = if vpn_tables.is_empty() {
        format!("no VPN egress rule found for uid {myuid}")
    } else if routed {
        format!("uid {myuid} routed into VPN table(s) {vpn_tables:?}")
    } else {
        format!("uid {myuid} not in any VPN table(s) {vpn_tables:?}")
    };
    (routed, detail)
}

/// In-process (app-view) entry. Class/package must match the Kotlin
/// `object dev.okhsunrog.vpnhide.checks.NativeProbe`.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_okhsunrog_vpnhide_checks_NativeProbe_runAllChecksJson<'local>(
    mut env: jni::EnvUnowned<'local>,
    _class: jni::objects::JClass<'local>,
) -> jni::objects::JString<'local> {
    env.with_env(
        |env| -> jni::errors::Result<jni::objects::JString<'local>> {
            env.new_string(run_all_json())
        },
    )
    .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}
