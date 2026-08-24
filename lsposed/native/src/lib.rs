mod generated;

use std::ffi::CStr;
use std::fs;
use std::io::{self, ErrorKind};
use std::mem;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::ptr;
use std::slice;

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

/// 8-byte-aligned byte buffer. The `SIOCGIFCONF` probe reinterprets the raw
/// bytes the kernel writes back as `ifreq` values. A plain `[u8; N]` is only
/// 1-aligned, so the backing storage must provide the alignment of `ifreq`.
#[repr(align(8))]
struct AlignedBytes<const N: usize>([u8; N]);

impl<const N: usize> AlignedBytes<N> {
    fn zeroed() -> Self {
        Self([0u8; N])
    }
}

fn is_selinux_denial(e: &io::Error) -> bool {
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

fn nul_terminated_bytes_to_string(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn last_os_error() -> String {
    io::Error::last_os_error().to_string()
}

fn last_os_errno() -> i32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(0)
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

const IFLA_IFNAME: u16 = 3;
const RTA_OIF: u16 = 4;

// ── check implementations ────────────────────────────────────────────

/// Open an IPv4 datagram socket and pass it to `f`, then close it.
/// Returns `CheckOutput::network_blocked(...)` if `socket()` returns
/// ECONNREFUSED (no NETWORK permission), `CheckOutput::fail(...)` for
/// any other socket() failure, otherwise the result of `f(fd)`.
fn with_inet_dgram_socket(f: impl FnOnce(libc::c_int) -> CheckOutput) -> CheckOutput {
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
    // SAFETY: `socket` returned a new owned descriptor above.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    f(fd.as_raw_fd())
}

fn check_ioctl_siocgifflags() -> CheckOutput {
    unsafe {
        with_inet_dgram_socket(|fd| {
            let mut ifr: libc::ifreq = mem::zeroed();
            let name = b"tun0\0";
            ifr.ifr_name[..name.len()].copy_from_slice(&name.map(|b| b as libc::c_char));

            let ret = libc::ioctl(fd, libc::SIOCGIFFLAGS as _, &mut ifr);
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
            let mut ifr: libc::ifreq = mem::zeroed();
            let name = b"tun0\0";
            ifr.ifr_name[..name.len()].copy_from_slice(&name.map(|b| b as libc::c_char));

            let ret = libc::ioctl(fd, libc::SIOCGIFMTU as _, &mut ifr);
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
            let mut ifc: libc::ifconf = mem::zeroed();
            ifc.ifc_len = buf.0.len() as libc::c_int;
            ifc.ifc_ifcu.ifcu_buf = buf.0.as_mut_ptr().cast();

            if libc::ioctl(fd, libc::SIOCGIFCONF as _, &mut ifc) < 0 {
                let e = last_os_error();
                return CheckOutput::fail(format!("ioctl error: {e}"));
            }

            let count = ifc.ifc_len as usize / mem::size_of::<libc::ifreq>();
            let reqs = slice::from_raw_parts(buf.0.as_ptr().cast::<libc::ifreq>(), count);

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
        let mut addrs: *mut libc::ifaddrs = ptr::null_mut();
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

/// `setsockopt(SO_BINDTODEVICE, "tun0")` — the pre-mutation bind-interface vector
/// (kmod/KPM `socket_bind_interface`, RKNHardering probe n18). A target that can
/// bind a socket to tun0 has leaked the interface's existence and can pin traffic
/// to it; the kernel backend denies this by returning ENODEV *before* mutating
/// `sk_bound_dev_if`.
///
/// Classification mirrors the backend contract and RKNHardering's own semantics
/// (only a completed bind is a leak):
///
/// - `rc == 0` → the bind succeeded → the interface is bindable/visible (FAIL).
/// - `ENODEV` → the hook denied it (or the iface is absent) → not visible (PASS);
///   the root ground-truth differential decides which.
/// - `EPERM`/`EACCES`/other → the app could not complete the bind for a reason that
///   is NOT our hook (capability/SELinux). Report as a permission block, never as a
///   backend success or a leak.
fn check_setsockopt_bindtodevice() -> CheckOutput {
    unsafe {
        with_inet_dgram_socket(|fd| {
            // Include the NUL terminator in optlen, exactly like the RKNHardering
            // probe (name.size() + 1); the kernel reads optval as a C string.
            let name = b"tun0\0";
            let ret = libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_BINDTODEVICE,
                name.as_ptr().cast(),
                name.len() as libc::socklen_t,
            );
            if ret == 0 {
                // Confirm the bind actually took, exactly like the RKNHardering
                // probe: a skip_origin-style hook (zygisk) returns 0 without binding,
                // which neutralises the vector rather than leaking it. Only a
                // getsockopt that echoes back tun0 is a real leak.
                let mut dev = [0u8; libc::IFNAMSIZ];
                let mut dev_len = dev.len() as libc::socklen_t;
                let got = libc::getsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_BINDTODEVICE,
                    dev.as_mut_ptr().cast(),
                    &mut dev_len,
                );
                let bound = got == 0 && cstr_to_str(dev.as_ptr().cast()) == "tun0";
                return if bound {
                    CheckOutput::fail(
                        "setsockopt(tun0, SO_BINDTODEVICE) succeeded and getsockopt confirmed — interface bindable/visible",
                    )
                } else {
                    CheckOutput::pass(
                        "setsockopt(tun0, SO_BINDTODEVICE) returned 0 but getsockopt shows unbound — bind neutralised",
                    )
                };
            }
            let err = last_os_errno();
            match err {
                libc::ENODEV | libc::ENXIO => CheckOutput::pass(
                    "setsockopt(tun0, SO_BINDTODEVICE) returned ENODEV — interface not visible",
                ),
                _ => CheckOutput::selinux_blocked(format!(
                    "SO_BINDTODEVICE denied (errno {err}: {}) — capability/SELinux block, not a backend hook",
                    last_os_error()
                )),
            }
        })
    }
}

/// First `max_bytes` bytes of `line`, backed off to the nearest char boundary.
///
/// Interface names are arbitrary bytes, so a `/proc/net` line can legitimately
/// carry multi-byte UTF-8. Slicing straight at `max_bytes` panics when that byte
/// lands mid-character. The JNI entry catches the unwind now, but a probe that
/// panics still loses the whole check run, and the truncation itself is not an
/// edge case. (Truncation itself is not an edge
/// case: `/proc/net/route` lines routinely run past 80 bytes.)
fn truncate_on_char_boundary(line: &str, max_bytes: usize) -> &str {
    let mut end = line.len().min(max_bytes);
    while end > 0 && !line.is_char_boundary(end) {
        end -= 1;
    }
    &line[..end]
}

#[cfg(test)]
mod truncate_tests {
    use super::truncate_on_char_boundary;

    #[test]
    fn keeps_short_lines_whole() {
        assert_eq!(truncate_on_char_boundary("tun0 up", 80), "tun0 up");
    }

    #[test]
    fn cuts_ascii_at_the_byte_budget() {
        let line = "a".repeat(100);
        assert_eq!(truncate_on_char_boundary(&line, 80).len(), 80);
    }

    #[test]
    fn backs_off_when_the_budget_splits_a_character() {
        // 'я' is two bytes: byte 80 falls inside the 40th one.
        let line = "я".repeat(60);
        let truncated = truncate_on_char_boundary(&line, 81);
        assert_eq!(truncated.len(), 80);
        assert!(line.starts_with(truncated));
    }

    #[test]
    fn yields_nothing_when_the_first_character_does_not_fit() {
        assert_eq!(truncate_on_char_boundary("я", 1), "");
    }
}

fn check_proc_file(path: &str) -> CheckOutput {
    match fs::read_to_string(path) {
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
                    vpn_lines.push(truncate_on_char_boundary(line, 80).to_string());
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
        let mut msg: libc::msghdr = mem::zeroed();
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
fn open_netlink() -> Result<OwnedFd, CheckOutput> {
    unsafe {
        let raw_fd = libc::socket(libc::AF_NETLINK, libc::SOCK_RAW, libc::NETLINK_ROUTE);
        if raw_fd < 0 {
            let e = io::Error::last_os_error();
            return Err(if is_selinux_denial(&e) {
                CheckOutput::selinux_blocked(format!("netlink socket denied by SELinux ({e})"))
            } else {
                CheckOutput::fail(format!("cannot create netlink socket: {e}"))
            });
        }
        // SAFETY: `socket` returned a new owned descriptor above.
        let fd = OwnedFd::from_raw_fd(raw_fd);

        let mut sa: libc::sockaddr_nl = mem::zeroed();
        sa.nl_family = libc::AF_NETLINK as u16;
        let sa_len = mem::size_of_val(&sa) as libc::socklen_t;
        if libc::bind(fd.as_raw_fd(), ptr::from_ref(&sa).cast(), sa_len) < 0 {
            let e = io::Error::last_os_error();
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
            fd.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            ptr::from_ref(&tv).cast(),
            mem::size_of::<libc::timeval>() as libc::socklen_t,
        );
        Ok(fd)
    }
}

fn read_u16_ne(buf: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_ne_bytes(
        buf.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn read_u32_ne(buf: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_ne_bytes(
        buf.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

/// Parse bounds-checked netlink messages from a byte buffer, calling `on_msg`
/// for each matching message. Returns false on `NLMSG_DONE`/`NLMSG_ERROR`.
fn parse_netlink_msgs(
    buf: &[u8],
    len: usize,
    msg_type: u16,
    mut on_msg: impl FnMut(&[u8], usize, usize),
) -> bool {
    let len = len.min(buf.len());
    let mut offset = 0usize;
    let hdr_size = mem::size_of::<libc::nlmsghdr>();
    while offset + hdr_size <= len {
        let Some(msg_len) = read_u32_ne(buf, offset).map(|len| len as usize) else {
            break;
        };
        if msg_len < hdr_size || msg_len > len - offset {
            break;
        }
        let Some(current_type) = read_u16_ne(buf, offset + 4) else {
            break;
        };
        if current_type == libc::NLMSG_DONE as u16 || current_type == libc::NLMSG_ERROR as u16 {
            return false;
        }
        if current_type == msg_type {
            on_msg(buf, offset, msg_len);
        }
        offset += (msg_len + 3) & !3;
    }
    true // continue receiving
}

/// Iterate bounds-checked `rtattr` entries within a netlink payload.
fn for_each_rtattr(buf: &[u8], start: usize, end: usize, mut on_attr: impl FnMut(u16, &[u8])) {
    // Walk rtattrs in `buf[start..end]`. For each, hand the callback
    // the header AND a slice covering its payload — already bounds-
    // checked against `end`, so callbacks can never read past the
    // message. A truncated tail (rta_len < 4, or rta_len reaching
    // past `end`) ends the walk; netlink dumps end on padding, so
    // this is the normal exit too.
    let end = end.min(buf.len());
    let mut off = start;
    while off + 4 <= end {
        let Some(rta_len) = read_u16_ne(buf, off).map(usize::from) else {
            break;
        };
        let Some(rta_type) = read_u16_ne(buf, off + 2) else {
            break;
        };
        if rta_len < 4 || off + rta_len > end {
            break;
        }
        on_attr(rta_type, &buf[off + 4..off + rta_len]);
        off += (rta_len + 3) & !3;
    }
}

/// Read the one-byte table id from the fixed `rtmsg`/`fib_rule_hdr` header.
fn rtmsg_table(buf: &[u8], msg_offset: usize) -> Option<u32> {
    let table_offset = msg_offset
        .checked_add(mem::size_of::<libc::nlmsghdr>())?
        .checked_add(4)?;
    buf.get(table_offset).copied().map(u32::from)
}

#[cfg(test)]
mod netlink_parser_tests {
    use super::{for_each_rtattr, parse_netlink_msgs, rtmsg_table};

    fn message(message_type: u16, payload: &[u8]) -> Vec<u8> {
        let len = 16 + payload.len();
        let mut message = Vec::with_capacity(len);
        message.extend_from_slice(&(len as u32).to_ne_bytes());
        message.extend_from_slice(&message_type.to_ne_bytes());
        message.extend_from_slice(&0_u16.to_ne_bytes());
        message.extend_from_slice(&1_u32.to_ne_bytes());
        message.extend_from_slice(&0_u32.to_ne_bytes());
        message.extend_from_slice(payload);
        message
    }

    #[test]
    fn parses_messages_from_an_unaligned_slice() {
        let mut storage = vec![0xff];
        storage.extend_from_slice(&message(42, &[1, 2, 3, 4]));
        let data = &storage[1..];
        let mut payload = Vec::new();

        assert!(parse_netlink_msgs(
            data,
            data.len(),
            42,
            |buf, offset, len| {
                payload.extend_from_slice(&buf[offset + 16..offset + len]);
            }
        ));
        assert_eq!(payload, [1, 2, 3, 4]);
    }

    #[test]
    fn rejects_truncated_attributes_without_calling_back() {
        let mut data = Vec::new();
        data.extend_from_slice(&12_u16.to_ne_bytes());
        data.extend_from_slice(&3_u16.to_ne_bytes());
        data.extend_from_slice(&[1, 2, 3, 4]);
        let mut called = false;

        for_each_rtattr(&data, 0, usize::MAX, |_, _| called = true);

        assert!(!called);
    }

    #[test]
    fn reads_rule_table_without_a_typed_pointer_cast() {
        let mut storage = vec![0xff];
        storage.extend_from_slice(&message(32, &[0, 0, 0, 0, 123, 0, 0, 0]));

        assert_eq!(rtmsg_table(&storage[1..], 0), Some(123));
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
        let mut req: Req = mem::zeroed();
        req.nlh.nlmsg_len = mem::size_of::<Req>() as u32;
        req.nlh.nlmsg_type = libc::RTM_GETLINK;
        req.nlh.nlmsg_flags = (libc::NLM_F_REQUEST | libc::NLM_F_DUMP) as u16;
        req.nlh.nlmsg_seq = 1;

        if libc::send(
            fd.as_raw_fd(),
            ptr::from_ref(&req).cast(),
            req.nlh.nlmsg_len as usize,
            0,
        ) < 0
        {
            let e = last_os_error();
            return CheckOutput::fail(format!("send error: {e}"));
        }

        let mut buf = AlignedBytes::<32768>::zeroed();
        let mut all = Vec::new();
        let mut vpn = Vec::new();
        let hdr_plus_ifinfo = mem::size_of::<libc::nlmsghdr>() + mem::size_of::<Ifinfomsg>();

        for _ in 0..MAX_NETLINK_RECV_ITERS {
            let len = netlink_recv(fd.as_raw_fd(), &mut buf.0);
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
                    for_each_rtattr(b, data_start, msg_end, |rta_type, payload| {
                        if rta_type == IFLA_IFNAME && !payload.is_empty() {
                            // IFLA_IFNAME is a NUL-terminated string;
                            // payload was bounds-checked by for_each_rtattr.
                            let name = nul_terminated_bytes_to_string(payload);
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
        let mut req: Req = mem::zeroed();
        req.nlh.nlmsg_len = mem::size_of::<Req>() as u32;
        req.nlh.nlmsg_type = libc::RTM_GETROUTE;
        req.nlh.nlmsg_flags = (libc::NLM_F_REQUEST | libc::NLM_F_DUMP) as u16;
        req.nlh.nlmsg_seq = 1;

        if libc::send(
            fd.as_raw_fd(),
            ptr::from_ref(&req).cast(),
            req.nlh.nlmsg_len as usize,
            0,
        ) < 0
        {
            let e = last_os_error();
            return CheckOutput::fail(format!("send error: {e}"));
        }

        let mut buf = AlignedBytes::<32768>::zeroed();
        let mut vpn = Vec::new();
        let mut total = 0u32;
        let hdr_plus_rtmsg = mem::size_of::<libc::nlmsghdr>() + mem::size_of::<Rtmsg>();

        for _ in 0..MAX_NETLINK_RECV_ITERS {
            let len = netlink_recv(fd.as_raw_fd(), &mut buf.0);
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
                    for_each_rtattr(b, data_start, msg_end, |rta_type, payload| {
                        if rta_type == RTA_OIF
                            && let Some(ifindex) = read_u32_ne(payload, 0)
                        {
                            let mut ifname_buf = [0u8; libc::IF_NAMESIZE];
                            let ptr = libc::if_indextoname(ifindex, ifname_buf.as_mut_ptr().cast());
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
        let mut req: Req = mem::zeroed();
        req.nlh.nlmsg_len = mem::size_of::<Req>() as u32;
        req.nlh.nlmsg_type = libc::RTM_GETRULE;
        req.nlh.nlmsg_flags = (libc::NLM_F_REQUEST | libc::NLM_F_DUMP) as u16;
        req.nlh.nlmsg_seq = seq;
        req.frh.rtm_family = family;

        if libc::send(
            fd,
            ptr::from_ref(&req).cast(),
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
        let hdr_plus_rtmsg = mem::size_of::<libc::nlmsghdr>() + mem::size_of::<Rtmsg>();

        // fib_rule_hdr shares Rtmsg's 12-byte layout (family + 7 u8 + u32 flags).
        let mut on_rule = |b: &[u8], offset: usize, msg_len: usize| {
            total += 1;
            // The low byte of the table id lives in the header; the full u32
            // arrives in FRA_TABLE (Android tun tables are > 255).
            let mut table = rtmsg_table(b, offset).unwrap_or(0);
            let mut uid_lo = 0u32;
            let mut uid_hi = 0u32;
            let mut has_uidrange = false;
            let mut iface_hit: Option<String> = None;
            for_each_rtattr(
                b,
                offset + hdr_plus_rtmsg,
                offset + msg_len,
                |rta_type, payload| match rta_type {
                    FRA_IIFNAME | FRA_OIFNAME if !payload.is_empty() => {
                        let name = nul_terminated_bytes_to_string(payload);
                        if is_vpn_iface(&name) {
                            iface_hit = Some(name);
                        }
                    }
                    FRA_TABLE if payload.len() >= 4 => {
                        table = read_u32_ne(payload, 0).unwrap_or(table);
                    }
                    FRA_UID_RANGE if payload.len() >= 8 => {
                        if let (Some(lo), Some(hi)) =
                            (read_u32_ne(payload, 0), read_u32_ne(payload, 4))
                        {
                            uid_lo = lo;
                            uid_hi = hi;
                            has_uidrange = true;
                        }
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
            if let Err(e) = dump_fib_rules(
                fd.as_raw_fd(),
                family,
                (i + 1) as u32,
                &mut buf.0,
                &mut on_rule,
            ) {
                return CheckOutput::fail(format!("send error: {e}"));
            }
        }
    }

    if leaks.is_empty() {
        CheckOutput::pass(format!("{total} policy rules, none reveal VPN"))
    } else {
        CheckOutput::fail(format!("VPN policy rule(s): {}", join_list(&leaks)))
    }
}

fn check_sys_class_net() -> CheckOutput {
    match fs::read_dir("/sys/class/net") {
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

fn check_proc_sys_net_ifaces() -> CheckOutput {
    const DIRS: [&str; 4] = [
        "/proc/sys/net/ipv4/conf",
        "/proc/sys/net/ipv4/neigh",
        "/proc/sys/net/ipv6/conf",
        "/proc/sys/net/ipv6/neigh",
    ];
    let mut visible = Vec::new();
    for path in DIRS {
        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(e) if is_selinux_denial(&e) => {
                return CheckOutput::selinux_blocked(format!("access denied by SELinux ({e})"));
            }
            Err(e) => return CheckOutput::fail(format!("cannot open {path}: {e}")),
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_vpn_iface(&name) {
                visible.push(format!("{path}/{name}"));
            }
        }
    }
    if visible.is_empty() {
        CheckOutput::pass("no VPN interface directories".to_string())
    } else {
        CheckOutput::fail(format!("VPN paths: {}", join_list(&visible)))
    }
}

// ── /proc/net/* wrappers: one fn per path, aggregated into the single
//    JSON-returning probe surface, so the Kotlin side never pushes path
//    strings across the FFI. ────────────────────────────────────────────

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
        j("so_bindtodevice", check_setsockopt_bindtodevice()),
        j("netlink_getlink", check_netlink_getlink()),
        j("netlink_getroute", check_netlink_getroute()),
        j("netlink_getrule", check_netlink_getrule()),
        j("proc_route", check_proc_net_route()),
        j("proc_ipv6_route", check_proc_net_ipv6_route()),
        j("proc_if_inet6", check_proc_net_if_inet6()),
        j("proc_dev", check_proc_net_dev()),
        j("sys_class_net", check_sys_class_net()),
        j("proc_sys_net", check_proc_sys_net_ifaces()),
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
        let hdr_plus_rtmsg = mem::size_of::<libc::nlmsghdr>() + mem::size_of::<Rtmsg>();

        let mut on_rule = |b: &[u8], offset: usize, msg_len: usize| {
            let mut r = GateRule {
                table: rtmsg_table(b, offset).unwrap_or(0),
                uid_lo: 0,
                uid_hi: 0,
                has_uidrange: false,
                oif_vpn: false,
            };
            for_each_rtattr(
                b,
                offset + hdr_plus_rtmsg,
                offset + msg_len,
                |rta_type, payload| match rta_type {
                    FRA_OIFNAME if !payload.is_empty() => {
                        let name = nul_terminated_bytes_to_string(payload);
                        if is_vpn_iface(&name) {
                            r.oif_vpn = true;
                        }
                    }
                    FRA_TABLE if payload.len() >= 4 => {
                        r.table = read_u32_ne(payload, 0).unwrap_or(r.table);
                    }
                    FRA_UID_RANGE if payload.len() >= 8 => {
                        if let (Some(lo), Some(hi)) =
                            (read_u32_ne(payload, 0), read_u32_ne(payload, 4))
                        {
                            r.uid_lo = lo;
                            r.uid_hi = hi;
                            r.has_uidrange = true;
                        }
                    }
                    _ => {}
                },
            );
            rules.push(r);
        };

        // v4 + v6: a VPN steers the uid into a tun table for both families; either
        // membership means routed, so merge both dumps before deciding.
        for (i, family) in RULE_FAMILIES.into_iter().enumerate() {
            if let Err(e) = dump_fib_rules(
                fd.as_raw_fd(),
                family,
                (i + 1) as u32,
                &mut buf.0,
                &mut on_rule,
            ) {
                return (false, format!("send error: {e}"));
            }
        }
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

/// Log tag for anything this crate reports. Listed in the app's `LogTags` so the
/// debug bundle's logcat filter picks it up.
#[cfg(target_os = "android")]
const LOG_TAG: &str = "VpnHide-Native";

#[cfg(target_os = "android")]
unsafe extern "C" {
    fn __android_log_write(
        prio: libc::c_int,
        tag: *const libc::c_char,
        text: *const libc::c_char,
    ) -> libc::c_int;
}

/// Send one line to logcat. Best effort: a message with an interior NUL is
/// dropped rather than truncated at a surprising place.
#[cfg(target_os = "android")]
fn log_error(message: &str) {
    use std::ffi::CString;

    const ANDROID_LOG_ERROR: libc::c_int = 6;
    let (Ok(tag), Ok(text)) = (CString::new(LOG_TAG), CString::new(message)) else {
        return;
    };
    // SAFETY: both pointers are NUL-terminated and outlive the call.
    unsafe {
        __android_log_write(ANDROID_LOG_ERROR, tag.as_ptr(), text.as_ptr());
    }
}

/// Route panics to logcat.
///
/// The default hook writes to stderr, which for an Android app process goes
/// nowhere — so a panic in here used to be invisible: the process just died (or,
/// since the switch to unwinding, surfaced as a Java exception with no location).
/// The hook runs under either panic strategy and carries what actually matters
/// for a bug report: the message and the `file:line` that raised it. Backtraces
/// are deliberately not attempted; this crate is built with fat LTO and stripped,
/// so they would be unsymbolised addresses.
#[cfg(target_os = "android")]
fn install_panic_hook() {
    use std::sync::Once;

    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            log_error(&format!("panic in native probe: {info}"));
            previous(info);
        }));
    });
}

/// In-process (app-view) entry. Class/package must match the Kotlin
/// `object dev.okhsunrog.vpnhide.checks.NativeProbe`.
///
/// A panic here must not take the app down with it: the probes parse whatever
/// the kernel hands back on an arbitrary vendor build, and this runs on every
/// cold start. `with_env` catches the unwind (which is why the crate builds with
/// `panic = "unwind"`) and `ThrowRuntimeExAndDefault` turns it into a Java
/// exception the caller can report as a failed check run.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_okhsunrog_vpnhide_checks_NativeProbe_runAllChecksJson<'local>(
    mut env: jni::EnvUnowned<'local>,
    _class: jni::objects::JClass<'local>,
) -> jni::objects::JString<'local> {
    install_panic_hook();
    env.with_env(
        |env| -> jni::errors::Result<jni::objects::JString<'local>> {
            env.new_string(run_all_json())
        },
    )
    .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}
