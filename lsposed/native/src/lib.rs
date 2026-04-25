mod generated;

use jni::JNIEnv;
use jni::objects::JClass;
use jni::sys::jstring;
use std::ffi::CStr;
use std::io::ErrorKind;

use crate::generated::iface_lists::matches_vpn;

fn is_vpn_iface(name: &str) -> bool {
    matches_vpn(name.as_bytes())
}

fn logi(msg: &str) {
    log::info!("{msg}");
}

fn result_to_jstring(env: &mut JNIEnv, s: &str) -> jstring {
    env.new_string(s)
        .map(|j| j.into_raw())
        .unwrap_or(std::ptr::null_mut())
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

fn format_iface_result(all: &[String], vpn: &[String], context: &str) -> String {
    if vpn.is_empty() {
        format!("PASS: {context} [{list}], no VPN", list = join_list(all))
    } else {
        format!(
            "FAIL: VPN interfaces [{vpn}] in [{all}]",
            vpn = join_list(vpn),
            all = join_list(all),
        )
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

fn check_ioctl_siocgifflags() -> String {
    logi("=== CHECK: ioctl SIOCGIFFLAGS on tun0 ===");
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
        if fd < 0 {
            let err = last_os_errno();
            if err == libc::ECONNREFUSED {
                return "NETWORK_BLOCKED: socket() returned ECONNREFUSED — network access disabled for this app".into();
            }
            return format!("FAIL: cannot create socket: {}", last_os_error());
        }

        let mut ifr: libc::ifreq = std::mem::zeroed();
        let name = b"tun0\0";
        ifr.ifr_name[..name.len()].copy_from_slice(&name.map(|b| b as libc::c_char));

        let ret = libc::ioctl(fd, libc::SIOCGIFFLAGS as i32, &ifr);
        let err = last_os_errno();
        libc::close(fd);

        if ret < 0 {
            if err == libc::ENODEV {
                "PASS: ioctl(tun0, SIOCGIFFLAGS) returned ENODEV — interface not visible".into()
            } else if err == libc::ENXIO {
                "PASS: ioctl(tun0, SIOCGIFFLAGS) returned ENXIO — interface not visible".into()
            } else {
                format!("FAIL: ioctl returned error {err} ({})", last_os_error())
            }
        } else {
            let flags = ifr.ifr_ifru.ifru_flags as u32;
            format!(
                "FAIL: tun0 is visible! flags=0x{flags:x} (IFF_UP={}, IFF_RUNNING={})",
                u8::from(flags & libc::IFF_UP as u32 != 0),
                u8::from(flags & libc::IFF_RUNNING as u32 != 0),
            )
        }
    }
}

fn check_ioctl_siocgifmtu() -> String {
    logi("=== CHECK: ioctl SIOCGIFMTU on tun0 ===");
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
        if fd < 0 {
            let err = last_os_errno();
            if err == libc::ECONNREFUSED {
                return "NETWORK_BLOCKED: socket() returned ECONNREFUSED — network access disabled for this app".into();
            }
            return format!("FAIL: cannot create socket: {}", last_os_error());
        }

        let mut ifr: libc::ifreq = std::mem::zeroed();
        let name = b"tun0\0";
        ifr.ifr_name[..name.len()].copy_from_slice(&name.map(|b| b as libc::c_char));

        let ret = libc::ioctl(fd, libc::SIOCGIFMTU as i32, &ifr);
        let err = last_os_errno();
        libc::close(fd);

        if ret < 0 {
            if err == libc::ENODEV || err == libc::ENXIO {
                "PASS: ioctl(tun0, SIOCGIFMTU) returned ENODEV — interface not visible".into()
            } else {
                format!("FAIL: ioctl returned error {err} ({})", last_os_error())
            }
        } else {
            let mtu = ifr.ifr_ifru.ifru_mtu;
            format!("FAIL: tun0 is visible! MTU={mtu}")
        }
    }
}

fn check_ioctl_siocgifconf() -> String {
    logi("=== CHECK: ioctl SIOCGIFCONF enumeration ===");
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
        if fd < 0 {
            let err = last_os_errno();
            if err == libc::ECONNREFUSED {
                return "NETWORK_BLOCKED: socket() returned ECONNREFUSED — network access disabled for this app".into();
            }
            return format!("FAIL: cannot create socket: {}", last_os_error());
        }

        let mut buf = [0u8; 4096];
        let mut ifc: libc::ifconf = std::mem::zeroed();
        ifc.ifc_len = buf.len() as libc::c_int;
        ifc.ifc_ifcu.ifcu_buf = buf.as_mut_ptr().cast();

        if libc::ioctl(fd, libc::SIOCGIFCONF as i32, &mut ifc) < 0 {
            let e = last_os_error();
            libc::close(fd);
            return format!("FAIL: ioctl error: {e}");
        }
        libc::close(fd);

        let count = ifc.ifc_len as usize / std::mem::size_of::<libc::ifreq>();
        let reqs = std::slice::from_raw_parts(buf.as_ptr() as *const libc::ifreq, count);

        let mut all = Vec::new();
        let mut vpn = Vec::new();
        for req in reqs {
            let name = cstr_to_str(req.ifr_name.as_ptr());
            logi(&format!("  SIOCGIFCONF: interface '{name}'"));
            if is_vpn_iface(&name) {
                vpn.push(name.clone());
            }
            all.push(name);
        }

        format_iface_result(&all, &vpn, &format!("{count} interfaces visible:"))
    }
}

fn check_getifaddrs() -> String {
    logi("=== CHECK: getifaddrs() enumeration ===");
    unsafe {
        let mut addrs: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut addrs) != 0 {
            return format!("FAIL: getifaddrs error: {}", last_os_error());
        }

        let mut all: Vec<String> = Vec::new();
        let mut vpn: Vec<String> = Vec::new();
        let mut ifa = addrs;
        while !ifa.is_null() {
            let entry = &*ifa;
            if !entry.ifa_name.is_null() {
                let name = cstr_to_str(entry.ifa_name);
                if !all.contains(&name) {
                    let family = if entry.ifa_addr.is_null() {
                        -1
                    } else {
                        i32::from((*entry.ifa_addr).sa_family)
                    };
                    logi(&format!(
                        "  getifaddrs: interface '{name}' (family={family}, flags=0x{:x})",
                        entry.ifa_flags
                    ));
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

fn check_proc_file(path: &str) -> String {
    logi(&format!("=== CHECK: {path} (native read) ==="));
    match std::fs::read_to_string(path) {
        Err(e) => {
            if is_selinux_denial(&e) {
                return format!("PASS: access denied by SELinux ({e}) — app cannot read {path}");
            }
            format!("FAIL: cannot open {path}: {e}")
        }
        Ok(content) => {
            let mut total = 0;
            let mut vpn_lines = Vec::new();
            for line in content.lines() {
                if line.is_empty() {
                    continue;
                }
                total += 1;
                logi(&format!("  {path} line: {}", &line[..line.len().min(120)]));
                // /proc/net/route and /proc/net/ipv6_route are
                // whitespace-separated; an iface name is one of the
                // tokens. Token-by-token check avoids the false
                // positive of substring "tun" or "gre" appearing
                // inside a hex-encoded IP address.
                if line.split_whitespace().any(is_vpn_iface) {
                    vpn_lines.push(line[..line.len().min(80)].to_string());
                }
            }
            if vpn_lines.is_empty() {
                format!("PASS: {total} lines in {path}, no VPN entries")
            } else {
                let details: String = vpn_lines.iter().map(|l| format!("\n  {l}")).collect();
                format!("FAIL: {} VPN lines in {path}:{details}", vpn_lines.len())
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

fn open_netlink() -> Result<i32, String> {
    unsafe {
        let fd = libc::socket(libc::AF_NETLINK, libc::SOCK_RAW, libc::NETLINK_ROUTE);
        if fd < 0 {
            let e = std::io::Error::last_os_error();
            return Err(if is_selinux_denial(&e) {
                format!("PASS: netlink socket denied by SELinux ({e})")
            } else {
                format!("FAIL: cannot create netlink socket: {e}")
            });
        }

        let mut sa: libc::sockaddr_nl = std::mem::zeroed();
        sa.nl_family = libc::AF_NETLINK as u16;
        let sa_len = std::mem::size_of_val(&sa) as libc::socklen_t;
        if libc::bind(fd, std::ptr::from_ref(&sa).cast(), sa_len) < 0 {
            let e = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(if is_selinux_denial(&e) {
                format!(
                    "PASS: netlink bind denied by SELinux ({e}) — app cannot enumerate interfaces"
                )
            } else {
                format!("FAIL: bind error: {e}")
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
    mut on_attr: impl FnMut(&Rtattr, usize),
) {
    let mut off = start;
    while off + 4 <= end {
        let rta = unsafe { &*(buf.as_ptr().add(off) as *const Rtattr) };
        if rta.rta_len < 4 {
            break;
        }
        on_attr(rta, off);
        off += (rta.rta_len as usize + 3) & !3;
    }
}

fn check_netlink_getlink() -> String {
    logi("=== CHECK: netlink RTM_GETLINK dump ===");
    let fd = match open_netlink() {
        Ok(fd) => fd,
        Err(msg) => return msg,
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
            return format!("FAIL: send error: {e}");
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
                    for_each_rtattr(b, data_start, msg_end, |rta, rta_off| {
                        if rta.rta_type == IFLA_IFNAME {
                            let name =
                                cstr_to_str(b.as_ptr().add(rta_off + 4) as *const libc::c_char);
                            logi(&format!("  netlink RTM_NEWLINK: interface '{name}'"));
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

/// Same as check_netlink_getlink but uses recv (→ recvfrom) instead of recvmsg.
/// Temporary check to verify the recvfrom hook works.
fn check_netlink_getlink_recv() -> String {
    logi("=== CHECK: netlink RTM_GETLINK via recv ===");
    let fd = match open_netlink() {
        Ok(fd) => fd,
        Err(msg) => return msg,
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
            return format!("FAIL: send error: {e}");
        }

        let mut buf = [0u8; 32768];
        let mut all = Vec::new();
        let mut vpn = Vec::new();
        let hdr_plus_ifinfo =
            std::mem::size_of::<libc::nlmsghdr>() + std::mem::size_of::<Ifinfomsg>();

        loop {
            let len = libc::recv(fd, buf.as_mut_ptr().cast(), buf.len(), 0);
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
                    for_each_rtattr(b, data_start, msg_end, |rta, rta_off| {
                        if rta.rta_type == IFLA_IFNAME {
                            let name =
                                cstr_to_str(b.as_ptr().add(rta_off + 4) as *const libc::c_char);
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
            &format!("{} interfaces via netlink (recv):", all.len()),
        )
    }
}

fn check_netlink_getroute() -> String {
    logi("=== CHECK: netlink RTM_GETROUTE dump ===");
    let fd = match open_netlink() {
        Ok(fd) => fd,
        Err(msg) => return msg,
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
            return format!("FAIL: send error: {e}");
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
                    for_each_rtattr(b, data_start, msg_end, |rta, rta_off| {
                        if rta.rta_type == RTA_OIF {
                            let ifindex = *(b.as_ptr().add(rta_off + 4) as *const i32);
                            let mut ifname_buf = [0u8; libc::IF_NAMESIZE];
                            let ptr = libc::if_indextoname(
                                ifindex as u32,
                                ifname_buf.as_mut_ptr().cast(),
                            );
                            if !ptr.is_null() {
                                let name = cstr_to_str(ptr);
                                if is_vpn_iface(&name) {
                                    logi(&format!("  RTM_GETROUTE: VPN route via '{name}'"));
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
            format!("PASS: {total} routes, no VPN")
        } else {
            format!("FAIL: VPN routes via [{}]", join_list(&vpn))
        }
    }
}

fn check_sys_class_net() -> String {
    logi("=== CHECK: /sys/class/net/ directory ===");
    match std::fs::read_dir("/sys/class/net") {
        Err(e) => {
            if is_selinux_denial(&e) {
                format!("PASS: access denied by SELinux ({e})")
            } else {
                format!("FAIL: cannot open /sys/class/net: {e}")
            }
        }
        Ok(entries) => {
            let mut all = Vec::new();
            let mut vpn = Vec::new();
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                logi(&format!("  /sys/class/net: '{name}'"));
                if is_vpn_iface(&name) {
                    vpn.push(name.clone());
                }
                all.push(name);
            }
            format_iface_result(&all, &vpn, &format!("[{}]:", all.len()))
        }
    }
}

// ── JNI exports ──────────────────────────────────────────────────────

// ── JNI exports ──────────────────────────────────────────────────────

macro_rules! jni_fn {
    ($name:ident, $body:expr) => {
        #[unsafe(no_mangle)]
        pub extern "system" fn $name(mut env: JNIEnv, _class: JClass) -> jstring {
            let result = $body;
            logi(&format!("RESULT: {result}"));
            result_to_jstring(&mut env, &result)
        }
    };
}

jni_fn!(
    Java_dev_okhsunrog_vpnhide_NativeChecks_checkIoctlSiocgifflags,
    check_ioctl_siocgifflags()
);
jni_fn!(
    Java_dev_okhsunrog_vpnhide_NativeChecks_checkIoctlSiocgifmtu,
    check_ioctl_siocgifmtu()
);
jni_fn!(
    Java_dev_okhsunrog_vpnhide_NativeChecks_checkIoctlSiocgifconf,
    check_ioctl_siocgifconf()
);
jni_fn!(
    Java_dev_okhsunrog_vpnhide_NativeChecks_checkGetifaddrs,
    check_getifaddrs()
);
jni_fn!(
    Java_dev_okhsunrog_vpnhide_NativeChecks_checkProcNetRoute,
    check_proc_file("/proc/net/route")
);
jni_fn!(
    Java_dev_okhsunrog_vpnhide_NativeChecks_checkProcNetIfInet6,
    check_proc_file("/proc/net/if_inet6")
);
jni_fn!(
    Java_dev_okhsunrog_vpnhide_NativeChecks_checkNetlinkGetlink,
    check_netlink_getlink()
);
jni_fn!(
    Java_dev_okhsunrog_vpnhide_NativeChecks_checkNetlinkGetlinkRecv,
    check_netlink_getlink_recv()
);
jni_fn!(
    Java_dev_okhsunrog_vpnhide_NativeChecks_checkNetlinkGetroute,
    check_netlink_getroute()
);
jni_fn!(
    Java_dev_okhsunrog_vpnhide_NativeChecks_checkProcNetIpv6Route,
    check_proc_file("/proc/net/ipv6_route")
);
jni_fn!(
    Java_dev_okhsunrog_vpnhide_NativeChecks_checkProcNetTcp,
    check_proc_file("/proc/net/tcp")
);
jni_fn!(
    Java_dev_okhsunrog_vpnhide_NativeChecks_checkProcNetTcp6,
    check_proc_file("/proc/net/tcp6")
);
jni_fn!(
    Java_dev_okhsunrog_vpnhide_NativeChecks_checkProcNetUdp,
    check_proc_file("/proc/net/udp")
);
jni_fn!(
    Java_dev_okhsunrog_vpnhide_NativeChecks_checkProcNetUdp6,
    check_proc_file("/proc/net/udp6")
);
jni_fn!(
    Java_dev_okhsunrog_vpnhide_NativeChecks_checkProcNetDev,
    check_proc_file("/proc/net/dev")
);
jni_fn!(
    Java_dev_okhsunrog_vpnhide_NativeChecks_checkProcNetFibTrie,
    check_proc_file("/proc/net/fib_trie")
);
jni_fn!(
    Java_dev_okhsunrog_vpnhide_NativeChecks_checkSysClassNet,
    check_sys_class_net()
);
