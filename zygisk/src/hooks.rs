#![allow(clippy::missing_const_for_thread_local)]
//! The actual libc hook functions.
//!
//! These are called INSTEAD OF the real libc symbols from any PLT we've
//! patched. Each hook calls the saved original function via a function
//! pointer stored in `statics` after `pltHookRegister` ran. We can't rely
//! on linkage-time names like `libc::ioctl` because those would resolve
//! back through our own PLT entry — which would either recurse infinitely
//! or, in a different library's process, not be hooked at all.
//!
//! ## Important safety notes
//!
//! * The `*const ()` function pointers used by Zygisk's PLT hook API are
//!   variadic in C (`ioctl` is `int ioctl(int fd, unsigned long req, ...)`)
//!   but Rust doesn't have first-class variadic functions. We work around
//!   this by declaring the relevant `ioctl` signatures we care about
//!   (`SIOCGIFNAME`, `SIOCGIFFLAGS`) as 3-argument forms and trusting the
//!   ABI — that's what the rest of the world does.
//! * The hook must be reentrant-safe. It WILL be called from arbitrary
//!   threads, before `main()`, from signal handlers in the worst case.
//!   No `println!`, no locks that could deadlock, no allocation unless
//!   absolutely necessary.
//! * If anything fails inside a hook, we fall back to calling the
//!   original. Panicking would abort the entire process.

use core::cell::Cell;
use core::ffi::{c_int, c_void};
use core::sync::atomic::{AtomicPtr, Ordering};

use libc::{SIOCGIFCONF, SIOCGIFFLAGS, SIOCGIFNAME, ifreq};

use crate::filter::is_vpn_iface_bytes;

/// `struct ifconf` from `<net/if.h>`. Not exported by the `libc` crate.
#[repr(C)]
struct ifconf {
    ifc_len: c_int,
    ifc_req: *mut ifreq,
}

// Thread-local guard: set by hooked_getifaddrs before calling the real
// getifaddrs, cleared after. While set, hooked_ioctl passes through
// without filtering. This prevents our ioctl hook from interfering with
// libc's INTERNAL ioctl(SIOCGIFFLAGS) calls made inside getifaddrs() —
// those calls are redundant (getifaddrs hook filters the result anyway)
// and harmful (returning ENODEV breaks libc's ifaddrs list construction,
// causing errors like `ioctl(SIOCGIFFLAGS) for "tun0" failed in ifaddrs`
// and corrupting NFC/HCE payment flows).
thread_local! {
    static IN_GETIFADDRS: Cell<bool> = const { Cell::new(false) };
}

// Android bionic exposes the thread-local errno via `int *__errno()`.
// The `libc` crate doesn't re-export this symbol for android targets,
// so declare it ourselves. Matches bionic's prototype exactly.
unsafe extern "C" {
    fn __errno() -> *mut c_int;
}

#[inline(always)]
fn set_errno(val: c_int) {
    unsafe {
        *__errno() = val;
    }
}

// ============================================================================
//  Saved originals
// ============================================================================

/// Pointer to the real `ioctl` entrypoint, captured by `pltHookRegister`.
/// Stored as `AtomicPtr<c_void>` so the hook can load it with a relaxed
/// atomic read — no locks.
static REAL_IOCTL: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());

/// Raw function type for the slice of ioctl variants we care about.
/// Matches the three-argument C signature `int ioctl(int, unsigned long, void*)`.
type IoctlFn = unsafe extern "C" fn(c_int, libc::c_ulong, *mut c_void) -> c_int;

/// Safely fetch the saved original ioctl. Returns None if for some reason
/// the pointer is null (should never happen after install, but defensive).
#[inline(always)]
fn real_ioctl() -> Option<IoctlFn> {
    let raw = REAL_IOCTL.load(Ordering::Relaxed);
    if raw.is_null() {
        None
    } else {
        // SAFETY: we only ever store a valid function pointer of this shape
        // in this slot via set_real_ioctl_ptr.
        Some(unsafe { core::mem::transmute::<*mut c_void, IoctlFn>(raw) })
    }
}

/// Stash the original pointer we got back from `pltHookRegister`.
pub fn set_real_ioctl_ptr(p: *const ()) {
    REAL_IOCTL.store(p as *mut c_void, Ordering::Relaxed);
}

// ============================================================================
//  Hook: ioctl
// ============================================================================

/// Replacement for `libc::ioctl`.
///
/// Handles:
/// * `SIOCGIFNAME` — called with `ifr_ifindex` to translate an index to an
///   interface name. If the real result is a VPN name, we rewrite it to
///   look like the index doesn't exist by setting errno=ENODEV and
///   returning -1. The app treats that like "no interface at this index"
///   and moves on.
/// * `SIOCGIFFLAGS` — called with `ifr_name` to get the flags of a specific
///   interface. If the input name is already a VPN, we short-circuit with
///   errno=ENODEV WITHOUT calling the real ioctl (so the kernel never tells
///   us the flags include `IFF_POINTOPOINT` and similar tells).
///
/// Any other request falls straight through to the real ioctl unchanged.
///
/// # Safety
///
/// Called from native code via an inline-hook trampoline. The variadic third
/// argument is promoted to `*mut c_void` — for SIOCGIF* requests it points
/// to a `struct ifreq`; for SIOCGIFCONF it points to a `struct ifconf`.
pub unsafe extern "C" fn hooked_ioctl(
    fd: c_int,
    request: libc::c_ulong,
    arg: *mut c_void,
) -> c_int {
    let Some(real) = real_ioctl() else {
        set_errno(libc::EFAULT);
        return -1;
    };

    // If we're inside a hooked_getifaddrs call on this thread, pass
    // through without filtering. See the IN_GETIFADDRS doc comment.
    if IN_GETIFADDRS.with(|f| f.get()) {
        return unsafe { real(fd, request, arg) };
    }

    // SIOCGIFFLAGS — the app has a name and wants flags. Pre-screen input.
    if request == SIOCGIFFLAGS as libc::c_ulong {
        if !arg.is_null() {
            let req = unsafe { &*(arg as *const ifreq) };
            let name_bytes = unsafe {
                core::slice::from_raw_parts(req.ifr_name.as_ptr().cast::<u8>(), req.ifr_name.len())
            };
            if is_vpn_iface_bytes(name_bytes) {
                set_errno(libc::ENODEV);
                return -1;
            }
        }
        return unsafe { real(fd, request, arg) };
    }

    // SIOCGIFNAME — the app has an index and wants a name. Call through,
    // then filter the result.
    if request == SIOCGIFNAME as libc::c_ulong {
        let ret = unsafe { real(fd, request, arg) };
        if ret == 0 && !arg.is_null() {
            let req = unsafe { &*(arg as *const ifreq) };
            let name_bytes = unsafe {
                core::slice::from_raw_parts(req.ifr_name.as_ptr().cast::<u8>(), req.ifr_name.len())
            };
            if is_vpn_iface_bytes(name_bytes) {
                set_errno(libc::ENODEV);
                return -1;
            }
        }
        return ret;
    }

    // SIOCGIFCONF — enumerate all interfaces. Call through, then compact
    // the returned ifreq array, removing VPN entries.
    if request == SIOCGIFCONF as libc::c_ulong {
        let ret = unsafe { real(fd, request, arg) };
        if ret == 0 && !arg.is_null() {
            unsafe { filter_ifconf(arg as *mut ifconf) };
        }
        return ret;
    }

    // Anything else: pass through unmodified.
    unsafe { real(fd, request, arg) }
}

/// Walk the `ifreq[]` array inside an `ifconf` and remove VPN entries
/// by shifting non-VPN entries forward, then adjusting `ifc_len`.
///
/// # Safety
///
/// `ifc` must point to a valid, caller-owned `struct ifconf` whose
/// `ifc_req` buffer has been filled by a successful `SIOCGIFCONF` ioctl.
unsafe fn filter_ifconf(ifc: *mut ifconf) {
    let ifc = unsafe { &mut *ifc };
    if ifc.ifc_req.is_null() || ifc.ifc_len <= 0 {
        return;
    }

    let entry_size = core::mem::size_of::<ifreq>() as c_int;
    let n = ifc.ifc_len / entry_size;
    let mut dst = 0i32;

    for i in 0..n {
        let entry = unsafe { &*ifc.ifc_req.offset(i as isize) };
        let name_bytes = unsafe {
            core::slice::from_raw_parts(entry.ifr_name.as_ptr().cast::<u8>(), entry.ifr_name.len())
        };
        if is_vpn_iface_bytes(name_bytes) {
            continue;
        }
        if dst != i {
            unsafe {
                core::ptr::copy_nonoverlapping(
                    ifc.ifc_req.offset(i as isize),
                    ifc.ifc_req.offset(dst as isize),
                    1,
                );
            }
        }
        dst += 1;
    }

    ifc.ifc_len = dst * entry_size;
}

// ============================================================================
//  Hook: getifaddrs
// ============================================================================

/// Pointer to the real `getifaddrs`, captured at install time.
static REAL_GETIFADDRS: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());

type GetifaddrsFn = unsafe extern "C" fn(*mut *mut libc::ifaddrs) -> c_int;

#[inline(always)]
fn real_getifaddrs() -> Option<GetifaddrsFn> {
    let raw = REAL_GETIFADDRS.load(Ordering::Relaxed);
    if raw.is_null() {
        None
    } else {
        // SAFETY: we only ever store a valid `getifaddrs` pointer in this
        // slot via `set_real_getifaddrs_ptr`.
        Some(unsafe { core::mem::transmute::<*mut c_void, GetifaddrsFn>(raw) })
    }
}

/// Stash the trampoline returned by shadowhook for `libc.so!getifaddrs`.
pub fn set_real_getifaddrs_ptr(p: *const ()) {
    REAL_GETIFADDRS.store(p as *mut c_void, Ordering::Relaxed);
}

/// Replacement for `libc::getifaddrs`.
///
/// Calls the real `getifaddrs`, then walks the returned linked list and
/// unlinks every entry whose `ifa_name` matches a VPN prefix. The caller
/// still calls `freeifaddrs` on the head pointer we return; it walks only
/// via `ifa_next`, so unlinked (VPN) nodes are leaked — a handful of
/// ~200-byte `struct ifaddrs` per `getifaddrs` call, which is acceptable
/// in exchange for not having to track a per-allocation shadow list. We
/// do not hook `freeifaddrs` for this reason.
///
/// Covers:
/// * native callers of `getifaddrs` directly from C/C++/NDK code;
/// * the Android libcore path: `java.net.NetworkInterface.getNetworkInterfaces()`
///   internally calls `getifaddrs` through a JNI shim (`Libcore.os`), so
///   this hook also catches Kotlin/Java apps if for some reason the
///   Java-level LSPosed hook didn't fire first.
///
/// # Safety
///
/// Called from native code. `ifap` is a valid out-pointer the caller
/// owns; on success the real `getifaddrs` fills it with a pointer to a
/// caller-owned linked list that we are free to mutate before returning.
pub unsafe extern "C" fn hooked_getifaddrs(ifap: *mut *mut libc::ifaddrs) -> c_int {
    let Some(real) = real_getifaddrs() else {
        set_errno(libc::EFAULT);
        return -1;
    };

    // Set the thread-local guard so hooked_ioctl passes through while
    // libc's real getifaddrs runs (it internally calls ioctl for each
    // interface to get flags — we must not filter those).
    IN_GETIFADDRS.with(|f| f.set(true));
    let rc = unsafe { real(ifap) };
    IN_GETIFADDRS.with(|f| f.set(false));

    if rc != 0 || ifap.is_null() {
        return rc;
    }

    // Walk the list using a "previous next-pointer slot" cursor so
    // unlinking the head works the same as unlinking an interior node.
    // `slot` always points at the ifa_next field (or the out-pointer *ifap
    // on the first iteration) whose value is the current entry.
    let mut slot: *mut *mut libc::ifaddrs = ifap;
    unsafe {
        while !(*slot).is_null() {
            let entry = *slot;
            let name_ptr = (*entry).ifa_name;
            let is_vpn = if name_ptr.is_null() {
                false
            } else {
                let name = core::ffi::CStr::from_ptr(name_ptr);
                crate::filter::is_vpn_iface_cstr(name)
            };
            if is_vpn {
                *slot = (*entry).ifa_next;
                // `entry` is intentionally leaked; see the doc comment.
            } else {
                slot = &mut (*entry).ifa_next;
            }
        }
    }

    rc
}

// ============================================================================
//  Hook: openat — intercept /proc/net/* reads
// ============================================================================

/// Pointer to the real `openat` entrypoint, captured at install time.
static REAL_OPENAT: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());

type OpenatFn = unsafe extern "C" fn(c_int, *const libc::c_char, c_int, libc::mode_t) -> c_int;

#[inline(always)]
fn real_openat() -> Option<OpenatFn> {
    let raw = REAL_OPENAT.load(Ordering::Relaxed);
    if raw.is_null() {
        None
    } else {
        Some(unsafe { core::mem::transmute::<*mut c_void, OpenatFn>(raw) })
    }
}

pub fn set_real_openat_ptr(p: *const ()) {
    REAL_OPENAT.store(p as *mut c_void, Ordering::Relaxed);
}

/// Which /proc/net file was matched.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ProcNetFile {
    Route,
    Ipv6Route,
    IfInet6,
    Tcp,
    Tcp6,
}

/// The basenames we intercept under /proc/.../net/.
const PROC_NET_FILES: &[(&[u8], ProcNetFile)] = &[
    (b"route", ProcNetFile::Route),
    (b"ipv6_route", ProcNetFile::Ipv6Route),
    (b"if_inet6", ProcNetFile::IfInet6),
    (b"tcp", ProcNetFile::Tcp),
    (b"tcp6", ProcNetFile::Tcp6),
];

/// Check an absolute path like `/proc/net/<file>`, `/proc/self/net/<file>`,
/// or `/proc/<pid>/net/<file>`.
fn match_abs_proc_net(path: &[u8]) -> Option<ProcNetFile> {
    // Strip the /proc/{net,self/net,<pid>/net}/ prefix, leaving the basename.
    let basename = if let Some(rest) = path.strip_prefix(b"/proc/net/") {
        rest
    } else if let Some(rest) = path.strip_prefix(b"/proc/self/net/") {
        rest
    } else if let Some(rest) = path.strip_prefix(b"/proc/") {
        // /proc/<digits>/net/<file>
        let slash = rest.iter().position(|&b| b == b'/')?;
        let pid = &rest[..slash];
        if pid.is_empty() || !pid.iter().all(|b| b.is_ascii_digit()) {
            return None;
        }
        rest.get(slash + 1..)?.strip_prefix(b"net/")?
    } else {
        return None;
    };

    PROC_NET_FILES
        .iter()
        .find(|(name, _)| *name == basename)
        .map(|(_, kind)| *kind)
}

/// Check a relative path against a dirfd that might point to /proc/.../net.
fn match_rel_proc_net(dirfd: c_int, basename: &[u8]) -> Option<ProcNetFile> {
    // First check if basename matches any known file.
    let kind = PROC_NET_FILES
        .iter()
        .find(|(name, _)| *name == basename)
        .map(|(_, kind)| *kind)?;

    // Then verify dirfd points to /proc/.../net.
    if is_dirfd_proc_net(dirfd) {
        Some(kind)
    } else {
        None
    }
}

fn is_dirfd_proc_net(dirfd: c_int) -> bool {
    let mut link_buf = [0u8; 128];
    let mut fd_path = [0u8; 32];

    let fd_path_len = {
        let prefix = b"/proc/self/fd/";
        fd_path[..prefix.len()].copy_from_slice(prefix);
        let num = fmt_u32(&mut fd_path[prefix.len()..], dirfd as u32);
        prefix.len() + num
    };

    let n = unsafe {
        libc::readlink(
            fd_path[..fd_path_len].as_ptr() as *const libc::c_char,
            link_buf.as_mut_ptr() as *mut libc::c_char,
            link_buf.len(),
        )
    };
    if n <= 0 {
        return false;
    }
    let target = &link_buf[..n as usize];

    if target == b"/proc/net" || target == b"/proc/self/net" {
        return true;
    }
    if let Some(rest) = target.strip_prefix(b"/proc/") {
        if let Some(slash) = rest.iter().position(|&b| b == b'/') {
            let pid = &rest[..slash];
            let tail = &rest[slash..];
            return !pid.is_empty() && pid.iter().all(|b| b.is_ascii_digit()) && tail == b"/net";
        }
    }
    false
}

fn fmt_u32(buf: &mut [u8], mut val: u32) -> usize {
    if val == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut tmp = [0u8; 10];
    let mut len = 0usize;
    while val > 0 {
        tmp[len] = b'0' + (val % 10) as u8;
        val /= 10;
        len += 1;
    }
    for i in 0..len {
        buf[i] = tmp[len - 1 - i];
    }
    len
}

/// Replacement for `libc::openat`.
///
/// Intercepts opens of `/proc/net/{route,ipv6_route,if_inet6,tcp,tcp6}`.
/// For each, reads the real file, filters out VPN-related entries, and
/// returns a `memfd` with the cleaned content.
///
/// Handles absolute paths and relative paths with a /proc/.../net dirfd.
pub unsafe extern "C" fn hooked_openat(
    dirfd: c_int,
    pathname: *const libc::c_char,
    flags: c_int,
    mode: libc::mode_t,
) -> c_int {
    let Some(real) = real_openat() else {
        set_errno(libc::EFAULT);
        return -1;
    };

    if !pathname.is_null() {
        let path = unsafe { core::ffi::CStr::from_ptr(pathname) };
        let path_bytes = path.to_bytes();

        let matched = if path_bytes.first() == Some(&b'/') {
            match_abs_proc_net(path_bytes)
        } else if dirfd >= 0 {
            match_rel_proc_net(dirfd, path_bytes)
        } else {
            None
        };

        if let Some(kind) = matched {
            return unsafe { open_filtered_proc_net(real, dirfd, pathname, flags, mode, kind) };
        }
    }

    unsafe { real(dirfd, pathname, flags, mode) }
}

/// Read a /proc/net file, apply the appropriate filter, return a memfd.
/// Buffer is 64KB to accommodate /proc/net/tcp which can be larger.
unsafe fn open_filtered_proc_net(
    real: OpenatFn,
    dirfd: c_int,
    pathname: *const libc::c_char,
    flags: c_int,
    mode: libc::mode_t,
    kind: ProcNetFile,
) -> c_int {
    let fd = unsafe { real(dirfd, pathname, flags, mode) };
    if fd < 0 {
        return fd;
    }

    // /proc/net/tcp can be larger than route/if_inet6, use 64KB.
    const BUF_SIZE: usize = 65536;
    // SAFETY: we're on a thread stack that can handle 64KB. Android's
    // default thread stack is 1MB+, and this function is only called
    // on specific /proc/net opens.
    let mut buf = [0u8; BUF_SIZE];
    let mut total = 0usize;
    loop {
        let remaining = buf.len() - total;
        if remaining == 0 {
            break;
        }
        let n = unsafe { libc::read(fd, buf[total..].as_mut_ptr() as *mut c_void, remaining) };
        if n <= 0 {
            break;
        }
        total += n as usize;
    }
    unsafe { libc::close(fd) };

    let filtered_len = apply_filter(&mut buf[..total], kind);

    let mfd_flags: libc::c_uint = if flags & libc::O_CLOEXEC != 0 { 1 } else { 0 };
    let memfd = unsafe { libc::syscall(libc::SYS_memfd_create, c"".as_ptr(), mfd_flags) as c_int };
    if memfd < 0 {
        set_errno(libc::EIO);
        return -1;
    }

    if filtered_len > 0 {
        let mut written = 0usize;
        while written < filtered_len {
            let n = unsafe {
                libc::write(
                    memfd,
                    buf[written..].as_ptr() as *const c_void,
                    filtered_len - written,
                )
            };
            if n < 0 {
                unsafe { libc::close(memfd) };
                set_errno(libc::EIO);
                return -1;
            }
            written += n as usize;
        }
        unsafe { libc::lseek(memfd, 0, libc::SEEK_SET) };
    }

    memfd
}

/// Dispatch to the right filter function based on the file type.
fn apply_filter(data: &mut [u8], kind: ProcNetFile) -> usize {
    use crate::filter::*;

    match kind {
        ProcNetFile::Route => filter_route_buf(data),
        ProcNetFile::Ipv6Route => filter_ipv6_route_buf(data),
        ProcNetFile::IfInet6 => filter_if_inet6_buf(data),
        ProcNetFile::Tcp => {
            let (addrs4, n4, _, _) = collect_vpn_addrs();
            filter_tcp4_buf(data, &addrs4, n4)
        }
        ProcNetFile::Tcp6 => {
            let (_, _, addrs6, n6) = collect_vpn_addrs();
            filter_tcp6_buf(data, &addrs6, n6)
        }
    }
}

/// Collect IPv4 and IPv6 addresses of VPN interfaces by calling the
/// real (unhooked) `getifaddrs`. Sets `IN_GETIFADDRS` guard so our
/// ioctl hook doesn't interfere with libc's internal SIOCGIFFLAGS calls.
fn collect_vpn_addrs() -> (
    [u32; crate::filter::MAX_VPN_ADDRS],
    usize,
    [[u32; 4]; crate::filter::MAX_VPN_ADDRS],
    usize,
) {
    use crate::filter::MAX_VPN_ADDRS;

    let mut addrs4 = [0u32; MAX_VPN_ADDRS];
    let mut addrs6 = [[0u32; 4]; MAX_VPN_ADDRS];
    let mut n4 = 0usize;
    let mut n6 = 0usize;

    let Some(real) = real_getifaddrs() else {
        return (addrs4, n4, addrs6, n6);
    };

    let mut ifap: *mut libc::ifaddrs = core::ptr::null_mut();

    // Guard: prevent our ioctl hook from filtering while libc's
    // getifaddrs calls ioctl(SIOCGIFFLAGS) internally.
    IN_GETIFADDRS.with(|f| f.set(true));
    let rc = unsafe { real(&mut ifap) };
    IN_GETIFADDRS.with(|f| f.set(false));

    if rc != 0 || ifap.is_null() {
        return (addrs4, n4, addrs6, n6);
    }

    let mut cur = ifap;
    while !cur.is_null() {
        let entry = unsafe { &*cur };
        cur = entry.ifa_next;

        if entry.ifa_name.is_null() || entry.ifa_addr.is_null() {
            continue;
        }
        let name = unsafe { core::ffi::CStr::from_ptr(entry.ifa_name) };
        if !crate::filter::is_vpn_iface_cstr(name) {
            continue;
        }

        let family = unsafe { (*entry.ifa_addr).sa_family } as c_int;
        if family == libc::AF_INET && n4 < MAX_VPN_ADDRS {
            let sin = unsafe { &*(entry.ifa_addr as *const libc::sockaddr_in) };
            addrs4[n4] = sin.sin_addr.s_addr;
            n4 += 1;
        } else if family == libc::AF_INET6 && n6 < MAX_VPN_ADDRS {
            let sin6 = unsafe { &*(entry.ifa_addr as *const libc::sockaddr_in6) };
            let bytes = sin6.sin6_addr.s6_addr;
            // Convert to 4×u32 matching /proc/net/tcp6's %08X format.
            addrs6[n6] = [
                u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
                u32::from_ne_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
                u32::from_ne_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
                u32::from_ne_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]),
            ];
            n6 += 1;
        }
    }

    unsafe { libc::freeifaddrs(ifap) };

    (addrs4, n4, addrs6, n6)
}

// ============================================================================
//  Hook: recvmsg — filter netlink RTM_NEWADDR / RTM_NEWLINK responses
// ============================================================================

static REAL_RECVMSG: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());

type RecvmsgFn = unsafe extern "C" fn(c_int, *mut libc::msghdr, c_int) -> isize;

#[inline(always)]
fn real_recvmsg() -> Option<RecvmsgFn> {
    let raw = REAL_RECVMSG.load(Ordering::Relaxed);
    if raw.is_null() {
        None
    } else {
        Some(unsafe { core::mem::transmute::<*mut c_void, RecvmsgFn>(raw) })
    }
}

pub fn set_real_recvmsg_ptr(p: *const ()) {
    REAL_RECVMSG.store(p as *mut c_void, Ordering::Relaxed);
}

/// Replacement for `libc::recvmsg`.
///
/// After the real recvmsg returns, checks if the response looks like a
/// netlink dump containing `RTM_NEWADDR` or `RTM_NEWLINK` messages. If
/// so, collects VPN interface indices and removes matching entries from
/// the buffer before returning to the caller.
///
/// Only handles the common single-iov case. Multi-iov netlink responses
/// pass through unfiltered (extremely rare in practice).
pub unsafe extern "C" fn hooked_recvmsg(fd: c_int, msg: *mut libc::msghdr, flags: c_int) -> isize {
    let Some(real) = real_recvmsg() else {
        set_errno(libc::EFAULT);
        return -1;
    };

    let ret = unsafe { real(fd, msg, flags) };

    // Need at least one nlmsghdr (16 bytes) to inspect.
    if ret <= 0 || msg.is_null() {
        return ret;
    }

    let hdr = unsafe { &*msg };
    if hdr.msg_iovlen != 1 || hdr.msg_iov.is_null() {
        return ret;
    }

    let iov = unsafe { &*hdr.msg_iov };
    if iov.iov_base.is_null() || (ret as usize) < 16 {
        return ret;
    }

    let buf = unsafe { core::slice::from_raw_parts_mut(iov.iov_base as *mut u8, ret as usize) };

    // Quick check: first message type must be RTM_NEWADDR or RTM_NEWLINK.
    let nlmsg_type = u16::from_ne_bytes([buf[4], buf[5]]);
    if nlmsg_type != crate::filter::RTM_NEWADDR && nlmsg_type != crate::filter::RTM_NEWLINK {
        return ret;
    }

    let (indices, n) = collect_vpn_iface_indices();
    if n == 0 {
        return ret;
    }

    crate::filter::filter_netlink_dump(buf, &indices[..n]) as isize
}

/// Collect interface indices of VPN interfaces. Uses real_getifaddrs
/// (with IN_GETIFADDRS guard) and `if_nametoindex` (which calls
/// ioctl(SIOCGIFINDEX) — passed through by our ioctl hook).
fn collect_vpn_iface_indices() -> ([u32; crate::filter::MAX_VPN_ADDRS], usize) {
    use crate::filter::MAX_VPN_ADDRS;

    let mut indices = [0u32; MAX_VPN_ADDRS];
    let mut n = 0usize;

    let Some(real) = real_getifaddrs() else {
        return (indices, 0);
    };

    let mut ifap: *mut libc::ifaddrs = core::ptr::null_mut();

    IN_GETIFADDRS.with(|f| f.set(true));
    let rc = unsafe { real(&mut ifap) };
    IN_GETIFADDRS.with(|f| f.set(false));

    if rc != 0 || ifap.is_null() {
        return (indices, 0);
    }

    let mut cur = ifap;
    while !cur.is_null() && n < MAX_VPN_ADDRS {
        let entry = unsafe { &*cur };
        cur = entry.ifa_next;

        if entry.ifa_name.is_null() {
            continue;
        }
        let name = unsafe { core::ffi::CStr::from_ptr(entry.ifa_name) };
        if !crate::filter::is_vpn_iface_cstr(name) {
            continue;
        }

        let idx = unsafe { libc::if_nametoindex(entry.ifa_name) };
        if idx == 0 || indices[..n].contains(&idx) {
            continue;
        }
        indices[n] = idx;
        n += 1;
    }

    unsafe { libc::freeifaddrs(ifap) };
    (indices, n)
}
