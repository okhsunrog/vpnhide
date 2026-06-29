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

use core::cell::{Cell, RefCell};
use core::ffi::{c_int, c_void};
use core::sync::atomic::{AtomicPtr, Ordering};

use libc::{SIOCGIFCONF, SIOCGIFNAME, ifreq};

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
    #[allow(clippy::missing_const_for_thread_local)]
    static IN_GETIFADDRS: Cell<bool> = const { Cell::new(false) };
}

/// Returns true if `fd` is an `AF_NETLINK` socket.
///
/// Used by recv/recvmsg hooks to skip TCP/UDP/Unix sockets entirely —
/// our netlink filter used to inspect `buf[4..6]` on every recv, which
/// for a TCP stream is arbitrary TLS ciphertext that occasionally
/// matches `RTM_NEWLINK` (16) or `RTM_NEWADDR` (20) by chance. When it
/// did, `filter_netlink_dump` then MUTATED the buffer as if parsing
/// netlink messages, corrupting the TLS stream and hanging the SSL
/// socket. On an HTTPS-heavy cold-start path (e.g. Ozon) the per-recv
/// chance accumulates to near-certain breakage.
fn is_netlink_fd(fd: c_int) -> bool {
    let mut domain: c_int = 0;
    let mut optlen = core::mem::size_of::<c_int>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_DOMAIN,
            &mut domain as *mut c_int as *mut c_void,
            &mut optlen,
        )
    };
    rc == 0 && domain == libc::AF_NETLINK
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
/// Handles all network interface ioctls that could reveal VPN presence:
///
/// * `SIOCGIFNAME` — translates index to name. If the result is a VPN
///   name, returns ENODEV.
/// * `SIOCGIFCONF` — enumerates all interfaces. VPN entries are compacted
///   out of the returned array.
/// * All other `SIOCGIF*` (FLAGS, MTU, INDEX, HWADDR, ADDR, etc.) — the
///   app provides an interface name in `ifr_name`. If it's a VPN name,
///   we short-circuit with ENODEV before calling the real ioctl.
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

    // SIOCGIFCONF — enumerate all interfaces. Call through, then compact
    // the returned ifreq array, removing VPN entries.
    if request == SIOCGIFCONF as libc::c_ulong {
        let ret = unsafe { real(fd, request, arg) };
        if ret == 0 && !arg.is_null() {
            unsafe { filter_ifconf(arg as *mut ifconf) };
        }
        return ret;
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

    // All other SIOCGIF* ioctls (FLAGS, MTU, INDEX, HWADDR, ADDR, etc.)
    // take an ifreq with the interface name as input. Pre-screen it.
    if !arg.is_null() && is_siocgif(request) {
        let req = unsafe { &*(arg as *const ifreq) };
        let name_bytes = unsafe {
            core::slice::from_raw_parts(req.ifr_name.as_ptr().cast::<u8>(), req.ifr_name.len())
        };
        if is_vpn_iface_bytes(name_bytes) {
            set_errno(libc::ENODEV);
            return -1;
        }
    }

    unsafe { real(fd, request, arg) }
}

/// Check if the ioctl request is a SIOCGIF* command that takes a struct
/// ifreq with `ifr_name` as input. Covers the whole get-by-name family,
/// 0x8910 (SIOCGIFNAME) through 0x8970 (SIOCGIFMAP).
///
/// The old ceiling of 0x8930 silently excluded SIOCGIFINDEX (0x8933) — the
/// ioctl `if_nametoindex()` issues — so `if_nametoindex("tun0")` returned the
/// real index and leaked VPN presence. SIOCGIFTXQLEN (0x8942) and SIOCGIFMAP
/// (0x8970) sat above the ceiling too. SIOCGIFNAME (0x8910) and SIOCGIFCONF
/// (0x8912) are handled in their own branches before this check.
fn is_siocgif(request: libc::c_ulong) -> bool {
    (0x8910..=0x8970).contains(&(request as u32))
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ProcNetFile {
    Route,
    Ipv6Route,
    IfInet6,
    Tcp,
    Tcp6,
    Udp,
    Udp6,
    Dev,
}

/// The basenames we intercept under /proc/.../net/.
const PROC_NET_FILES: &[(&[u8], ProcNetFile)] = &[
    (b"route", ProcNetFile::Route),
    (b"ipv6_route", ProcNetFile::Ipv6Route),
    (b"if_inet6", ProcNetFile::IfInet6),
    (b"tcp", ProcNetFile::Tcp),
    (b"tcp6", ProcNetFile::Tcp6),
    // udp/udp6 share the tcp/tcp6 line format (local address is filtered the
    // same way); dev lists interface names verbatim. On enforcing SELinux these
    // are blocked for untrusted_app, but we filter them too as defence in depth.
    (b"udp", ProcNetFile::Udp),
    (b"udp6", ProcNetFile::Udp6),
    (b"dev", ProcNetFile::Dev),
];

/// Given the text after `/proc/`, return the basename following the
/// `.../net/` segment for any owner form we recognise:
/// `net/`, `self/net/`, `thread-self/net/`, `<pid>/net/`,
/// `<pid>/task/<tid>/net/`. Returns `None` if the shape doesn't match.
///
/// Covering `thread-self` and the `task/<tid>` forms closes the bypass where
/// a detector opens `/proc/thread-self/net/tcp` (or its own task dir) to dodge
/// a matcher that only knew `self` and the top-level pid dir.
fn strip_owner_net(rest: &[u8]) -> Option<&[u8]> {
    if let Some(r) = rest.strip_prefix(b"net/") {
        return Some(r);
    }
    if let Some(r) = rest.strip_prefix(b"self/net/") {
        return Some(r);
    }
    if let Some(r) = rest.strip_prefix(b"thread-self/net/") {
        return Some(r);
    }
    // <pid>/... — either <pid>/net/ or <pid>/task/<tid>/net/.
    let slash = rest.iter().position(|&b| b == b'/')?;
    if rest[..slash].is_empty() || !rest[..slash].iter().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let after = &rest[slash + 1..];
    if let Some(r) = after.strip_prefix(b"net/") {
        return Some(r);
    }
    let after = after.strip_prefix(b"task/")?;
    let slash2 = after.iter().position(|&b| b == b'/')?;
    if after[..slash2].is_empty() || !after[..slash2].iter().all(|b| b.is_ascii_digit()) {
        return None;
    }
    after.get(slash2 + 1..)?.strip_prefix(b"net/")
}

/// Check an absolute path like `/proc/net/<file>`, `/proc/self/net/<file>`,
/// `/proc/thread-self/net/<file>`, `/proc/<pid>/net/<file>`, or
/// `/proc/<pid>/task/<tid>/net/<file>`.
fn match_abs_proc_net(path: &[u8]) -> Option<ProcNetFile> {
    let basename = strip_owner_net(path.strip_prefix(b"/proc/")?)?;
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

/// Resolves what `dirfd` currently points at by reading
/// `/proc/self/fd/<dirfd>`. There is a TOCTOU window: the caller can
/// race `dup2`/`fchdir` between this readlink and the subsequent open
/// (`open_filtered_proc_net`), so the open could land on a different
/// directory than the one we just classified.
///
/// Treated as accepted exposure: closing it would mean opening through
/// an unhooked syscall and validating inode/path manually, which is a
/// large amount of code for what amounts to caller-controlled self-DoS
/// (the caller has to fight its own fd table to lose the race). Real
/// detectors don't need this — they just don't `openat` through dirfd
/// in the first place.
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

    // A dirfd pointing at any `.../net` directory we recognise. Reuse
    // strip_owner_net by appending a trailing '/': it expects `.../net/<rest>`,
    // so an empty <rest> means the dir itself.
    if let Some(rest) = target.strip_prefix(b"/proc/") {
        let mut probe = [0u8; 160];
        let body = rest.len();
        if body < probe.len() {
            probe[..body].copy_from_slice(rest);
            probe[body] = b'/';
            if let Some(tail) = strip_owner_net(&probe[..body + 1]) {
                return tail.is_empty();
            }
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

thread_local! {
    /// Reusable buffer for the read+filter+memfd path of `open_filtered_proc_net`.
    ///
    /// Initial capacity 64 KiB matches the previous fixed-size stack array
    /// — covers typical contents on tested devices (`/proc/net/tcp6` ≤ 16 KiB
    /// on a Pixel 8 Pro with ~80 sockets) with headroom for busier systems.
    /// The Vec grows past 64 KiB on demand, so a device with several hundred
    /// active sockets no longer truncates the file (the prior fixed buffer
    /// silently dropped any tail past 64 KiB, occasionally hiding caller's
    /// own sockets and breaking apps that walk the list).
    ///
    /// Lifetime is per-thread: the first call allocates 64 KiB, every later
    /// call into this function on the same thread reuses the allocation
    /// (`clear()` keeps capacity intact). Memory cost is bounded by
    /// max-observed-size × number of threads that ever opened a /proc/net
    /// file, which on Android is typically 1-2.
    ///
    /// Lazy init (no `const { … }` block) is required because
    /// `Vec::with_capacity` is not const; the per-`with()` flag check is
    /// negligible relative to the syscalls we're about to issue.
    static PROC_NET_BUF: RefCell<Vec<u8>> = RefCell::new(Vec::with_capacity(65536));
}

/// Read a /proc/net file, apply the appropriate filter, return a memfd.
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

    PROC_NET_BUF.with(|cell| {
        let mut buf = cell.borrow_mut();
        buf.clear();

        // Read until EOF or error, growing the Vec as needed. Reserves
        // 8 KiB chunks when full; Vec's amortised doubling keeps the realloc
        // count O(log size) for unusually large files. Reads land directly
        // into the unused tail (`spare_capacity_mut`) — no intermediate copy.
        loop {
            if buf.len() == buf.capacity() {
                buf.reserve(8 * 1024);
            }
            let len = buf.len();
            let cap = buf.capacity();
            let n =
                unsafe { libc::read(fd, buf.as_mut_ptr().add(len).cast::<c_void>(), cap - len) };
            if n <= 0 {
                break;
            }
            // SAFETY: libc::read returns ≤ `cap - len`, so the new len is
            // within capacity. The bytes were just initialised by the kernel.
            unsafe {
                buf.set_len(len + n as usize);
            }
        }
        unsafe { libc::close(fd) };

        let filtered_len = apply_filter(&mut buf[..], kind);

        let mfd_flags: libc::c_uint = if flags & libc::O_CLOEXEC != 0 { 1 } else { 0 };
        let memfd =
            unsafe { libc::syscall(libc::SYS_memfd_create, c"".as_ptr(), mfd_flags) as c_int };
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
                        buf[written..filtered_len].as_ptr().cast::<c_void>(),
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
    })
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
        ProcNetFile::Udp => {
            let (addrs4, n4, _, _) = collect_vpn_addrs();
            filter_tcp4_buf(data, &addrs4, n4)
        }
        ProcNetFile::Udp6 => {
            let (_, _, addrs6, n6) = collect_vpn_addrs();
            filter_tcp6_buf(data, &addrs6, n6)
        }
        ProcNetFile::Dev => filter_dev_buf(data),
    }
}

/// Walk `getifaddrs()` with the `IN_GETIFADDRS` re-entrancy guard set,
/// invoking `f` for each entry whose `ifa_name` matches a VPN interface.
///
/// The guard prevents our ioctl hook from filtering while libc's
/// getifaddrs calls ioctl(SIOCGIFFLAGS) internally. Returns `false` if
/// the real getifaddrs() symbol couldn't be resolved or the call failed;
/// the closure is then never invoked.
unsafe fn walk_getifaddrs_vpn(mut f: impl FnMut(&core::ffi::CStr, &libc::ifaddrs)) -> bool {
    let Some(real) = real_getifaddrs() else {
        return false;
    };

    let mut ifap: *mut libc::ifaddrs = core::ptr::null_mut();

    IN_GETIFADDRS.with(|flag| flag.set(true));
    let rc = unsafe { real(&mut ifap) };
    IN_GETIFADDRS.with(|flag| flag.set(false));

    if rc != 0 || ifap.is_null() {
        return false;
    }

    let mut cur = ifap;
    while !cur.is_null() {
        let entry = unsafe { &*cur };
        cur = entry.ifa_next;

        if entry.ifa_name.is_null() {
            continue;
        }
        let name = unsafe { core::ffi::CStr::from_ptr(entry.ifa_name) };
        if !crate::filter::is_vpn_iface_cstr(name) {
            continue;
        }

        f(name, entry);
    }

    unsafe { libc::freeifaddrs(ifap) };
    true
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

    unsafe {
        walk_getifaddrs_vpn(|_name, entry| {
            if entry.ifa_addr.is_null() {
                return;
            }
            let family = (*entry.ifa_addr).sa_family as c_int;
            if family == libc::AF_INET && n4 < MAX_VPN_ADDRS {
                let sin = &*(entry.ifa_addr as *const libc::sockaddr_in);
                addrs4[n4] = sin.sin_addr.s_addr;
                n4 += 1;
            } else if family == libc::AF_INET6 && n6 < MAX_VPN_ADDRS {
                let sin6 = &*(entry.ifa_addr as *const libc::sockaddr_in6);
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
        });
    }

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
/// recvmsg distributes `ret` bytes across the iov array in order:
/// iov[0] fills first, then iov[1], and so on. We filter the portion
/// that landed in iov[0] — netlink dumps fit there in any sane caller
/// (bionic always uses iov[0] only). A deliberately split call where
/// the netlink message crosses an iov boundary is a corner case we
/// don't try to stitch back together: filtering iov[0] still hides
/// VPN entries that landed in it; entries fully inside iov[1+] pass
/// through unfiltered. Strictly better than the previous behaviour,
/// which bailed out entirely as soon as `iovlen != 1` and let a caller
/// bypass the filter with a single extra zero-length iov.
pub unsafe extern "C" fn hooked_recvmsg(fd: c_int, msg: *mut libc::msghdr, flags: c_int) -> isize {
    let Some(real) = real_recvmsg() else {
        set_errno(libc::EFAULT);
        return -1;
    };

    let ret = unsafe { real(fd, msg, flags) };

    if ret <= 0 || msg.is_null() {
        return ret;
    }

    let hdr = unsafe { &*msg };
    if hdr.msg_iovlen == 0 || hdr.msg_iov.is_null() {
        return ret;
    }

    let iov = unsafe { &*hdr.msg_iov };
    if iov.iov_base.is_null() {
        return ret;
    }

    let in_first = ret.min(iov.iov_len as isize);
    let filtered_first = unsafe { maybe_filter_netlink_buf(fd, iov.iov_base as *mut u8, in_first) };

    // Propagate any shrink of iov[0] to the total return value; bytes
    // that landed in iov[1+] are unchanged.
    ret - (in_first - filtered_first)
}

/// Post-process a netlink read: if `fd` is a netlink socket and the
/// `IN_GETIFADDRS` guard isn't set, parse the first message type from
/// `buf[..ret as usize]`, collect VPN iface indices, and rewrite the
/// buffer to filter out VPN entries. Returns the new (possibly smaller)
/// `ret` to propagate to the caller. If anything is wrong (not netlink,
/// guard set, ret < 16, no VPN indices, wrong message type) returns
/// `ret` unchanged.
///
/// # Safety
///
/// `buf` must be valid for `ret` bytes of read+write access when `ret >= 16`.
unsafe fn maybe_filter_netlink_buf(fd: c_int, buf: *mut u8, ret: isize) -> isize {
    // Need at least an nlmsghdr (16 bytes) before we can read nlmsg_type.
    if ret < 16 || buf.is_null() {
        return ret;
    }

    // Skip non-netlink sockets entirely. For TCP/UDP/Unix the first bytes
    // of the buffer are arbitrary user data (TLS ciphertext, HTTP body,
    // etc.), and used to randomly match RTM_NEWLINK/RTM_NEWADDR and trip
    // the mutating filter path — corrupting the receive buffer and
    // stalling the SSL stream on top.
    if !is_netlink_fd(fd) {
        return ret;
    }

    // Guard against re-entry from bionic's getifaddrs internals calling
    // recv/recvmsg on its own netlink socket while we're already mid-filter.
    if IN_GETIFADDRS.with(|f| f.get()) {
        return ret;
    }

    let data = unsafe { core::slice::from_raw_parts_mut(buf, ret as usize) };

    // Quick check: first message type must be one we filter. Route dumps
    // (RTM_GETROUTE) come back as RTM_NEWROUTE and must not be skipped —
    // that gap was the issue #86 `if<N>` leak.
    let nlmsg_type = u16::from_ne_bytes([data[4], data[5]]);
    if nlmsg_type != crate::filter::RTM_NEWADDR
        && nlmsg_type != crate::filter::RTM_NEWLINK
        && nlmsg_type != crate::filter::RTM_NEWROUTE
    {
        return ret;
    }

    let (indices, n) = collect_vpn_iface_indices();
    if n == 0 {
        return ret;
    }

    crate::filter::filter_netlink_dump(data, &indices[..n]) as isize
}

// ============================================================================
//  Hook: recv — filter netlink responses received via recv()
// ============================================================================
//
// Covers callers that resolve the `recv` libc symbol directly. bionic's
// `recv()` tail-calls `recvfrom()`, and FORTIFY lowers fixed-buffer
// `recv(fd, buf, sizeof(buf), 0)` calls to `recvfrom` / `__recvfrom_chk`;
// those paths are caught by the dedicated recvfrom hooks below, so this
// hook only has to handle the bare `recv` symbol. (Hooking `recvfrom`
// turned out to be fine in practice — see the recvfrom hook block.)

static REAL_RECV: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());

type RecvFn = unsafe extern "C" fn(c_int, *mut c_void, usize, c_int) -> isize;

#[inline(always)]
fn real_recv() -> Option<RecvFn> {
    let raw = REAL_RECV.load(Ordering::Relaxed);
    if raw.is_null() {
        None
    } else {
        Some(unsafe { core::mem::transmute::<*mut c_void, RecvFn>(raw) })
    }
}

pub fn set_real_recv_ptr(p: *const ()) {
    REAL_RECV.store(p as *mut c_void, Ordering::Relaxed);
}

/// Replacement for `libc::recv`.
///
/// Same filtering logic as `hooked_recvmsg`, but operates on the flat
/// buffer that `recv` writes into directly.
pub unsafe extern "C" fn hooked_recv(
    fd: c_int,
    buf: *mut c_void,
    len: usize,
    flags: c_int,
) -> isize {
    let Some(real) = real_recv() else {
        set_errno(libc::EFAULT);
        return -1;
    };

    let ret = unsafe { real(fd, buf, len, flags) };

    // Clamp to the buffer size: with MSG_TRUNC the kernel returns the full
    // datagram length, which can exceed `len`, and the filter must never read
    // or write past the caller's buffer. Propagate any shrink like recvmsg.
    let in_buf = ret.min(len as isize);
    let filtered = unsafe { maybe_filter_netlink_buf(fd, buf as *mut u8, in_buf) };
    ret - (in_buf - filtered)
}

// ============================================================================
//  Hook: recvfrom / __recvfrom_chk — filter netlink responses
// ============================================================================
//
// `recv()` is NOT enough. With `_FORTIFY_SOURCE` (the NDK default in release
// builds) a call like `recv(fd, buf, sizeof(buf), 0)` over a fixed-size `buf`
// is lowered by the compiler to `__recvfrom_chk` (or the bypass-fortify
// `recvfrom`), and `recv()` itself tail-calls `recvfrom()`. So a fortified
// native detector — e.g. RKNHardering reading the `RTM_GETROUTE` dump —
// never touches the `recv` symbol and slips past the recv hook. We hook both
// `recvfrom` and `__recvfrom_chk` so the netlink filter sees that traffic
// too (this is what actually closes the issue #86 `if<N>` leak on-device).
//
// `is_netlink_fd` in `maybe_filter_netlink_buf` keeps TCP/UDP/Unix recvfrom
// traffic untouched, so hooking these high-traffic symbols is safe.

static REAL_RECVFROM: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());

type RecvfromFn =
    unsafe extern "C" fn(c_int, *mut c_void, usize, c_int, *mut c_void, *mut c_void) -> isize;

#[inline(always)]
fn real_recvfrom() -> Option<RecvfromFn> {
    let raw = REAL_RECVFROM.load(Ordering::Relaxed);
    if raw.is_null() {
        None
    } else {
        Some(unsafe { core::mem::transmute::<*mut c_void, RecvfromFn>(raw) })
    }
}

pub fn set_real_recvfrom_ptr(p: *const ()) {
    REAL_RECVFROM.store(p as *mut c_void, Ordering::Relaxed);
}

/// Replacement for `libc::recvfrom`. Filters the flat buffer like `recv`.
pub unsafe extern "C" fn hooked_recvfrom(
    fd: c_int,
    buf: *mut c_void,
    len: usize,
    flags: c_int,
    src_addr: *mut c_void,
    addrlen: *mut c_void,
) -> isize {
    let Some(real) = real_recvfrom() else {
        set_errno(libc::EFAULT);
        return -1;
    };

    let ret = unsafe { real(fd, buf, len, flags, src_addr, addrlen) };

    // Clamp to the buffer size (MSG_TRUNC can return more than `len`).
    let in_buf = ret.min(len as isize);
    let filtered = unsafe { maybe_filter_netlink_buf(fd, buf as *mut u8, in_buf) };
    ret - (in_buf - filtered)
}

static REAL_RECVFROM_CHK: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());

/// `ssize_t __recvfrom_chk(int, void*, size_t len, size_t buf_size, int flags,
///                         const struct sockaddr*, socklen_t*)` — bionic FORTIFY.
type RecvfromChkFn = unsafe extern "C" fn(
    c_int,
    *mut c_void,
    usize,
    usize,
    c_int,
    *mut c_void,
    *mut c_void,
) -> isize;

#[inline(always)]
fn real_recvfrom_chk() -> Option<RecvfromChkFn> {
    let raw = REAL_RECVFROM_CHK.load(Ordering::Relaxed);
    if raw.is_null() {
        None
    } else {
        Some(unsafe { core::mem::transmute::<*mut c_void, RecvfromChkFn>(raw) })
    }
}

pub fn set_real_recvfrom_chk_ptr(p: *const ()) {
    REAL_RECVFROM_CHK.store(p as *mut c_void, Ordering::Relaxed);
}

/// Replacement for bionic's `__recvfrom_chk`. Same filtering as `recvfrom`.
pub unsafe extern "C" fn hooked_recvfrom_chk(
    fd: c_int,
    buf: *mut c_void,
    len: usize,
    buf_size: usize,
    flags: c_int,
    src_addr: *mut c_void,
    addrlen: *mut c_void,
) -> isize {
    let Some(real) = real_recvfrom_chk() else {
        set_errno(libc::EFAULT);
        return -1;
    };

    let ret = unsafe { real(fd, buf, len, buf_size, flags, src_addr, addrlen) };

    // Clamp to the requested length, itself bounded by the FORTIFY buffer size
    // (MSG_TRUNC can return more than was written).
    let in_buf = ret.min(len.min(buf_size) as isize);
    let filtered = unsafe { maybe_filter_netlink_buf(fd, buf as *mut u8, in_buf) };
    ret - (in_buf - filtered)
}

/// Collect interface indices of VPN interfaces. Uses real_getifaddrs
/// (with IN_GETIFADDRS guard) and `if_nametoindex` (which calls
/// ioctl(SIOCGIFINDEX) — passed through by our ioctl hook).
fn collect_vpn_iface_indices() -> ([u32; crate::filter::MAX_VPN_ADDRS], usize) {
    use crate::filter::MAX_VPN_ADDRS;

    let mut indices = [0u32; MAX_VPN_ADDRS];
    let mut n = 0usize;

    unsafe {
        walk_getifaddrs_vpn(|_name, entry| {
            if n >= MAX_VPN_ADDRS {
                return;
            }
            // if_nametoindex issues ioctl(SIOCGIFINDEX), which our ioctl hook
            // now blocks for VPN names. Run it under the IN_GETIFADDRS guard so
            // it passes through to the real index — otherwise this filter loses
            // the VPN indices and the netlink route dump (#86) stops filtering.
            let idx = IN_GETIFADDRS.with(|f| {
                let prev = f.get();
                f.set(true);
                let i = libc::if_nametoindex(entry.ifa_name);
                f.set(prev);
                i
            });
            if idx == 0 || indices[..n].contains(&idx) {
                return;
            }
            indices[n] = idx;
            n += 1;
        });
    }

    (indices, n)
}

#[cfg(test)]
mod proc_net_path_tests {
    use super::{match_abs_proc_net, strip_owner_net, ProcNetFile};

    #[test]
    fn matches_classic_forms() {
        assert_eq!(match_abs_proc_net(b"/proc/net/tcp"), Some(ProcNetFile::Tcp));
        assert_eq!(match_abs_proc_net(b"/proc/self/net/tcp6"), Some(ProcNetFile::Tcp6));
        assert_eq!(match_abs_proc_net(b"/proc/1234/net/route"), Some(ProcNetFile::Route));
    }

    #[test]
    fn matches_new_dev_udp_files() {
        assert_eq!(match_abs_proc_net(b"/proc/net/dev"), Some(ProcNetFile::Dev));
        assert_eq!(match_abs_proc_net(b"/proc/net/udp"), Some(ProcNetFile::Udp));
        assert_eq!(match_abs_proc_net(b"/proc/net/udp6"), Some(ProcNetFile::Udp6));
    }

    #[test]
    fn matches_thread_self_and_task_forms() {
        // These previously bypassed the matcher (#14/#52).
        assert_eq!(match_abs_proc_net(b"/proc/thread-self/net/tcp"), Some(ProcNetFile::Tcp));
        assert_eq!(match_abs_proc_net(b"/proc/1234/task/5678/net/tcp"), Some(ProcNetFile::Tcp));
        assert_eq!(match_abs_proc_net(b"/proc/self/net/dev"), Some(ProcNetFile::Dev));
    }

    #[test]
    fn rejects_non_proc_net() {
        assert_eq!(match_abs_proc_net(b"/proc/net/unknownfile"), None);
        assert_eq!(match_abs_proc_net(b"/etc/passwd"), None);
        assert_eq!(match_abs_proc_net(b"/proc/abc/net/tcp"), None);
        assert_eq!(strip_owner_net(b"bogus/path"), None);
    }
}
