//! vpnhide-zygisk — a Zygisk (NeoZygisk) module that hides an active
//! Android VPN from selected apps by hooking the libc network-introspection
//! syscalls that the apps use to detect `tun0`/`wg0`/etc.
//!
//! ## Zygisk module lifecycle (where things actually run)
//!
//! This is the part that's easy to get wrong on the first read, so it's
//! spelled out here. Sources verified against:
//!
//!   * Magisk upstream `api.hpp` (v2):
//!     <https://github.com/topjohnwu/Magisk/blob/06531f6d06a73b4770762964e41201b9f157923b/native/jni/zygisk/api.hpp>
//!     — explicitly: _"modules will only be loaded after zygote has forked
//!     the child process. THIS MEANS ALL OF YOUR CODE RUNS IN THE
//!     APP/SYSTEM SERVER PROCESS, NOT THE ZYGOTE DAEMON!"_
//!   * `zygisk-api-rs` trait docs (`api/mod.rs:26-27`): _"This method gets
//!     called as soon as the Zygisk module gets loaded into the target
//!     process"_.
//!   * NeoZygisk loader `module.cpp::run_modules_pre()`: `DlopenMem(...)`
//!     then `m.onLoad(env)` — both inside the `fork_pre`-child branch.
//!
//! Concrete sequence, per app launch:
//!
//! 1. Zygote receives a request to spawn an app.
//! 2. Zygote calls `fork()`. Two processes now exist; the parent (zygote)
//!    returns to its event loop. Everything below runs in the **child**.
//! 3. NeoZygisk's loader `dlopen`s our `arm64-v8a.so` from a memfd in the
//!    child. This is when our static initialisers run and our
//!    `module_entry` symbol gets resolved.
//! 4. **`on_load`** — first callback we get. Runs in the freshly-forked,
//!    not-yet-specialised child. We have root privileges here (zygote
//!    privileges, not yet dropped). The Zygisk API gives us a fd to
//!    `/data/adb/modules/<id>` via `get_module_dir()` — usable now,
//!    closed by SELinux later.
//! 5. **`pre_app_specialize`** — last call before the kernel transitions
//!    the process to the app's UID and SELinux context. `args.uid` holds the
//!    UID this fork will become; we match it against the config's target UIDs
//!    to decide whether to install hooks. For non-targets we set
//!    `DlCloseModuleLibrary`.
//! 6. The kernel/zygote applies app specialisation: setuid to the app's
//!    UID, switch SELinux context, install seccomp filter, drop caps.
//! 7. **`post_app_specialize`** — runs as the app, before the app's
//!    `main()` / `ContentProvider.onCreate()`. Native libs from the APK
//!    haven't loaded yet either; that's why we install inline hooks
//!    on `libc.so` itself instead of PLT-hooking each caller.
//! 8. NeoZygisk `dlclose`s our `.so` from this child if we set
//!    `DlCloseModuleLibrary` in step 5; otherwise the .so stays mapped
//!    until the app exits.
//!
//! ## Implications for state (the easy-to-miss part)
//!
//! Because the `.so` is `dlopen`ed afresh in every child, **every Rust
//! `static` is reset to its initial state on every app launch**. Concretely:
//!
//!   * `static CACHED_CONFIG: OnceLock<…>` re-initialises in every
//!     forked child. `targets.txt` (a `vpnhide 1 config` snapshot) is read on
//!     every app launch — so a force-stop + restart of a target app picks up
//!     edits to the file immediately. There is no zygote-side cache to
//!     invalidate; live-reload is automatic by virtue of the lifecycle.
//!   * Saved-original libc pointers (`REAL_IOCTL` etc.) are also fresh
//!     per child, which is fine because we only ever set them inside
//!     `install_hooks()` from `post_app_specialize`.
//!   * No cross-app state can ever leak through Rust statics. Anything
//!     persistent must live on disk.
//!
//! Mental shortcut: think of the `.so` as if it were freshly compiled
//! and loaded into a brand-new process every time an app launches —
//! because that's literally what happens.
//!
//! ## Module metadata
//!
//! The KernelSU module install script places this shared library at
//! `/data/adb/modules/vpnhide_zygisk/zygisk/arm64-v8a.so`. NeoZygisk
//! injects it into every forked app process; the `pre_app_specialize`
//! filter ensures we only actually do work for targeted apps.

mod filter;
mod generated;
mod hooks;
mod shadowhook;

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::Once;

use log::{debug, error, info};
use vpnhide_protocol as protocol;
use vpnhide_protocol::hook_ids::{Hook, ZYGISK_HOOK_MASK};
use zygisk_api::ZygiskModule;
use zygisk_api::api::ZygiskApi;
use zygisk_api::api::v2::{AppSpecializeArgs, V2, ZygiskOption};
use zygisk_api::jni::JNIEnv;

use crate::hooks::{
    hooked_getifaddrs, hooked_ioctl, hooked_openat, hooked_recv, hooked_recvfrom,
    hooked_recvfrom_chk, hooked_recvmsg, set_real_getifaddrs_ptr, set_real_ioctl_ptr,
    set_real_openat_ptr, set_real_recv_ptr, set_real_recvfrom_chk_ptr, set_real_recvfrom_ptr,
    set_real_recvmsg_ptr,
};

const LOG_TAG: &str = "vpnhide-zygisk";
const APP_PACKAGE: &str = "dev.okhsunrog.vpnhide";
const APP_STATUS_FILE: &str = "/data/user/0/dev.okhsunrog.vpnhide/files/vpnhide_zygisk_active";
/// Config-snapshot filename within the module directory. Carries a
/// `vpnhide 1 config` payload (docs/protocol.md): the target UIDs this module
/// hides for, plus the folded `debug` flag — the same wire format every
/// backend speaks. Package→UID resolution is the producer's job (the app on
/// Save, the boot script at boot). Read once per app launch via the module
/// dir-fd.
const TARGETS_FILENAME: &str = "targets.txt";

/// Initialize `android_logger` exactly once. Cheap to call from every
/// forked process — subsequent calls are no-ops. The compile-time log
/// filter is controlled by the `log` crate's `release_max_level_*`
/// Cargo feature (see our `Cargo.toml`); anything below that level is
/// monomorphized away to a no-op. Runtime level is then narrowed by
/// [set_log_level] so the user's toggle silences logcat.
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

/// Narrow the global `log` filter to the user's debug-logging choice. The flag
/// is now folded into the config snapshot (`debug` line, §4.3) rather than a
/// separate file — `on_load` reads it from the same `targets.txt` it reads the
/// target UIDs from. Errors stay on even when disabled: the handful of
/// `error!` calls (hook-install failures, status-file write failures) fire at
/// most once per process and are the only signal we have if things go wrong.
/// Same principle as HookLog.e on the Kotlin side.
fn set_log_level(enabled: bool) {
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
    /// to hook. Carries only Zygisk-owned hook bits and is read by
    /// `postAppSpecialize`. Accessed single-threaded (Zygisk calls pre/post
    /// sequentially on the zygote main thread).
    target_hookmask: core::cell::Cell<u32>,
    /// True only when the currently specializing app is the VPN Hide app
    /// itself, so we can write a heartbeat the dashboard can trust.
    report_status: core::cell::Cell<bool>,
}

// Single-threaded access by construction.
unsafe impl Sync for VpnHide {}

/// The parsed control config this module acts on, cached once per process via
/// Zygisk's module dir fd.
///
/// **Lifetime is per app launch, not per zygote boot.** The `.so` is
/// `dlopen`ed fresh in every forked child, so this `OnceLock` starts empty and
/// gets initialised by `on_load` on every app launch. That means edits to
/// `targets.txt` are picked up on the next force-stop + restart — no zygote
/// restart, no reboot needed. See the lifecycle block at the top of this file.
///
/// `targets` carries one Zygisk-owned hook mask per target UID (protocol §4.3 —
/// UID is the key on every channel, including Zygisk); `debug` is the folded
/// debug flag.
struct ZygiskConfig {
    targets: Vec<protocol::Target>,
    debug: bool,
}

static CACHED_CONFIG: std::sync::OnceLock<ZygiskConfig> = std::sync::OnceLock::new();

/// Read + parse the `vpnhide 1 config` snapshot from targets.txt via the module
/// directory fd Zygisk provides. This fd is opened by Zygisk with root
/// privileges, bypassing the SELinux restrictions that block direct file
/// access on Magisk. A missing/unreadable/invalid file fails closed: no
/// targets, logging off.
fn load_config_from_dir_fd(dir_fd: std::os::fd::RawFd) -> ZygiskConfig {
    use std::io::Read;
    use std::os::fd::FromRawFd;
    let empty = ZygiskConfig {
        targets: Vec::new(),
        debug: false,
    };
    let filename = std::ffi::CString::new(TARGETS_FILENAME).unwrap();
    let fd = unsafe { libc::openat(dir_fd, filename.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    if fd < 0 {
        log::warn!(
            "can't open {TARGETS_FILENAME} via module dir fd: {}",
            std::io::Error::last_os_error()
        );
        return empty;
    }
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    let mut content = Vec::new();
    if let Err(e) = file.read_to_end(&mut content) {
        log::warn!("can't read {TARGETS_FILENAME}: {e}");
        return empty;
    }
    match protocol::parse_config(&content) {
        Some(cfg) => ZygiskConfig {
            targets: cfg
                .targets
                .iter()
                .filter_map(|t| {
                    let hookmask = t.hookmask & ZYGISK_HOOK_MASK;
                    (hookmask != 0).then_some(protocol::Target {
                        uid: t.uid,
                        hookmask,
                    })
                })
                .collect(),
            debug: cfg.debug.unwrap_or(false),
        },
        None => {
            // Rejected whole — bad/missing header, or a version newer than
            // this build knows (§3 version fuse). Fail closed.
            log::warn!("{TARGETS_FILENAME}: not a valid config snapshot; ignoring");
            empty
        }
    }
}

impl ZygiskModule for VpnHide {
    type Api = V2;

    fn on_load(&self, api: ZygiskApi<'_, V2>, _env: JNIEnv<'_>) {
        init_logger();
        // Zygisksu returns a raw fd to /data/adb/modules/<id> that we own.
        // Wrap it in OwnedFd so the drop at the end of this function closes
        // it before this app's code starts running. We are already in the
        // forked child here (see the lifecycle block at the top of this
        // file), so the fd would otherwise be visible to:
        //   1. The app itself reading /proc/self/fd — anti-tamper SDKs
        //      (e.g. Ozon's) scan for fds pointing inside /data/adb and
        //      refuse to run.
        //   2. Any sub-process the app spawns via fork()/Runtime.exec() —
        //      child inherits our open fd unless we close it here.
        // Either way the fd needs to die before pre_app_specialize returns.
        let dir_fd = unsafe { OwnedFd::from_raw_fd(api.get_module_dir()) };
        let cfg = CACHED_CONFIG.get_or_init(|| load_config_from_dir_fd(dir_fd.as_raw_fd()));
        // Debug is folded into the config snapshot now (§4.3); apply it before
        // anything below logs. Default Off ⇒ silence on a fresh/empty install.
        set_log_level(cfg.debug);
        debug!(
            "on_load: {} zygisk targets cached, debug={}",
            cfg.targets.len(),
            cfg.debug
        );
        // dir_fd drops here → closed before any app fork.
    }

    fn pre_app_specialize<'a>(
        &self,
        mut api: ZygiskApi<'a, V2>,
        env: JNIEnv<'a>,
        args: &'a mut AppSpecializeArgs<'_>,
    ) {
        // UID is the targeting key (protocol §4.3). args.uid already holds the
        // UID this child will become, and one UID covers every process of a
        // multi-process app — so no package/sub-process name matching needed.
        let uid = *args.uid as u32;
        // nice_name is still read, but only to recognise the VPN Hide app
        // itself for the status heartbeat — an identity check, not targeting.
        let package = read_jstring(&env, args.nice_name);
        let hookmask = target_hookmask(uid);
        if hookmask != 0 {
            info!("pre_app_specialize: targeting uid {uid} hooks=0x{hookmask:x}");
            self.target_hookmask.set(hookmask);
            self.report_status
                .set(package.as_deref() == Some(APP_PACKAGE));
        } else {
            self.target_hookmask.set(0);
            self.report_status.set(false);
            mark_cleanup(&mut api);
        }
    }

    fn post_app_specialize<'a>(
        &self,
        _api: ZygiskApi<'a, V2>,
        _env: JNIEnv<'a>,
        _args: &'a AppSpecializeArgs<'_>,
    ) {
        let hookmask = self.target_hookmask.get();
        if hookmask == 0 {
            return;
        }
        match install_hooks(hookmask) {
            Ok(()) => {
                info!("selected libc hooks installed (mask=0x{hookmask:x})");
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

/// Install selected inline hooks on `libc.so` via ByteDance shadowhook.
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
///   * `recvmsg` / `recv` / `recvfrom` / `__recvfrom_chk` — filter netlink
///     `RTM_NEWADDR` / `RTM_NEWLINK` dump responses, removing VPN interface
///     entries.
///
/// This replaces the earlier PLT-hook approach. PLT hooks can only patch
/// callers that are already mapped at `post_app_specialize` time — which
/// excludes `libflutter.so`/`libapp.so` and any other library loaded later
/// via `dlopen`. Inline-hooking libc's entry points themselves catches
/// every caller regardless of load order.
fn install_hooks(hookmask: u32) -> Result<(), String> {
    shadowhook::init_once().map_err(|rc| format!("shadowhook_init: rc={rc}"))?;

    // Every libc symbol the Zygisk backend can hook, in install order (we roll
    // back LIFO on failure). Each row is gated by its per-hook bit in
    // `hookmask`; adding a hook is one row here plus a `saved_original!` block
    // and a `hooked_*` fn — no parallel match arms to keep in sync.
    //
    // recv is hooked directly because bionic's `recv()` tail-calls recvfrom via
    // a bare `b` branch — patching recvfrom's prologue would break recv. recv
    // itself is 12 bytes (3 instructions), safe for island-mode hooking.
    // recvfrom + __recvfrom_chk catch FORTIFY'd / direct callers that never
    // touch the `recv` symbol (issue #86 native route dump).
    struct HookDesc {
        sym: &'static core::ffi::CStr,
        hook_id: Hook,
        replacement: *mut core::ffi::c_void,
        store_orig: fn(*const ()),
    }
    let table = [
        HookDesc {
            sym: c"ioctl",
            hook_id: Hook::ZygiskIoctl,
            replacement: hooked_ioctl as *mut _,
            store_orig: set_real_ioctl_ptr,
        },
        HookDesc {
            sym: c"getifaddrs",
            hook_id: Hook::ZygiskGetifaddrs,
            replacement: hooked_getifaddrs as *mut _,
            store_orig: set_real_getifaddrs_ptr,
        },
        HookDesc {
            sym: c"openat",
            hook_id: Hook::ZygiskOpenat,
            replacement: hooked_openat as *mut _,
            store_orig: set_real_openat_ptr,
        },
        HookDesc {
            sym: c"recvmsg",
            hook_id: Hook::ZygiskRecvmsg,
            replacement: hooked_recvmsg as *mut _,
            store_orig: set_real_recvmsg_ptr,
        },
        HookDesc {
            sym: c"recv",
            hook_id: Hook::ZygiskRecv,
            replacement: hooked_recv as *mut _,
            store_orig: set_real_recv_ptr,
        },
        HookDesc {
            sym: c"recvfrom",
            hook_id: Hook::ZygiskRecvfrom,
            replacement: hooked_recvfrom as *mut _,
            store_orig: set_real_recvfrom_ptr,
        },
        HookDesc {
            sym: c"__recvfrom_chk",
            hook_id: Hook::ZygiskRecvfromChk,
            replacement: hooked_recvfrom_chk as *mut _,
            store_orig: set_real_recvfrom_chk_ptr,
        },
    ];

    let mut installed: Vec<*mut core::ffi::c_void> = Vec::with_capacity(table.len());
    for desc in table {
        if !zygisk_hook_enabled(hookmask, desc.hook_id) {
            continue;
        }
        match hook_libc_sym(desc.sym, desc.replacement, desc.store_orig) {
            Ok(stub) => installed.push(stub),
            Err(err) => {
                // Roll back any hooks already installed before this
                // one failed — better to be fully off than to leave
                // the process with a torn hook plan that filters
                // some libc paths but not others.
                for stub in installed.into_iter().rev() {
                    let rc = unsafe { shadowhook::unhook(stub) };
                    if rc != 0 {
                        // Nothing useful to do — we're already on
                        // the error path. Surface it via log so a
                        // bad install at least leaves a trail.
                        error!("install_hooks rollback: shadowhook_unhook rc={rc}");
                    }
                }
                return Err(err);
            }
        }
    }

    Ok(())
}

/// Install a single inline hook on a libc symbol and stash the
/// original trampoline via `store_orig`. Returns the shadowhook stub
/// pointer on success — caller keeps it for the partial-install
/// rollback in `install_hooks`.
fn hook_libc_sym(
    sym: &core::ffi::CStr,
    new_fn: *mut core::ffi::c_void,
    store_orig: fn(*const ()),
) -> Result<*mut core::ffi::c_void, String> {
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
    Ok(stub)
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

/// Return this UID's Zygisk-owned hook mask, or zero if it is not targeted.
/// Protocol §4.3 uses UID as the key on every channel, including Zygisk.
///
/// Called from `pre_app_specialize`, which runs on the zygote side BEFORE the
/// uid drop, so `args.uid` already holds the UID this child will become. One
/// UID covers every process of a multi-process app, so this needs no
/// sub-process name handling (the old package-name path did).
#[inline(never)]
fn target_hookmask(uid: u32) -> u32 {
    match CACHED_CONFIG.get() {
        Some(cfg) => cfg
            .targets
            .iter()
            .find(|target| target.uid == uid)
            .map(|target| target.hookmask)
            .unwrap_or(0),
        None => 0,
    }
}

fn zygisk_hook_enabled(hookmask: u32, hook: Hook) -> bool {
    hookmask & (1u32 << hook as u32) != 0
}

/// Decode a `JString` (as stored in `AppSpecializeArgs::nice_name`) into
/// an owned Rust `String`. Returns None on any failure.
fn read_jstring<'a>(
    env: &JNIEnv<'a>,
    jstr: &zygisk_api::jni::objects::JString<'a>,
) -> Option<String> {
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
