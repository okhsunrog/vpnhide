//! Central native-backend detection (core-2.0 §4). Today each activator probes runtime
//! identity and self-selects; core owns all state, so it makes that **one** observation and
//! everything downstream (which family to project for, what the app shows) reads it.
//!
//! `active` comes from **runtime identity**, not a module directory merely existing (§2): the
//! kernel control node reports the live backend id. `installed` is the weaker "module present
//! on disk" signal. This slice covers the kernel families (Kmod/Builtin, one shared control
//! node); KPM (its own `kpatch` liveness) and Zygisk (runtime + heartbeat) `active` signals
//! land next — they still show under `installed`.
//!
//! Read-only: detection never writes, so it is safe against the live node even from the dev
//! sandbox core (the delivery path is what is isolated, not observation).

use std::path::Path;

use vpnhide_activator::{BUILTIN_BACKEND_ID, KMOD_BACKEND_ID};
use vpnhide_core_rpc::{NativeBackend, NativeBackendStatus};

use crate::activation;

/// The production module root; each backend installs a directory (or artifact) under it.
pub const MODULES_ROOT: &str = "/data/adb/modules";

/// The module directory basename for each backend (its "installed" evidence).
fn module_dir(backend: NativeBackend) -> &'static str {
    match backend {
        NativeBackend::Kmod => "vpnhide_kmod",
        NativeBackend::Builtin => "vpnhide_builtin",
        NativeBackend::Kpm => "vpnhide_kpm",
        NativeBackend::Zygisk => "vpnhide_zygisk",
    }
}

/// Map a kernel control-node backend id to a family. `None` for a foreign/unknown owner —
/// which must never be treated as one of ours (§2/§9: refuse, do not misconfigure).
pub fn kernel_backend_from_id(id: u32) -> Option<NativeBackend> {
    if id == BUILTIN_BACKEND_ID {
        Some(NativeBackend::Builtin)
    } else if id == KMOD_BACKEND_ID {
        Some(NativeBackend::Kmod)
    } else {
        None
    }
}

/// Observe the native backend: the live kernel family from `ctl_path` (read-only) and every
/// backend whose module is present under `modules_root`.
pub fn detect(ctl_path: &Path, modules_root: &Path) -> NativeBackendStatus {
    // Active: the live kernel backend id, if the node is present and reports one of ours.
    let active = activation::read_status(ctl_path)
        .ok()
        .and_then(|status| kernel_backend_from_id(status.backend));

    // Installed: module directories present. In ConfigPhase/enum order for a stable list.
    let installed = [
        NativeBackend::Kmod,
        NativeBackend::Builtin,
        NativeBackend::Kpm,
        NativeBackend::Zygisk,
    ]
    .into_iter()
    .filter(|&b| modules_root.join(module_dir(b)).exists())
    .collect();

    NativeBackendStatus { active, installed }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_status(dir: &Path, backend_hex: &str) -> std::path::PathBuf {
        let path = dir.join("vpnhide_ctl");
        fs::write(
            &path,
            format!(
                "vpnhide 1 status\nbackend 0x{backend_hex}\nkver 0x60100\nhooks 0x3ff\nerror 0x0\n"
            ),
        )
        .unwrap();
        path
    }

    #[test]
    fn detects_the_live_builtin_backend_and_installed_modules() {
        let dir = tempfile::tempdir().unwrap();
        let ctl = write_status(dir.path(), "4"); // 0x4 == Builtin
        let modules = dir.path().join("modules");
        fs::create_dir_all(modules.join("vpnhide_builtin")).unwrap();
        fs::create_dir_all(modules.join("vpnhide_zygisk")).unwrap();

        let status = detect(&ctl, &modules);
        assert_eq!(status.active, Some(NativeBackend::Builtin));
        assert_eq!(status.installed, vec![NativeBackend::Builtin, NativeBackend::Zygisk]);
    }

    #[test]
    fn absent_node_is_no_active_backend() {
        let dir = tempfile::tempdir().unwrap();
        let status = detect(&dir.path().join("nope"), &dir.path().join("modules"));
        assert_eq!(status.active, None);
        assert!(status.installed.is_empty());
    }

    #[test]
    fn foreign_owner_is_not_one_of_ours() {
        let dir = tempfile::tempdir().unwrap();
        let ctl = write_status(dir.path(), "99"); // unknown backend id
        let status = detect(&ctl, &dir.path().join("modules"));
        assert_eq!(status.active, None, "a foreign live backend is never mapped to ours");
    }
}
