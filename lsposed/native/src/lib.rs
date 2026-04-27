mod generated;

use std::ffi::CStr;
use std::io::ErrorKind;

use crate::generated::iface_lists::matches_vpn;

uniffi::setup_scaffolding!();

// ── Probe outcome types — crossing the FFI ───────────────────────────

#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(uniffi::Record, Debug, Clone)]
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

#[uniffi::export]
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

#[uniffi::export]
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

#[uniffi::export]
fn check_ioctl_siocgifconf() -> CheckOutput {
    unsafe {
        with_inet_dgram_socket(|fd| {
            let mut buf = [0u8; 4096];
            let mut ifc: libc::ifconf = std::mem::zeroed();
            ifc.ifc_len = buf.len() as libc::c_int;
            ifc.ifc_ifcu.ifcu_buf = buf.as_mut_ptr().cast();

            if libc::ioctl(fd, libc::SIOCGIFCONF as _, &mut ifc) < 0 {
                let e = last_os_error();
                return CheckOutput::fail(format!("ioctl error: {e}"));
            }

            let count = ifc.ifc_len as usize / std::mem::size_of::<libc::ifreq>();
            let reqs = std::slice::from_raw_parts(buf.as_ptr() as *const libc::ifreq, count);

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

#[uniffi::export]
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
    // Walk rtattrs in `buf[start..end]`. Drop the upper-bound check
    // since `while off + 4 <= end` already keeps the header read in
    // range — saves a branch on a hot path during getlink dumps.
    let mut off = start;
    while off + 4 <= end {
        let rta = unsafe { &*(buf.as_ptr().add(off) as *const Rtattr) };
        let rta_len = rta.rta_len as usize;
        if rta_len < 4 {
            break;
        }
        on_attr(rta, &buf[off + 4..off + rta_len]);
        off += (rta_len + 3) & !3;
    }
}

#[uniffi::export]
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

        let mut buf = [0u8; 32768];
        let mut all = Vec::new();
        let mut vpn = Vec::new();
        let hdr_plus_ifinfo =
            std::mem::size_of::<libc::nlmsghdr>() + std::mem::size_of::<Ifinfomsg>();

        loop {
            let len = netlink_recv(fd, &mut buf);
            if len <= 0 {
                break;
            }
            let cont = parse_netlink_msgs(
                &buf,
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

#[uniffi::export]
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

        let mut buf = [0u8; 32768];
        let mut vpn = Vec::new();
        let mut total = 0u32;
        let hdr_plus_rtmsg = std::mem::size_of::<libc::nlmsghdr>() + std::mem::size_of::<Rtmsg>();

        loop {
            let len = netlink_recv(fd, &mut buf);
            if len <= 0 {
                break;
            }
            let cont = parse_netlink_msgs(
                &buf,
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

#[uniffi::export]
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

#[uniffi::export]
fn check_proc_net_route() -> CheckOutput {
    check_proc_file("/proc/net/route")
}

#[uniffi::export]
fn check_proc_net_if_inet6() -> CheckOutput {
    check_proc_file("/proc/net/if_inet6")
}

#[uniffi::export]
fn check_proc_net_ipv6_route() -> CheckOutput {
    check_proc_file("/proc/net/ipv6_route")
}

#[uniffi::export]
fn check_proc_net_tcp() -> CheckOutput {
    check_proc_file("/proc/net/tcp")
}

#[uniffi::export]
fn check_proc_net_tcp6() -> CheckOutput {
    check_proc_file("/proc/net/tcp6")
}

#[uniffi::export]
fn check_proc_net_udp() -> CheckOutput {
    check_proc_file("/proc/net/udp")
}

#[uniffi::export]
fn check_proc_net_udp6() -> CheckOutput {
    check_proc_file("/proc/net/udp6")
}

#[uniffi::export]
fn check_proc_net_dev() -> CheckOutput {
    check_proc_file("/proc/net/dev")
}

#[uniffi::export]
fn check_proc_net_fib_trie() -> CheckOutput {
    check_proc_file("/proc/net/fib_trie")
}
