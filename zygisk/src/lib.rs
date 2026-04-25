//! vpnhide-zygisk — a Zygisk (NeoZygisk) module that hides an active
//! Android VPN from selected apps by hooking the libc network-introspection
//! syscalls that the apps use to detect `tun0`/`wg0`/etc.
//!
//! ## High-level flow
//!
//! 1. **`on_load`** — stash nothing, just log. The target app's native
//!    libraries aren't loaded yet at this stage so we can't PLT-hook them.
//! 2. **`preAppSpecialize`** — we get told which app is about to be forked
//!    (via `args.nice_name`). Look up the package name against our
//!    allowlist file (`/data/adb/modules/vpnhide_zygisk/targets.txt`). If
//!    the current app isn't in the list, set `DlcloseModuleLibrary` and
//!    return — the module is unloaded cleanly after specialization. If
//!    it IS in the list, stash a flag and wait for post-specialize.
//! 3. **`postAppSpecialize`** — the app's main native libraries have now
//!    been loaded into the process. Parse `/proc/self/maps`, collect every
//!    distinct ELF file backing an executable mapping, and register a PLT
//!    hook on each that redirects `ioctl`, `getifaddrs`, and `openat`
//!    to our replacement functions. From this point on, any call from
//!    any loaded library into these libc symbols goes through our filter.
//!
//! ## Module metadata
//!
//! The KernelSU module install script places this shared library at
//! `/data/adb/modules/vpnhide_zygisk/zygisk/arm64-v8a.so`. NeoZygisk
//! injects it into every forked app process; the `preAppSpecialize` filter
//! ensures we only actually do work for targeted apps.

mod filter;
mod generated;
mod hooks;
mod shadowhook;

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::Once;

use jni::JNIEnv;
use log::{debug, error, info};
use zygisk_api::ZygiskModule;
use zygisk_api::api::ZygiskApi;
use zygisk_api::api::v2::{AppSpecializeArgs, V2, ZygiskOption};

use crate::hooks::{
    hooked_getifaddrs, hooked_ioctl, hooked_openat, hooked_recv, hooked_recvmsg,
    set_real_getifaddrs_ptr, set_real_ioctl_ptr, set_real_openat_ptr, set_real_recv_ptr,
    set_real_recvmsg_ptr,
};

const LOG_TAG: &str = "vpnhide-zygisk";
const APP_PACKAGE: &str = "dev.okhsunrog.vpnhide";
const APP_STATUS_FILE: &str = "/data/user/0/dev.okhsunrog.vpnhide/files/vpnhide_zygisk_active";
/// Path to the user's allowlist. Lives OUTSIDE the module directory so
/// it survives module updates (KSU/Magisk wipe `/data/adb/modules/<id>/`
/// on every install). `customize.sh` is responsible for creating the
/// directory and migrating the legacy in-module file on first run.
/// Targets filename within the module directory.
const TARGETS_FILENAME: &str = "targets.txt";
/// Runtime debug-logging flag file. Written by the VPN Hide app when
/// the user toggles the setting. Absent or not "1" ⇒ logging is off —
/// stealth-first default matches the rest of the project.
const DEBUG_LOGGING_FILENAME: &str = "debug_logging";

/// Initialize `android_logger` exactly once. Cheap to call from every
/// forked process — subsequent calls are no-ops. The compile-time log
/// filter is controlled by the `log` crate's `release_max_level_*`
/// Cargo feature (see our `Cargo.toml`); anything below that level is
/// monomorphized away to a no-op. Runtime level is then narrowed by
/// [apply_debug_logging_flag] so the user's toggle silences logcat.
fn init_logger() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        android_logger::init_once(
            android_logger::Config::default()
                .with_tag(LOG_TAG)
                .with_max_level(log::LevelFilter::Trace),
        );
    });
}

/// Read the debug-logging flag file from the module dir fd and drop
/// the global `log` filter to `Off` unless the flag is set to `1`.
/// Any read error is treated as "disabled": if the file doesn't exist
/// yet (fresh install, flag never toggled), we shouldn't leak logs.
fn apply_debug_logging_flag(dir_fd: std::os::fd::RawFd) {
    use std::io::Read;
    use std::os::fd::FromRawFd;
    let enabled = (|| -> Option<bool> {
        let filename = std::ffi::CString::new(DEBUG_LOGGING_FILENAME).ok()?;
        let fd =
            unsafe { libc::openat(dir_fd, filename.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
        if fd < 0 {
            return None;
        }
        let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
        let mut content = String::new();
        file.read_to_string(&mut content).ok()?;
        Some(content.trim() == "1")
    })()
    .unwrap_or(false);
    // Leave errors on even when the user disables logging — the handful of
    // `error!` calls (hook-install failures, status-file write failures)
    // fire at most once per process and are the only signal we have if
    // things go wrong. Same principle as HookLog.e on the Kotlin side.
    let level = if enabled {
        log::LevelFilter::Trace
    } else {
        log::LevelFilter::Error
    };
    log::set_max_level(level);
}

/// The module struct. Held as a `Default` singleton by the
/// `register_module!` macro.
#[derive(Default)]
pub struct VpnHide {
    /// Set by `preAppSpecialize` if the forked process is a target we want
    /// to hook. Read by `postAppSpecialize`. Accessed single-threaded
    /// (Zygisk calls pre/post sequentially on the zygote main thread).
    is_target: core::cell::Cell<bool>,
    /// True only when the currently specializing app is the VPN Hide app
    /// itself, so we can write a heartbeat the dashboard can trust.
    report_status: core::cell::Cell<bool>,
}

// Single-threaded access by construction.
unsafe impl Sync for VpnHide {}

/// Cached targets loaded via Zygisk's module dir fd.
static CACHED_TARGETS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

fn parse_targets(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                None
            } else {
                Some(line.to_string())
            }
        })
        .collect()
}

/// Read targets.txt via the module directory fd provided by Zygisk.
/// This fd is opened by Zygisk with root privileges, bypassing SELinux
/// restrictions that block direct file access on Magisk.
fn load_targets_from_dir_fd(dir_fd: std::os::fd::RawFd) -> Vec<String> {
    use std::io::Read;
    use std::os::fd::FromRawFd;
    let filename = std::ffi::CString::new(TARGETS_FILENAME).unwrap();
    let fd = unsafe { libc::openat(dir_fd, filename.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    if fd < 0 {
        log::warn!(
            "can't open {TARGETS_FILENAME} via module dir fd: {}",
            std::io::Error::last_os_error()
        );
        return Vec::new();
    }
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    let mut content = String::new();
    if let Err(e) = file.read_to_string(&mut content) {
        log::warn!("can't read {TARGETS_FILENAME}: {e}");
        return Vec::new();
    }
    parse_targets(&content)
}

impl ZygiskModule for VpnHide {
    type Api = V2;

    fn on_load(&self, api: ZygiskApi<'_, V2>, _env: JNIEnv<'_>) {
        init_logger();
        // Zygisksu returns a raw fd to /data/adb/modules/<id> that we own.
        // Wrap it in OwnedFd so the drop at the end of this function closes
        // it before zygote forks any app. If we leak it, every app forked
        // after us inherits an fd pointing inside /data/adb/ — and apps
        // that scan /proc/self/fd for root-managed paths (e.g. Ozon's
        // anti-tamper SDK) detect it and refuse to run, even though our
        // .so is dlclosed for non-target apps.
        let dir_fd = unsafe { OwnedFd::from_raw_fd(api.get_module_dir()) };
        // Apply the user's debug-logging preference before anything below
        // gets a chance to log. Default is Off, so silence is the no-config
        // behavior even on a fresh install where the flag file is absent.
        apply_debug_logging_flag(dir_fd.as_raw_fd());
        CACHED_TARGETS.get_or_init(|| load_targets_from_dir_fd(dir_fd.as_raw_fd()));
        debug!(
            "on_load: {} targets cached",
            CACHED_TARGETS.get().map_or(0, |v| v.len())
        );
        // dir_fd drops here → closed before any app fork.
    }

    fn pre_app_specialize<'a>(
        &self,
        mut api: ZygiskApi<'a, V2>,
        env: JNIEnv<'a>,
        args: &'a mut AppSpecializeArgs<'_>,
    ) {
        let package = read_jstring(&env, args.nice_name);
        match package.as_deref() {
            Some(p) if is_targeted(p) => {
                info!("pre_app_specialize: targeting {p}");
                self.is_target.set(true);
                self.report_status.set(p == APP_PACKAGE);
            }
            _ => {
                self.is_target.set(false);
                self.report_status.set(false);
                mark_cleanup(&mut api);
            }
        }
    }

    fn post_app_specialize<'a>(
        &self,
        _api: ZygiskApi<'a, V2>,
        _env: JNIEnv<'a>,
        _args: &'a AppSpecializeArgs<'_>,
    ) {
        if !self.is_target.get() {
            return;
        }
        match install_hooks() {
            Ok(()) => {
                info!("hooks installed (inline libc!ioctl + getifaddrs + openat for proc/net/*)");
                if self.report_status.get() {
                    write_status_file();
                }
                // Erase shadowhook's fingerprints from /proc/self/maps
                // before any anti-tamper SDK gets a chance to scan
                // them via raw syscalls.
                scrub_shadowhook_maps();
            }
            Err(err) => error!("install_hooks failed: {err}"),
        }
    }

    // No pre_server_specialize override — the trait default is empty,
    // which is what we want: system_server isn't in scope for this module.
}

fn write_status_file() {
    let boot_id = match std::fs::read_to_string("/proc/sys/kernel/random/boot_id") {
        Ok(v) => v.trim().to_string(),
        Err(err) => {
            error!("failed to read boot_id: {err}");
            return;
        }
    };
    let timestamp = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(v) => v.as_secs(),
        Err(err) => {
            error!("failed to get timestamp: {err}");
            return;
        }
    };
    let content = format!(
        "version={}\nboot_id={}\npid={}\ntimestamp={}\n",
        env!("CARGO_PKG_VERSION"),
        boot_id,
        std::process::id(),
        timestamp,
    );
    if let Some(parent) = std::path::Path::new(APP_STATUS_FILE).parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            error!("failed to create status dir {}: {err}", parent.display());
            return;
        }
    }
    match std::fs::write(APP_STATUS_FILE, content) {
        Ok(()) => info!("wrote zygisk status heartbeat to {APP_STATUS_FILE}"),
        Err(err) => error!("failed to write status heartbeat: {err}"),
    }
}

/// Tell Zygisk to `dlclose` our .so once the current callback returns.
/// Saves memory in every process where we don't actually hook anything.
fn mark_cleanup(api: &mut ZygiskApi<'_, V2>) {
    api.set_option(ZygiskOption::DlCloseModuleLibrary);
}

/// Install inline hooks on `libc.so` via ByteDance shadowhook. We patch
/// three entry points:
///
///   * `ioctl` — catches `SIOCGIFNAME` / `SIOCGIFFLAGS` interface
///     probes from native code.
///   * `getifaddrs` — catches the higher-level interface enumeration
///     API used by `NetworkInterface.getNetworkInterfaces()`
///     inside libcore, by the Dart VM's
///     `NetworkInterface.list()`, and by anything in C/C++
///     that calls `getifaddrs()` directly.
///   * `openat` — intercepts opens of `/proc/net/{route,ipv6_route,
///     if_inet6,tcp,tcp6}`; returns a `memfd` with VPN
///     entries stripped out.
///   * `recvmsg` — filters netlink `RTM_NEWADDR` / `RTM_NEWLINK`
///     dump responses, removing VPN interface entries.
///
/// This replaces the earlier PLT-hook approach. PLT hooks can only patch
/// callers that are already mapped at `post_app_specialize` time — which
/// excludes `libflutter.so`/`libapp.so` and any other library loaded later
/// via `dlopen`. Inline-hooking libc's entry points themselves catches
/// every caller regardless of load order.
fn install_hooks() -> Result<(), String> {
    shadowhook::init_once().map_err(|rc| format!("shadowhook_init: rc={rc}"))?;

    hook_libc_sym(c"ioctl", hooked_ioctl as *mut _, set_real_ioctl_ptr)?;
    hook_libc_sym(
        c"getifaddrs",
        hooked_getifaddrs as *mut _,
        set_real_getifaddrs_ptr,
    )?;
    hook_libc_sym(c"openat", hooked_openat as *mut _, set_real_openat_ptr)?;
    hook_libc_sym(c"recvmsg", hooked_recvmsg as *mut _, set_real_recvmsg_ptr)?;
    // Hook recv directly. We cannot hook recvfrom because bionic's recv()
    // does a bare `b recvfrom` — patching recvfrom's prologue breaks recv.
    // recv itself is 12 bytes (3 instructions), safe for island-mode hooking.
    hook_libc_sym(c"recv", hooked_recv as *mut _, set_real_recv_ptr)?;

    Ok(())
}

/// Install a single inline hook on a libc symbol and stash the original
/// trampoline via `store_orig`. Used for every entry point from
/// `install_hooks`.
fn hook_libc_sym(
    sym: &core::ffi::CStr,
    new_fn: *mut core::ffi::c_void,
    store_orig: fn(*const ()),
) -> Result<(), String> {
    let mut orig: *mut core::ffi::c_void = core::ptr::null_mut();
    // SAFETY: `new_fn` has an ABI-compatible signature with the target
    // libc symbol; `&mut orig` is a valid writable pointer.
    let stub = unsafe { shadowhook::hook_sym(c"libc.so", sym, new_fn, &mut orig) };
    if stub.is_null() {
        return Err(format!(
            "shadowhook_hook_sym_name(libc.so, {}) returned null",
            sym.to_string_lossy()
        ));
    }
    if orig.is_null() {
        return Err(format!(
            "shadowhook returned null trampoline for libc.so!{}",
            sym.to_string_lossy()
        ));
    }
    store_orig(orig as *const ());
    Ok(())
}

// ============================================================================
//  Anti-detection: scrub shadowhook fingerprints from /proc/self/maps
// ============================================================================

/// After shadowhook installs inline hooks it leaves two named anonymous
/// memory regions visible in `/proc/self/maps`:
///
///   - `[anon:shadowhook-island]`  — trampoline island
///   - `[anon:shadowhook-enter]`   — hook entry stubs
///
/// Some anti-tamper SDKs read `/proc/self/maps` via raw `svc #0`
/// syscalls — completely bypassing any libc hook we could place — and
/// scan for known hooking framework names. If they see "shadowhook"
/// they abort the process.
///
/// Fix: rename those regions to an empty string via `prctl(PR_SET_VMA,
/// PR_SET_VMA_ANON_NAME, ...)`. The kernel updates the name in its VMA
/// metadata, so subsequent reads of `/proc/self/maps` (via any path,
/// including raw syscalls) will show a plain `[anon:]` entry,
/// indistinguishable from the hundreds of other anonymous mappings in
/// any Android process.
///
/// Must be called immediately after `install_hooks()` — before the app's
/// ContentProviders are initialized.
fn scrub_shadowhook_maps() {
    let names_to_scrub: &[&str] = &["shadowhook-island", "shadowhook-enter"];
    // Use raw open to bypass our own hooked_openat — /proc/self/maps is
    // not in the filter list today, but a future addition would silently
    // break this function.
    let maps = {
        use std::io::Read;
        use std::os::fd::FromRawFd;
        let fd = unsafe {
            libc::open(
                c"/proc/self/maps".as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            log::warn!("scrub_shadowhook_maps: can't open /proc/self/maps");
            return;
        }
        let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
        let mut buf = String::new();
        if file.read_to_string(&mut buf).is_err() {
            log::warn!("scrub_shadowhook_maps: can't read /proc/self/maps");
            return;
        }
        buf
    };

    for line in maps.lines() {
        // Format: "start-end perms offset dev inode  pathname"
        // For named anon regions: "7ff152000-7ff153000 ... [anon:shadowhook-island]"
        let should_scrub = names_to_scrub
            .iter()
            .any(|name| line.contains(&format!("[anon:{name}]")));
        if !should_scrub {
            continue;
        }

        // Parse the start-end addresses from the first column.
        let Some(range) = line.split_whitespace().next() else {
            continue;
        };
        let Some((start_hex, end_hex)) = range.split_once('-') else {
            continue;
        };
        let Ok(start) = usize::from_str_radix(start_hex, 16) else {
            continue;
        };
        let Ok(end) = usize::from_str_radix(end_hex, 16) else {
            continue;
        };
        let len = end.saturating_sub(start);
        if len == 0 {
            continue;
        }

        // PR_SET_VMA = 0x53564d41, PR_SET_VMA_ANON_NAME = 0
        // prctl(PR_SET_VMA, PR_SET_VMA_ANON_NAME, addr, len, name)
        // Setting name to an empty C string "" makes the region show as
        // plain "[anon:]" in maps.
        let rc = unsafe {
            libc::prctl(
                0x53564d41_u32 as libc::c_int, // PR_SET_VMA
                0,                             // PR_SET_VMA_ANON_NAME
                start,
                len,
                c"".as_ptr(),
            )
        };
        if rc == 0 {
            debug!("scrubbed anon region at {start_hex}-{end_hex}");
        } else {
            log::warn!(
                "prctl(PR_SET_VMA_ANON_NAME) failed for {start_hex}-{end_hex}: errno={}",
                std::io::Error::last_os_error()
            );
        }
    }
}

/// Is this package on our allowlist?
///
/// Called from `pre_app_specialize`, which runs on the zygote side BEFORE
/// the uid drop and SELinux context transition, so `/data/adb/modules/...`
/// is still readable here.
///
/// An entry matches if the target file contains either the exact package
/// name (e.g. `com.example.app`) or the process base name that is the
/// package of a multi-process app (e.g. `com.example.app:background` is
/// matched by an entry for `com.example.app`). This means a single line
/// per app in `targets.txt` covers all of its subprocesses.
#[inline(never)]
fn is_targeted(package: &str) -> bool {
    let targets = match CACHED_TARGETS.get() {
        Some(t) => t,
        None => return false,
    };

    let base_package = match package.split_once(':') {
        Some((base, _)) => base,
        None => package,
    };

    targets.iter().any(|t| t == package || t == base_package)
}

/// Decode a `JString` (as stored in `AppSpecializeArgs::nice_name`) into
/// an owned Rust `String`. Returns None on any failure.
fn read_jstring<'a>(env: &JNIEnv<'a>, jstr: &jni::objects::JString<'a>) -> Option<String> {
    if jstr.is_null() {
        return None;
    }
    let mut env_clone = unsafe { env.unsafe_clone() };
    env_clone
        .get_string(jstr)
        .ok()
        .and_then(|s| s.to_str().ok().map(|s| s.to_string()))
}

zygisk_api::register_module!(VpnHide);

// Empty companion — declared so we don't have to recompile the .so if we
// ever want to add a root-privileged helper later.
zygisk_api::register_companion!(|_| ());
