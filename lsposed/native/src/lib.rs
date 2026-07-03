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
    /// Probe ran and saw nothing VPN-shaped, or was legitimately blocked
    /// (SELinux denial, ENODEV, etc.) — both outcomes confirm the VPN is
    /// hidden from this surface.
    Pass,
    /// Probe surfaced VPN-shaped data the kmod / zygisk should have hidden.
    Fail,
    /// App has no network permission, so the probe couldn't run at all.
    /// Reported separately from Pass/Fail so the UI can tell the user to
    /// enable network access before trusting the results.
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
                return CheckOutput::pass(format!(
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
                CheckOutput::pass(format!("netlink socket denied by SELinux ({e})"))
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
                CheckOutput::pass(format!(
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

fn check_sys_class_net() -> CheckOutput {
    match std::fs::read_dir("/sys/class/net") {
        Err(e) => {
            if is_selinux_denial(&e) {
                CheckOutput::pass(format!("access denied by SELinux ({e})"))
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

/// In-process (app-view) entry. Class/package must match the Kotlin
/// `object dev.okhsunrog.vpnhide.checks.NativeProbe`.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_okhsunrog_vpnhide_checks_NativeProbe_runAllChecksJson<'local>(
    mut env: jni::EnvUnowned<'local>,
    _class: jni::objects::JClass<'local>,
) -> jni::objects::JString<'local> {
    env.with_env(|env| -> jni::errors::Result<jni::objects::JString<'local>> {
        env.new_string(run_all_json())
    })
    .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}
