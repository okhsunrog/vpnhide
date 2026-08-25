//! The actual libc hook functions.
//!
//! These replace the real libc symbols through shadowhook's inline trampolines.
//! Each hook calls the original function through the trampoline captured when
//! the inline hook was installed. We can't call linkage-time names such as
//! `libc::ioctl`, because they would resolve back through the hooked entry point
//! and recurse.
//!
//! ## Important safety notes
//!
//! * The `*const ()` function pointers returned by shadowhook represent
//!   functions that may be variadic in C (`ioctl` is
//!   `int ioctl(int fd, unsigned long req, ...)`)
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
use core::ffi::{CStr, c_int, c_void};
use core::sync::atomic::{AtomicI32, AtomicPtr, AtomicU8, AtomicU32, Ordering};
use core::{mem, ptr, slice};

use libc::{SIOCGIFCONF, SIOCGIFNAME, ifreq};

use crate::filter::{
    MAX_VPN_ADDRS, RTM_NEWADDR, RTM_NEWLINK, RTM_NEWROUTE, RTM_NEWRULE, filter_dev_buf,
    filter_if_inet6_buf, filter_ipv6_route_buf, filter_netlink_dump, filter_route_buf,
    filter_tcp4_buf, filter_tcp6_buf, is_vpn_iface_bytes, is_vpn_iface_cstr,
};
use crate::generated::hook_ids::Hook;

static ACTIVE_HOOKMASK: AtomicU32 = AtomicU32::new(0);

pub(crate) fn set_active_hookmask(hookmask: u32) {
    ACTIVE_HOOKMASK.store(hookmask, Ordering::Release);
}

pub(crate) fn filesystem_enabled() -> bool {
    ACTIVE_HOOKMASK.load(Ordering::Acquire) & Hook::FilesystemIfacePaths.bit() != 0
}

fn proc_net_open_enabled() -> bool {
    ACTIVE_HOOKMASK.load(Ordering::Acquire) & Hook::ZygiskOpenat.bit() != 0
}

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

    /// Reusable contiguous view for scatter/gather netlink recvmsg payloads.
    /// Most callers use one iovec and stay on the allocation-free fast path;
    /// split payloads borrow this buffer only long enough to gather, filter,
    /// and scatter the compacted bytes back.
    #[allow(clippy::missing_const_for_thread_local)]
    static NETLINK_IOV_BUF: RefCell<Vec<u8>> = RefCell::new(Vec::with_capacity(65536));
}

/// Run one libc operation without letting the ioctl hook filter nested calls.
/// Preserve the previous value so nested guarded operations remain correct.
fn with_ioctl_passthrough<T>(operation: impl FnOnce() -> T) -> T {
    IN_GETIFADDRS.with(|guard| {
        let previous = guard.replace(true);
        let result = operation();
        guard.set(previous);
        result
    })
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
    let mut optlen = mem::size_of::<c_int>() as libc::socklen_t;
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
// so declare it ourselves. Matches bionic's prototype exactly. Host unit
// tests use glibc's equivalent entry point.
#[cfg(target_os = "android")]
unsafe extern "C" {
    fn __errno() -> *mut c_int;
}

#[cfg(all(not(target_os = "android"), target_os = "linux"))]
unsafe extern "C" {
    fn __errno_location() -> *mut c_int;
}

#[inline(always)]
pub(crate) fn get_errno() -> c_int {
    #[cfg(target_os = "android")]
    unsafe {
        *__errno()
    }

    #[cfg(all(not(target_os = "android"), target_os = "linux"))]
    unsafe {
        *__errno_location()
    }
}

#[inline(always)]
pub(crate) fn set_errno(val: c_int) {
    #[cfg(target_os = "android")]
    unsafe {
        *__errno() = val;
    }

    #[cfg(all(not(target_os = "android"), target_os = "linux"))]
    unsafe {
        *__errno_location() = val;
    }
}

// ============================================================================
//  Saved originals
// ============================================================================

/// Declare a saved-original slot for one inline-hooked libc function.
///
/// Each hook needs the same four items to call through to the function it
/// replaced, so we generate them from one declaration instead of hand-copying
/// the block eight times:
///   - `static $slot: AtomicPtr<c_void>` — the captured original pointer,
///   - `type $ty` — its function-pointer shape (doc comments carry through),
///   - `fn $getter() -> Option<$ty>` — None before install or if somehow null,
///   - `pub fn $setter(p: *const ())` — stores the original trampoline.
///
/// The slot is read/written with relaxed atomics: install happens-before any
/// hook fires, so no stronger ordering is needed and no locks are taken.
macro_rules! saved_original {
    (
        $(#[$tymeta:meta])*
        type $ty:ident = $sig:ty;
        static $slot:ident;
        fn $getter:ident;
        pub fn $setter:ident;
    ) => {
        static $slot: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

        $(#[$tymeta])*
        type $ty = $sig;

        #[inline(always)]
        fn $getter() -> Option<$ty> {
            let raw = $slot.load(Ordering::Relaxed);
            if raw.is_null() {
                None
            } else {
                // SAFETY: the slot only ever holds a valid pointer of this
                // exact shape, stored once at install via the setter below.
                Some(unsafe { mem::transmute::<*mut c_void, $ty>(raw) })
            }
        }

        /// Stash the original trampoline returned by shadowhook.
        pub fn $setter(p: *const ()) {
            $slot.store(p as *mut c_void, Ordering::Relaxed);
        }
    };
}

saved_original! {
    /// Raw function type for the slice of ioctl variants we care about.
    /// Matches the three-argument C signature `int ioctl(int, unsigned long, void*)`.
    type IoctlFn = unsafe extern "C" fn(c_int, libc::c_ulong, *mut c_void) -> c_int;
    static REAL_IOCTL;
    fn real_ioctl;
    pub fn set_real_ioctl_ptr;
}

saved_original! {
    /// Raw function type for `setsockopt`.
    type SetsockoptFn = unsafe extern "C" fn(
        c_int,
        c_int,
        c_int,
        *const c_void,
        libc::socklen_t,
    ) -> c_int;
    static REAL_SETSOCKOPT;
    fn real_setsockopt;
    pub fn set_real_setsockopt_ptr;
}

// Android's UAPI has exposed SO_BINDTOIFINDEX since Linux 5.7, but the libc
// crate intentionally follows the portable libc surface and does not define
// it. Keep the kernel ABI value local to this hook.
const SO_BINDTOIFINDEX: c_int = 62;

const KERNEL_BIND_POLICY_UNKNOWN: u8 = 0;
const KERNEL_BIND_POLICY_NATIVE_ONLY: u8 = 1;
const KERNEL_BIND_POLICY_HOOK: u8 = 2;
static KERNEL_BIND_POLICY: AtomicU8 = AtomicU8::new(KERNEL_BIND_POLICY_UNKNOWN);
/// The measured errno to deny a hidden bind with; meaningful once
/// [`KERNEL_BIND_POLICY`] says HOOK.
static KERNEL_BIND_ERRNO: AtomicI32 = AtomicI32::new(libc::ENODEV);

fn parse_decimal_component(input: &[u8], cursor: &mut usize) -> Option<u32> {
    let start = *cursor;
    let mut value = 0u32;
    while let Some(&byte) = input.get(*cursor) {
        if !byte.is_ascii_digit() {
            break;
        }
        value = value.checked_mul(10)?.checked_add(u32::from(byte - b'0'))?;
        *cursor += 1;
    }
    (*cursor > start).then_some(value)
}

fn release_has_unprivileged_first_bind(release: &[u8]) -> bool {
    let mut cursor = 0usize;
    let Some(major) = parse_decimal_component(release, &mut cursor) else {
        return false;
    };
    if release.get(cursor) != Some(&b'.') {
        return false;
    }
    cursor += 1;
    let Some(minor) = parse_decimal_component(release, &mut cursor) else {
        return false;
    };
    major > 5 || (major == 5 && minor >= 7)
}

/// What a bind to a hidden interface must return, given what this kernel says
/// about a name that does not exist.
///
/// Hiding an interface means making it answer exactly like an absent one. Which
/// error that is depends on the order of the kernel's own checks, and that order
/// differs between trees: upstream 5.4 tests `CAP_NET_RAW` before it even looks
/// at the name, so *every* bind — existing or not — fails with EPERM there;
/// 5.7+ resolves the name first, so an absent one fails with ENODEV. Mirroring
/// the measured answer is what keeps the reply from being an oracle: if a
/// non-existent name gives EPERM, answering ENODEV for `tun0` alone would
/// announce that `tun0` exists.
///
/// `probe` is the errno an unprivileged bind to a made-up name produced here, or
/// `None` if we could not measure it. `release_allows_first_bind` is the old
/// version heuristic, kept as the fallback for that case so behaviour is never
/// worse than before the probe existed.
fn hidden_bind_errno(probe: Option<c_int>, release_allows_first_bind: bool) -> Option<c_int> {
    match probe {
        // A made-up name binding successfully means the probe told us nothing
        // usable (or the name existed after all) — fall back.
        Some(0) | None => release_allows_first_bind.then_some(libc::ENODEV),
        Some(errno) => Some(errno),
    }
}

/// Ask the kernel what an unprivileged bind to a name that cannot exist returns.
///
/// One socket and one setsockopt, on our own fd, through the real libc entry —
/// the hook is not consulted, so there is no recursion. Cached for the life of
/// the process: the answer is a property of the kernel, not of the call.
fn probe_absent_iface_errno() -> Option<c_int> {
    // Deliberately not VPN-shaped, so a future matcher change cannot make the
    // probe name itself interesting, and unlikely to the point of impossibility
    // as a real interface.
    const ABSENT: &[u8] = b"zzvpnhideprobe\0";

    let real = real_setsockopt()?;
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
    if fd < 0 {
        return None;
    }
    let rc = unsafe {
        real(
            fd,
            libc::SOL_SOCKET,
            libc::SO_BINDTODEVICE,
            ABSENT.as_ptr().cast(),
            ABSENT.len() as libc::socklen_t,
        )
    };
    let errno = if rc == 0 { 0 } else { get_errno() };
    unsafe { libc::close(fd) };
    Some(errno)
}

/// The errno this process denies hidden binds with, or `None` to stay out of the
/// way entirely. Measured once, then cached in [`KERNEL_BIND_POLICY`].
fn socket_bind_denial_errno() -> Option<c_int> {
    match KERNEL_BIND_POLICY.load(Ordering::Relaxed) {
        KERNEL_BIND_POLICY_NATIVE_ONLY => return None,
        KERNEL_BIND_POLICY_HOOK => return Some(KERNEL_BIND_ERRNO.load(Ordering::Relaxed)),
        _ => {}
    }

    let release_allows_first_bind = unsafe {
        let mut uts = mem::MaybeUninit::<libc::utsname>::zeroed();
        if libc::uname(uts.as_mut_ptr()) != 0 {
            false
        } else {
            let uts = uts.assume_init();
            let release =
                slice::from_raw_parts(uts.release.as_ptr().cast::<u8>(), uts.release.len());
            let end = release
                .iter()
                .position(|&byte| byte == 0)
                .unwrap_or(release.len());
            release_has_unprivileged_first_bind(&release[..end])
        }
    };
    let decision = hidden_bind_errno(probe_absent_iface_errno(), release_allows_first_bind);
    if let Some(errno) = decision {
        KERNEL_BIND_ERRNO.store(errno, Ordering::Relaxed);
    }
    KERNEL_BIND_POLICY.store(
        if decision.is_some() {
            KERNEL_BIND_POLICY_HOOK
        } else {
            KERNEL_BIND_POLICY_NATIVE_ONLY
        },
        Ordering::Relaxed,
    );
    decision
}

/// Copy caller memory without dereferencing its pointer in-process.
///
/// A libc hook receives the same untrusted pointer the kernel syscall would.
/// Dereferencing it in Rust would turn a normal `setsockopt(..., EFAULT)` into
/// a process crash. `process_vm_readv` against our own pid gives us the same
/// fault-contained user-memory read primitive. Use the raw syscall so the
/// module does not acquire a dynamic dependency on bionic's API-23 symbol.
fn copy_from_self(src: *const c_void, dst: &mut [u8]) -> bool {
    if dst.is_empty() {
        return true;
    }
    if src.is_null() {
        return false;
    }

    let local = libc::iovec {
        iov_base: dst.as_mut_ptr().cast(),
        iov_len: dst.len(),
    };
    let remote = libc::iovec {
        // process_vm_readv's ABI uses mutable iovec fields even though the
        // remote side is read-only.
        iov_base: src.cast_mut(),
        iov_len: dst.len(),
    };
    let copied = unsafe {
        libc::syscall(
            libc::SYS_process_vm_readv,
            libc::getpid(),
            ptr::from_ref(&local),
            1usize,
            ptr::from_ref(&remote),
            1usize,
            0usize,
        )
    };
    copied == dst.len() as libc::c_long
}

/// Resolve an interface index without letting our own ioctl hook hide it.
///
/// `if_indextoname` normally issues `SIOCGIFNAME`. The thread-local guard is
/// shared with getifaddrs' internal lookups and makes this one lookup reach the
/// real ioctl before we classify the returned name ourselves.
fn ifindex_is_vpn(ifindex: c_int) -> Option<bool> {
    if ifindex <= 0 {
        return Some(false);
    }

    let mut name = [0u8; libc::IFNAMSIZ];
    let result = with_ioctl_passthrough(|| unsafe {
        libc::if_indextoname(ifindex as u32, name.as_mut_ptr().cast())
    });
    (!result.is_null()).then(|| is_vpn_iface_bytes(&name))
}

#[inline(always)]
fn deny_ifindex_bind(ifindex: c_int, resolved_is_vpn: Option<bool>) -> bool {
    // Zero unbinds; negative values are left to the kernel's native EINVAL.
    // A positive index that disappears during resolution is denied: letting an
    // unresolved index through would re-open the oracle during interface churn.
    ifindex > 0 && resolved_is_vpn.unwrap_or(true)
}

/// Check the syscall's first validation step without changing socket state.
/// Returning false also covers non-socket fds, so the real call can preserve
/// its native `EBADF`/`ENOTSOCK` ordering before we inspect the interface name.
fn is_socket_fd(fd: c_int) -> bool {
    let mut socket_type: c_int = 0;
    let mut len = mem::size_of::<c_int>() as libc::socklen_t;
    unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_TYPE,
            (&mut socket_type as *mut c_int).cast(),
            &mut len,
        ) == 0
    }
}

// ============================================================================
//  Hook: setsockopt
// ============================================================================

/// Best-effort Zygisk replacement for bionic's `setsockopt` entry point.
///
/// Denies successful socket binds to VPN interfaces before the real syscall,
/// matching the kernel backends' externally visible `ENODEV` result without
/// ever leaving `sk_bound_dev_if` mutated. Raw syscalls still bypass this hook;
/// this exists only as a fallback for devices where kmod/KPM cannot run.
///
/// Invalid pointers and unsupported lengths are passed to the real function so
/// the kernel remains the authority for `EFAULT`/`EINVAL` instead of this hook
/// crashing or inventing a different result.
///
/// # Safety
///
/// Called from native code via an inline-hook trampoline. `optval` is untrusted
/// caller memory and must only be inspected through [`copy_from_self`].
pub unsafe extern "C" fn hooked_setsockopt(
    fd: c_int,
    level: c_int,
    optname: c_int,
    optval: *const c_void,
    optlen: libc::socklen_t,
) -> c_int {
    let Some(real) = real_setsockopt() else {
        set_errno(libc::EFAULT);
        return -1;
    };

    if level != libc::SOL_SOCKET {
        return unsafe { real(fd, level, optname, optval, optlen) };
    }

    let denial_errno = if optname == libc::SO_BINDTODEVICE || optname == SO_BINDTOIFINDEX {
        match socket_bind_denial_errno() {
            Some(errno) if is_socket_fd(fd) => errno,
            // Nothing to mirror (or not a socket): stay out of the way.
            _ => return unsafe { real(fd, level, optname, optval, optlen) },
        }
    } else {
        libc::ENODEV
    };

    if optname == libc::SO_BINDTODEVICE {
        // The kernel takes a signed length internally. Preserve its native
        // handling for values that wrapped through the unsigned libc ABI.
        if optlen == 0 || optlen > c_int::MAX as libc::socklen_t {
            return unsafe { real(fd, level, optname, optval, optlen) };
        }

        // sock_bindtoindex() copies at most IFNAMSIZ - 1 bytes, then appends a
        // NUL. Mirror that exact classification window.
        let copied_len = (optlen as usize).min(libc::IFNAMSIZ - 1);
        let mut name = [0u8; libc::IFNAMSIZ];
        if !copy_from_self(optval, &mut name[..copied_len]) {
            return unsafe { real(fd, level, optname, optval, optlen) };
        }
        if is_vpn_iface_bytes(&name) {
            // The measured "no such interface" answer, so a hidden name is
            // indistinguishable from one that never existed on this kernel.
            set_errno(denial_errno);
            return -1;
        }

        // Pass the validated snapshot, not the racy caller buffer. The kernel
        // caps this option to the same copied_len, so shortening a larger
        // optlen does not change the selected interface.
        return unsafe {
            real(
                fd,
                level,
                optname,
                name.as_ptr().cast(),
                copied_len as libc::socklen_t,
            )
        };
    }

    if optname == SO_BINDTOIFINDEX {
        if optlen < mem::size_of::<c_int>() as libc::socklen_t
            || optlen > c_int::MAX as libc::socklen_t
        {
            return unsafe { real(fd, level, optname, optval, optlen) };
        }

        let mut raw = [0u8; mem::size_of::<c_int>()];
        if !copy_from_self(optval, &mut raw) {
            return unsafe { real(fd, level, optname, optval, optlen) };
        }
        let ifindex = c_int::from_ne_bytes(raw);
        if deny_ifindex_bind(ifindex, ifindex_is_vpn(ifindex)) {
            set_errno(denial_errno);
            return -1;
        }

        // The kernel reads one int and ignores any trailing bytes for this
        // option. Passing our snapshot closes a caller-memory race.
        return unsafe {
            real(
                fd,
                level,
                optname,
                (&ifindex as *const c_int).cast(),
                mem::size_of::<c_int>() as libc::socklen_t,
            )
        };
    }

    unsafe { real(fd, level, optname, optval, optlen) }
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
                slice::from_raw_parts(req.ifr_name.as_ptr().cast::<u8>(), req.ifr_name.len())
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
    //
    // The name is read through a fault-contained self-read, never by
    // dereferencing `arg`: this branch inspects the caller's buffer BEFORE the
    // kernel has validated it, so a target app passing a bad or short pointer
    // (the case the real ioctl answers with EFAULT) would otherwise take a
    // SIGSEGV inside its own process. Same reasoning as the setsockopt hook.
    // A non-socket fd cannot carry this family at all, so it is passed straight
    // through rather than paying for a read.
    if !arg.is_null() && is_siocgif(request) && is_socket_fd(fd) {
        let mut name_bytes = [0u8; libc::IFNAMSIZ];
        if copy_from_self(arg, &mut name_bytes) && is_vpn_iface_bytes(&name_bytes) {
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
    // c_ulong is u64 on 64-bit (the `as u32` truncates the ioctl request to
    // its low 32 bits) but already u32 on armeabi-v7a, where clippy sees the
    // cast as unnecessary — the truncation is intended, so allow it.
    #[allow(clippy::unnecessary_cast)]
    let request = request as u32;
    (0x8910..=0x8970).contains(&request)
}

/// Walk the `ifreq[]` array inside an `ifconf`, shift non-VPN entries forward,
/// clear the removed part of the kernel-written range, and adjust `ifc_len`.
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

    let entry_size = mem::size_of::<ifreq>() as c_int;
    let n = ifc.ifc_len / entry_size;
    let mut dst = 0i32;

    for i in 0..n {
        let entry = unsafe { &*ifc.ifc_req.offset(i as isize) };
        let name_bytes = unsafe {
            slice::from_raw_parts(entry.ifr_name.as_ptr().cast::<u8>(), entry.ifr_name.len())
        };
        if is_vpn_iface_bytes(name_bytes) {
            continue;
        }
        if dst != i {
            unsafe {
                ptr::copy_nonoverlapping(
                    ifc.ifc_req.offset(i as isize),
                    ifc.ifc_req.offset(dst as isize),
                    1,
                );
            }
        }
        dst += 1;
    }

    if dst != n {
        // A caller can inspect its buffer past the shortened ifc_len. Erase
        // only slots returned by the kernel and removed by compaction.
        unsafe {
            ptr::write_bytes(ifc.ifc_req.offset(dst as isize), 0, (n - dst) as usize);
        }
    }

    ifc.ifc_len = dst * entry_size;
}

#[cfg(test)]
mod ifconf_tests {
    use super::{filter_ifconf, ifconf};

    fn named_ifreq(name: &[u8]) -> libc::ifreq {
        assert!(name.len() < libc::IFNAMSIZ);
        let mut entry = unsafe { core::mem::zeroed::<libc::ifreq>() };
        unsafe {
            core::ptr::copy_nonoverlapping(
                name.as_ptr(),
                entry.ifr_name.as_mut_ptr().cast::<u8>(),
                name.len(),
            );
        }
        entry
    }

    fn name(entry: &libc::ifreq) -> &[u8] {
        let bytes = unsafe {
            core::slice::from_raw_parts(entry.ifr_name.as_ptr().cast::<u8>(), entry.ifr_name.len())
        };
        &bytes[..bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len())]
    }

    #[test]
    fn compaction_clears_only_the_removed_kernel_output() {
        let mut entries = [
            named_ifreq(b"eth0"),
            named_ifreq(b"tun0"),
            named_ifreq(b"wlan0"),
            named_ifreq(b"caller-owned"),
        ];
        let entry_size = core::mem::size_of::<libc::ifreq>();
        let mut conf = ifconf {
            ifc_len: (3 * entry_size) as libc::c_int,
            ifc_req: entries.as_mut_ptr(),
        };

        unsafe { filter_ifconf(&mut conf) };

        assert_eq!(conf.ifc_len, (2 * entry_size) as libc::c_int);
        assert_eq!(name(&entries[0]), b"eth0");
        assert_eq!(name(&entries[1]), b"wlan0");
        assert!(
            unsafe {
                core::slice::from_raw_parts(
                    (&entries[2] as *const libc::ifreq).cast::<u8>(),
                    entry_size,
                )
            }
            .iter()
            .all(|b| *b == 0)
        );
        assert_eq!(name(&entries[3]), b"caller-owned");
    }
}

// ============================================================================
//  Hook: getifaddrs
// ============================================================================

saved_original! {
    /// Raw function type for `getifaddrs`.
    type GetifaddrsFn = unsafe extern "C" fn(*mut *mut libc::ifaddrs) -> c_int;
    static REAL_GETIFADDRS;
    fn real_getifaddrs;
    pub fn set_real_getifaddrs_ptr;
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
    let rc = with_ioctl_passthrough(|| unsafe { real(ifap) });

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
                let name = CStr::from_ptr(name_ptr);
                is_vpn_iface_cstr(name)
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

saved_original! {
    /// Raw function type for `openat`.
    type OpenatFn = unsafe extern "C" fn(c_int, *const libc::c_char, c_int, libc::mode_t) -> c_int;
    static REAL_OPENAT;
    fn real_openat;
    pub fn set_real_openat_ptr;
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

    if crate::filesystem::hidden_path(dirfd, pathname, false) {
        set_errno(libc::ENOENT);
        return -1;
    }

    if proc_net_open_enabled() && !pathname.is_null() {
        let Some(path) = crate::filesystem::read_caller_path(pathname) else {
            return unsafe { real(dirfd, pathname, flags, mode) };
        };
        let path_bytes = path.as_bytes();

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
unsafe fn walk_getifaddrs_vpn(mut f: impl FnMut(&CStr, &libc::ifaddrs)) -> bool {
    let Some(real) = real_getifaddrs() else {
        return false;
    };

    let mut ifap: *mut libc::ifaddrs = ptr::null_mut();
    let rc = with_ioctl_passthrough(|| unsafe { real(&mut ifap) });

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
        let name = unsafe { CStr::from_ptr(entry.ifa_name) };
        if !is_vpn_iface_cstr(name) {
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
    [u32; MAX_VPN_ADDRS],
    usize,
    [[u32; 4]; MAX_VPN_ADDRS],
    usize,
) {
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

saved_original! {
    /// Raw function type for `recvmsg`.
    type RecvmsgFn = unsafe extern "C" fn(c_int, *mut libc::msghdr, c_int) -> isize;
    static REAL_RECVMSG;
    fn real_recvmsg;
    pub fn set_real_recvmsg_ptr;
}

/// Replacement for `libc::recvmsg`.
///
/// After the real recvmsg returns, checks if the response looks like a
/// netlink dump containing `RTM_NEWADDR` or `RTM_NEWLINK` messages. If
/// so, collects VPN interface indices and removes matching entries from
/// the buffer before returning to the caller.
///
/// recvmsg distributes `ret` bytes across the iov array in order. The common
/// one-iovec case is filtered in place. For scatter/gather callers we assemble
/// the complete returned prefix in a reusable thread-local buffer, filter it as
/// one netlink datagram (including messages crossing an iovec boundary), then
/// scatter the compacted bytes back across the caller's iovecs.
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

    if hdr.msg_iovlen == 1 {
        let iov = unsafe { &*hdr.msg_iov };
        if iov.iov_base.is_null() {
            return ret;
        }
        let in_first = ret.min(iov.iov_len as isize);
        let filtered = unsafe { maybe_filter_netlink_buf(fd, iov.iov_base as *mut u8, in_first) };
        return ret - (in_first - filtered);
    }

    if !is_netlink_fd(fd) || IN_GETIFADDRS.with(|guard| guard.get()) {
        return ret;
    }
    let Some(in_iovecs) = (unsafe { iovec_payload_len(hdr.msg_iov, hdr.msg_iovlen, ret as usize) })
    else {
        return ret;
    };
    if in_iovecs < 16 {
        return ret;
    }
    let (indices, n) = collect_vpn_iface_indices();
    if n == 0 {
        return ret;
    }
    let vpn_uid = unsafe { libc::getuid() };

    NETLINK_IOV_BUF.with(|cell| {
        let Ok(mut scratch) = cell.try_borrow_mut() else {
            return ret;
        };
        let Some(filtered) = (unsafe {
            rewrite_iovec_payload(
                hdr.msg_iov,
                hdr.msg_iovlen,
                in_iovecs,
                &mut scratch,
                |data| filter_netlink_dump(data, &indices[..n], vpn_uid),
            )
        }) else {
            return ret;
        };
        ret - (in_iovecs - filtered) as isize
    })
}

/// Number of bytes actually copied into an iovec array by a successful
/// recvmsg. With MSG_TRUNC, `returned` may exceed the total capacity, so clamp
/// to the writable prefix before gathering.
///
/// # Safety
///
/// `iov` must point to `iovlen` readable descriptors from a successful recvmsg.
unsafe fn iovec_payload_len(
    iov: *const libc::iovec,
    iovlen: usize,
    returned: usize,
) -> Option<usize> {
    let mut capacity = 0usize;
    for index in 0..iovlen {
        let entry = unsafe { &*iov.add(index) };
        if entry.iov_len > 0 && entry.iov_base.is_null() {
            return None;
        }
        capacity = capacity.saturating_add(entry.iov_len);
    }
    Some(returned.min(capacity))
}

/// Gather a returned iovec prefix, compact it with `filter`, and scatter the
/// resulting prefix back. Returns `None` on an invalid descriptor or allocation
/// failure so the hook can safely fall back to the original unmodified result.
///
/// # Safety
///
/// The first `payload_len` bytes described by `iov[0..iovlen]` must be valid for
/// reads and writes, as guaranteed after a successful recvmsg.
unsafe fn rewrite_iovec_payload(
    iov: *mut libc::iovec,
    iovlen: usize,
    payload_len: usize,
    scratch: &mut Vec<u8>,
    filter: impl FnOnce(&mut [u8]) -> usize,
) -> Option<usize> {
    scratch.clear();
    if scratch.try_reserve(payload_len).is_err() {
        return None;
    }

    let mut remaining = payload_len;
    for index in 0..iovlen {
        if remaining == 0 {
            break;
        }
        let entry = unsafe { &*iov.add(index) };
        let take = remaining.min(entry.iov_len);
        if take == 0 {
            continue;
        }
        if entry.iov_base.is_null() {
            return None;
        }
        let bytes = unsafe { slice::from_raw_parts(entry.iov_base.cast::<u8>(), take) };
        scratch.extend_from_slice(bytes);
        remaining -= take;
    }
    if remaining != 0 {
        return None;
    }

    let filtered = filter(scratch.as_mut_slice()).min(payload_len);
    let mut copied = 0usize;
    for index in 0..iovlen {
        if copied == filtered {
            break;
        }
        let entry = unsafe { &*iov.add(index) };
        let take = (filtered - copied).min(entry.iov_len);
        if take == 0 {
            continue;
        }
        unsafe {
            ptr::copy_nonoverlapping(
                scratch.as_ptr().add(copied),
                entry.iov_base.cast::<u8>(),
                take,
            );
        }
        copied += take;
    }
    (copied == filtered).then_some(filtered)
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

    let data = unsafe { slice::from_raw_parts_mut(buf, ret as usize) };

    // Quick check: first message type must be one we filter. Route dumps
    // (RTM_GETROUTE) come back as RTM_NEWROUTE and must not be skipped —
    // that gap was the issue #86 `if<N>` leak.
    let nlmsg_type = u16::from_ne_bytes([data[4], data[5]]);
    if nlmsg_type != RTM_NEWADDR
        && nlmsg_type != RTM_NEWLINK
        && nlmsg_type != RTM_NEWROUTE
        && nlmsg_type != RTM_NEWRULE
    {
        return ret;
    }

    let (indices, n) = collect_vpn_iface_indices();
    if n == 0 {
        return ret;
    }
    let vpn_uid = unsafe { libc::getuid() };

    filter_netlink_dump(data, &indices[..n], vpn_uid) as isize
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

saved_original! {
    /// Raw function type for `recv`.
    type RecvFn = unsafe extern "C" fn(c_int, *mut c_void, usize, c_int) -> isize;
    static REAL_RECV;
    fn real_recv;
    pub fn set_real_recv_ptr;
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

saved_original! {
    /// Raw function type for `recvfrom`.
    type RecvfromFn =
        unsafe extern "C" fn(c_int, *mut c_void, usize, c_int, *mut c_void, *mut c_void) -> isize;
    static REAL_RECVFROM;
    fn real_recvfrom;
    pub fn set_real_recvfrom_ptr;
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

saved_original! {
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
    static REAL_RECVFROM_CHK;
    fn real_recvfrom_chk;
    pub fn set_real_recvfrom_chk_ptr;
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
fn collect_vpn_iface_indices() -> ([u32; MAX_VPN_ADDRS], usize) {
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
            let idx = with_ioctl_passthrough(|| libc::if_nametoindex(entry.ifa_name));
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
mod setsockopt_tests {
    use core::ffi::{c_int, c_void};
    use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

    use super::{
        KERNEL_BIND_ERRNO, KERNEL_BIND_POLICY, KERNEL_BIND_POLICY_HOOK, SO_BINDTOIFINDEX,
        copy_from_self, deny_ifindex_bind, hidden_bind_errno, hooked_setsockopt,
        release_has_unprivileged_first_bind, set_errno, set_real_setsockopt_ptr,
    };

    static REAL_CALLS: AtomicUsize = AtomicUsize::new(0);
    static LAST_OPTLEN: AtomicU32 = AtomicU32::new(0);

    // socklen_t is u32 on 64-bit bionic but i32 on 32-bit (armeabi-v7a); the
    // `as u32` below is required on arm and a no-op on arm64, so allow the
    // "unnecessary cast" clippy fires on the 64-bit build.
    #[allow(clippy::unnecessary_cast)]
    unsafe extern "C" fn fake_setsockopt(
        _fd: c_int,
        _level: c_int,
        _optname: c_int,
        _optval: *const c_void,
        optlen: libc::socklen_t,
    ) -> c_int {
        REAL_CALLS.fetch_add(1, Ordering::Relaxed);
        LAST_OPTLEN.store(optlen as u32, Ordering::Relaxed);
        73
    }

    #[test]
    fn self_copy_contains_bad_caller_pointers() {
        let source = *b"tun0";
        let mut destination = [0u8; 4];
        assert!(copy_from_self(source.as_ptr().cast(), &mut destination));
        assert_eq!(destination, source);

        assert!(!copy_from_self(
            core::ptr::dangling::<c_void>(),
            &mut destination
        ));
    }

    /// Hiding an interface means answering exactly like an absent one, and which
    /// errno that is depends on the kernel's own check order — EPERM on trees
    /// that test CAP_NET_RAW before parsing the name, ENODEV on trees that
    /// resolve first. Mirroring the measured answer is what stops the reply
    /// being an oracle.
    #[test]
    fn hidden_bind_mirrors_the_kernels_answer_for_an_absent_name() {
        // 5.7+ style: an absent name is ENODEV, so a hidden one is too.
        assert_eq!(
            hidden_bind_errno(Some(libc::ENODEV), true),
            Some(libc::ENODEV)
        );
        // Gated tree: every bind is EPERM, so denying with ENODEV would single
        // the VPN out. Mirror EPERM instead.
        assert_eq!(
            hidden_bind_errno(Some(libc::EPERM), false),
            Some(libc::EPERM)
        );
        // Any other refusal is mirrored verbatim rather than reinterpreted.
        assert_eq!(
            hidden_bind_errno(Some(libc::EINVAL), true),
            Some(libc::EINVAL)
        );
    }

    #[test]
    fn an_unusable_probe_falls_back_to_the_release_heuristic() {
        // Probe failed: behave exactly as before it existed.
        assert_eq!(hidden_bind_errno(None, true), Some(libc::ENODEV));
        assert_eq!(hidden_bind_errno(None, false), None);
        // An absent name that binds successfully tells us nothing usable.
        assert_eq!(hidden_bind_errno(Some(0), true), Some(libc::ENODEV));
        assert_eq!(hidden_bind_errno(Some(0), false), None);
    }

    #[test]
    fn ifindex_decision_denies_vpn_and_resolution_races_only() {
        assert!(deny_ifindex_bind(42, Some(true)));
        assert!(deny_ifindex_bind(42, None));
        assert!(!deny_ifindex_bind(42, Some(false)));
        assert!(!deny_ifindex_bind(0, Some(true)));
        assert!(!deny_ifindex_bind(-1, Some(true)));
    }

    #[test]
    fn socket_bind_oracle_starts_at_linux_5_7() {
        assert!(!release_has_unprivileged_first_bind(b"4.19.325-android"));
        assert!(!release_has_unprivileged_first_bind(b"5.6.19"));
        assert!(release_has_unprivileged_first_bind(b"5.7.0"));
        assert!(release_has_unprivileged_first_bind(b"6.1.134-gki"));
        assert!(!release_has_unprivileged_first_bind(b"not-a-release"));
    }

    #[test]
    fn libc_hook_denies_vpn_names_before_calling_the_real_function() {
        set_real_setsockopt_ptr(fake_setsockopt as *const ());
        KERNEL_BIND_POLICY.store(KERNEL_BIND_POLICY_HOOK, Ordering::Relaxed);
        REAL_CALLS.store(0, Ordering::Relaxed);
        LAST_OPTLEN.store(u32::MAX, Ordering::Relaxed);
        let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
        assert!(fd >= 0);

        let vpn = *b"tun0";
        set_errno(0);
        let rc = unsafe {
            hooked_setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_BINDTODEVICE,
                vpn.as_ptr().cast(),
                vpn.len() as libc::socklen_t,
            )
        };
        assert_eq!(rc, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ENODEV)
        );
        assert_eq!(REAL_CALLS.load(Ordering::Relaxed), 0);

        // On a tree where every bind is refused for lack of CAP_NET_RAW, the
        // denial has to mirror that refusal — answering ENODEV only for the VPN
        // name would announce it. Same call, different measured kernel.
        KERNEL_BIND_ERRNO.store(libc::EPERM, Ordering::Relaxed);
        set_errno(0);
        let rc = unsafe {
            hooked_setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_BINDTODEVICE,
                vpn.as_ptr().cast(),
                vpn.len() as libc::socklen_t,
            )
        };
        assert_eq!(rc, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::EPERM)
        );
        assert_eq!(REAL_CALLS.load(Ordering::Relaxed), 0);
        KERNEL_BIND_ERRNO.store(libc::ENODEV, Ordering::Relaxed);

        let physical = *b"eth0";
        let rc = unsafe {
            hooked_setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_BINDTODEVICE,
                physical.as_ptr().cast(),
                physical.len() as libc::socklen_t,
            )
        };
        assert_eq!(rc, 73);
        assert_eq!(REAL_CALLS.load(Ordering::Relaxed), 1);
        assert_eq!(LAST_OPTLEN.load(Ordering::Relaxed), 4);

        // Empty names unbind and must retain the kernel's native behavior.
        let rc = unsafe {
            hooked_setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_BINDTODEVICE,
                core::ptr::null(),
                0,
            )
        };
        assert_eq!(rc, 73);
        assert_eq!(REAL_CALLS.load(Ordering::Relaxed), 2);

        // A bad pointer is handed to the real syscall, which remains the
        // authority for EFAULT; the hook itself must not dereference it.
        let rc = unsafe {
            hooked_setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_BINDTODEVICE,
                core::ptr::dangling::<c_void>(),
                4,
            )
        };
        assert_eq!(rc, 73);
        assert_eq!(REAL_CALLS.load(Ordering::Relaxed), 3);

        // Wrapped signed lengths likewise stay on the native path.
        let rc = unsafe {
            hooked_setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_BINDTODEVICE,
                core::ptr::dangling::<c_void>(),
                c_int::MAX as libc::socklen_t + 1,
            )
        };
        assert_eq!(rc, 73);
        assert_eq!(REAL_CALLS.load(Ordering::Relaxed), 4);

        // Index zero is an unbind and the real function receives one validated
        // int rather than a potentially mutable caller buffer.
        let ifindex = 0i32;
        let rc = unsafe {
            hooked_setsockopt(
                fd,
                libc::SOL_SOCKET,
                SO_BINDTOIFINDEX,
                (&ifindex as *const c_int).cast(),
                core::mem::size_of::<c_int>() as libc::socklen_t,
            )
        };
        assert_eq!(rc, 73);
        assert_eq!(REAL_CALLS.load(Ordering::Relaxed), 5);
        assert_eq!(
            LAST_OPTLEN.load(Ordering::Relaxed),
            core::mem::size_of::<c_int>() as u32
        );

        // Validation ordering matters: even a VPN-looking value on an invalid
        // fd belongs to the real syscall's EBADF path, not our ENODEV path.
        let rc = unsafe {
            hooked_setsockopt(
                -1,
                libc::SOL_SOCKET,
                libc::SO_BINDTODEVICE,
                vpn.as_ptr().cast(),
                vpn.len() as libc::socklen_t,
            )
        };
        assert_eq!(rc, 73);
        assert_eq!(REAL_CALLS.load(Ordering::Relaxed), 6);

        unsafe {
            libc::close(fd);
        }
    }
}

#[cfg(test)]
mod iovec_tests {
    use super::{iovec_payload_len, rewrite_iovec_payload};
    use crate::filter::{RTM_NEWLINK, filter_netlink_dump};

    fn make_nlmsg(if_index: u32) -> Vec<u8> {
        let mut msg = Vec::new();
        msg.extend_from_slice(&24u32.to_ne_bytes());
        msg.extend_from_slice(&RTM_NEWLINK.to_ne_bytes());
        msg.extend_from_slice(&0u16.to_ne_bytes());
        msg.extend_from_slice(&1u32.to_ne_bytes());
        msg.extend_from_slice(&0u32.to_ne_bytes());
        msg.extend_from_slice(&[0u8; 4]);
        msg.extend_from_slice(&if_index.to_ne_bytes());
        msg
    }

    #[test]
    fn filters_netlink_messages_across_iovec_boundaries() {
        let mut payload = Vec::new();
        payload.extend(make_nlmsg(2));
        payload.extend(make_nlmsg(7));
        payload.extend(make_nlmsg(3));
        let mut expected = Vec::new();
        expected.extend(make_nlmsg(2));
        expected.extend(make_nlmsg(3));

        // Include an empty first iovec and split the first netlink header over
        // the next two. A caller using this layout previously bypassed recvmsg
        // filtering because only iov[0] was inspected.
        let mut first = payload[..7].to_vec();
        let mut rest = payload[7..].to_vec();
        let mut iovecs = [
            libc::iovec {
                iov_base: core::ptr::null_mut(),
                iov_len: 0,
            },
            libc::iovec {
                iov_base: first.as_mut_ptr().cast(),
                iov_len: first.len(),
            },
            libc::iovec {
                iov_base: rest.as_mut_ptr().cast(),
                iov_len: rest.len(),
            },
        ];
        let mut scratch = Vec::new();

        let copied =
            unsafe { iovec_payload_len(iovecs.as_ptr(), iovecs.len(), payload.len()).unwrap() };
        let filtered = unsafe {
            rewrite_iovec_payload(
                iovecs.as_mut_ptr(),
                iovecs.len(),
                copied,
                &mut scratch,
                |data| filter_netlink_dump(data, &[7], 0),
            )
            .unwrap()
        };

        let mut actual = first;
        actual.extend(rest);
        assert_eq!(filtered, expected.len());
        assert_eq!(&actual[..filtered], expected.as_slice());
    }
}

#[cfg(test)]
mod proc_net_path_tests {
    use super::{ProcNetFile, match_abs_proc_net, strip_owner_net};

    #[test]
    fn matches_classic_forms() {
        assert_eq!(match_abs_proc_net(b"/proc/net/tcp"), Some(ProcNetFile::Tcp));
        assert_eq!(
            match_abs_proc_net(b"/proc/self/net/tcp6"),
            Some(ProcNetFile::Tcp6)
        );
        assert_eq!(
            match_abs_proc_net(b"/proc/1234/net/route"),
            Some(ProcNetFile::Route)
        );
    }

    #[test]
    fn matches_new_dev_udp_files() {
        assert_eq!(match_abs_proc_net(b"/proc/net/dev"), Some(ProcNetFile::Dev));
        assert_eq!(match_abs_proc_net(b"/proc/net/udp"), Some(ProcNetFile::Udp));
        assert_eq!(
            match_abs_proc_net(b"/proc/net/udp6"),
            Some(ProcNetFile::Udp6)
        );
    }

    #[test]
    fn matches_thread_self_and_task_forms() {
        // These previously bypassed the matcher (#14/#52).
        assert_eq!(
            match_abs_proc_net(b"/proc/thread-self/net/tcp"),
            Some(ProcNetFile::Tcp)
        );
        assert_eq!(
            match_abs_proc_net(b"/proc/1234/task/5678/net/tcp"),
            Some(ProcNetFile::Tcp)
        );
        assert_eq!(
            match_abs_proc_net(b"/proc/self/net/dev"),
            Some(ProcNetFile::Dev)
        );
    }

    #[test]
    fn rejects_non_proc_net() {
        assert_eq!(match_abs_proc_net(b"/proc/net/unknownfile"), None);
        assert_eq!(match_abs_proc_net(b"/etc/passwd"), None);
        assert_eq!(match_abs_proc_net(b"/proc/abc/net/tcp"), None);
        assert_eq!(strip_owner_net(b"bogus/path"), None);
    }
}
