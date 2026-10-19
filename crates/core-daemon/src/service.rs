//! `vpnhide-core service`: the module's whole boot entry. The root manager's
//! `service.sh` is a single `exec` of this; every path, flag and lifecycle step the
//! script used to set up (persist dir, phase markers, production env, backgrounding,
//! log redirection) lives here, from compiled-in constants plus the binary's own
//! location (`MODDIR`).

use std::ffi::CString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Core's private state dir (journal, markers, boot status, log).
pub const PERSIST_DIR: &str = "/data/adb/vpnhide/core";
/// The root managers' module root (Magisk, KernelSU, APatch).
pub const MODULES_ROOT: &str = "/data/adb/modules";
/// The app's device-encrypted data dir; core binds its socket inside it.
pub const APP_DATA_DIR: &str = "/data/user_de/0/dev.okhsunrog.vpnhide";
pub const CONFIG_PATH: &str = "/data/system/vpnhide_config.json";
pub const KMOD_CTL: &str = "/proc/vpnhide_ctl";
pub const KPM_LOCK: &str = "/data/adb/vpnhide_kpm/ctl.lock";

/// The modules root this binary was installed under (`MODDIR/..`), falling back to the
/// compiled-in root when the binary is not inside a `modules` dir (e.g. a manual run).
pub fn modules_root(exe: &Path) -> PathBuf {
    exe.parent()
        .and_then(Path::parent)
        .filter(|root| root.file_name().is_some_and(|n| n == "modules"))
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(MODULES_ROOT))
}

/// The production configuration, as the variables the daemon's lanes read. Enabled only
/// by this entry point; host/dev runs leave every writer off.
pub fn production_env(modules: &Path) -> Vec<(&'static str, String)> {
    let persist = Path::new(PERSIST_DIR);
    let p = |rel: &str| persist.join(rel).display().to_string();
    vec![
        ("VPNHIDE_CONFIG_PATH", CONFIG_PATH.into()),
        ("VPNHIDE_JOURNAL_PATH", p("operations.json")),
        ("VPNHIDE_CORE_OWNER_DIR", PERSIST_DIR.into()),
        ("VPNHIDE_CORE_ADOPT", "1".into()),
        ("VPNHIDE_BOOT_APPLY", "1".into()),
        ("VPNHIDE_KMOD_CTL", KMOD_CTL.into()),
        // Native backends: core is their only loader and writer (backend modules ship
        // artifacts only).
        ("VPNHIDE_KERNEL_MODULES", modules.display().to_string()),
        ("VPNHIDE_KPM_LOCK", KPM_LOCK.into()),
        (
            "VPNHIDE_ZYGISK_MODULE",
            modules.join("vpnhide_zygisk").display().to_string(),
        ),
        ("VPNHIDE_PORTS", "1".into()),
        ("VPNHIDE_PORTS_OWNER_MARKER", p("ports_owned")),
        // App IPC: socket in the app's own data dir, peer-checked (SO_PEERCRED).
        ("VPNHIDE_APP_DATA_DIR", APP_DATA_DIR.into()),
        // Typed root work the app may ask for (diagnostics probes, fixed effects).
        ("VPNHIDE_ROOT_EFFECTS", "1".into()),
    ]
}

/// Prepare the persist dir and phase markers, export the production profile into this
/// process, then detach from the manager's script runner with stdout/stderr appended to
/// `core.log`. Returns in the detached child only; the parent exits 0 so the manager's
/// late-start step completes at once.
///
/// Must run before any thread exists (the environment is written and the process forks).
pub fn enter() {
    let persist = Path::new(PERSIST_DIR);
    let _ = fs::create_dir_all(persist);
    let _ = fs::set_permissions(persist, fs::Permissions::from_mode(0o700));
    let _ = fs::write(persist.join("boot_phase"), "phase=service\n");
    // Diagnostics-only adoption marker (core is the sole writer by construction).
    let owned = persist.join("config_owned");
    if !owned.exists() {
        let _ = fs::write(&owned, "owned-by-core\n");
    }
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from(MODULES_ROOT));
    for (key, value) in production_env(&modules_root(&exe)) {
        // SAFETY: called from `main` before the async runtime or any thread starts, so
        // no other thread can read the environment concurrently.
        unsafe { std::env::set_var(key, value) };
    }
    detach(&persist.join("core.log"));
    let started = now_utc();
    eprintln!("phase=starting {started}");
}

fn detach(log: &Path) {
    // SAFETY: single-threaded here (see `enter`); fork duplicates only this thread.
    match unsafe { libc::fork() } {
        -1 => {
            eprintln!("core: fork failed: {}", std::io::Error::last_os_error());
            return;
        }
        0 => {}
        _ => std::process::exit(0),
    }
    // SAFETY: setsid in the fresh child: leave the manager's session/process group.
    unsafe { libc::setsid() };
    let null = CString::new("/dev/null").expect("static path");
    // SAFETY: plain open/dup2 on fds this process owns.
    unsafe {
        let fd = libc::open(null.as_ptr(), libc::O_RDONLY);
        if fd >= 0 {
            libc::dup2(fd, 0);
            libc::close(fd);
        }
    }
    if let Ok(file) = OpenOptions::new().create(true).append(true).open(log) {
        let _ = fs::set_permissions(log, fs::Permissions::from_mode(0o600));
        // SAFETY: dup2 onto stdout/stderr from a valid open fd.
        unsafe {
            libc::dup2(file.as_raw_fd(), 1);
            libc::dup2(file.as_raw_fd(), 2);
        }
    }
}

/// Append the exit marker the old script wrote after the daemon returned.
pub fn exited(code: i32) {
    eprintln!("phase=exited rc={code} {}", now_utc());
    let _ = std::io::stderr().flush();
}

fn now_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix={secs}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modules_root_comes_from_moddir_or_the_compiled_in_root() {
        assert_eq!(
            modules_root(Path::new("/data/adb/modules/vpnhide_core/vpnhide-core")),
            PathBuf::from("/data/adb/modules")
        );
        assert_eq!(
            modules_root(Path::new("/tmp/x/vpnhide-core")),
            PathBuf::from(MODULES_ROOT)
        );
    }

    #[test]
    fn production_env_enables_every_writer_with_fixed_paths() {
        let env = production_env(Path::new("/data/adb/modules"));
        let get = |k: &str| {
            env.iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("VPNHIDE_BOOT_APPLY"), Some("1"));
        assert_eq!(get("VPNHIDE_CORE_ADOPT"), Some("1"));
        assert_eq!(get("VPNHIDE_ROOT_EFFECTS"), Some("1"));
        assert_eq!(get("VPNHIDE_KERNEL_MODULES"), Some("/data/adb/modules"));
        assert_eq!(get("VPNHIDE_KPM_LOCK"), Some(KPM_LOCK));
        assert_eq!(
            get("VPNHIDE_ZYGISK_MODULE"),
            Some("/data/adb/modules/vpnhide_zygisk")
        );
        assert_eq!(get("VPNHIDE_APP_DATA_DIR"), Some(APP_DATA_DIR));
        assert_eq!(
            get("VPNHIDE_PORTS_OWNER_MARKER"),
            Some("/data/adb/vpnhide/core/ports_owned")
        );
        assert!(env.iter().all(|(k, _)| *k != "VPNHIDE_CORE_ALLOW_UIDS"));
    }
}
