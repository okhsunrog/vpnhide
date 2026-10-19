//! Central native-backend detection (core-2.0 §4). Today each activator probes runtime
//! identity and self-selects; core owns all state, so it makes that **one** observation and
//! everything downstream (which family to project for, what the app shows) reads it.
//!
//! `active` comes from **runtime identity**, not a module directory merely existing (§2): the
//! kernel control node reports the live backend id. `installed` is the weaker "module present
//! on disk" signal. This slice covers the kernel families (Kmod/Builtin, one shared control
//! node) and KPM (authenticated runtime enumeration plus telemetry identity).
//! KPM management is explicitly enabled with its lock path. Zygisk file delivery
//! is separately enabled; selection/publication never invent runtime liveness.
//!
//! `detect` is read-only and safe against the live node from the dev sandbox. The
//! once-per-boot selection and loading live in [`crate::native_boot`].

use std::path::Path;

use vpnhide_activator::{BUILTIN_BACKEND_ID, KMOD_BACKEND_ID};
use vpnhide_core_rpc::{NativeBackend, NativeBackendStatus};

use crate::activation;

/// The production module root; each backend installs a directory (or artifact) under it.
pub const MODULES_ROOT: &str = "/data/adb/modules";

/// Explicit opt-in to kmod loading while legacy boot ownership is being migrated.
/// Unset means core may manage an existing kernel node, but never calls insmod.
pub const KERNEL_MODULES_ENV: &str = "VPNHIDE_KERNEL_MODULES";
/// Opt-in to KPM management; production must use the same lock as legacy readers.
/// No default until the sole-writer cutover is implemented.
pub const KPM_LOCK_ENV: &str = "VPNHIDE_KPM_LOCK";

pub fn kpm_lock_from_env() -> Option<std::path::PathBuf> {
    std::env::var_os(KPM_LOCK_ENV)
        .filter(|p| !p.is_empty())
        .map(Into::into)
}

/// Include KPM runtime evidence only when its management channel is explicitly
/// enabled. Failure never invents an active backend from an installed artifact.
pub fn detect_managed(ctl: &Path, modules: &Path, kpm_lock: Option<&Path>) -> NativeBackendStatus {
    let mut detected = detect(ctl, modules);
    if let Some(lock) = kpm_lock {
        let observed = (|| -> Result<Option<NativeBackend>, String> {
            let kernel = observe_kernel(ctl)?;
            let client = vpnhide_activator::CoreKpm::connect(lock).map_err(|e| e.to_string())?;
            let kpm_active = client
                .as_ref()
                .map(|c| c.status())
                .transpose()
                .map_err(|e| e.to_string())?
                .flatten()
                .is_some();
            match (kernel, kpm_active) {
                (Some(_), true) => Err("conflicting kernel and KPM identities".into()),
                (None, true) => Ok(Some(NativeBackend::Kpm)),
                (kernel, false) => Ok(kernel),
            }
        })();
        detected.active = observed.ok().flatten();
    }
    detected
}

pub fn kernel_modules_from_env() -> Option<std::path::PathBuf> {
    std::env::var_os(KERNEL_MODULES_ENV)
        .filter(|p| !p.is_empty())
        .map(Into::into)
}

fn observe_kernel(ctl: &Path) -> Result<Option<NativeBackend>, String> {
    let id = vpnhide_activator::observed_kernel_backend(ctl).map_err(|e| e.to_string())?;
    id.map(|id| {
        kernel_backend_from_id(id).ok_or_else(|| format!("unknown kernel backend 0x{id:x}"))
    })
    .transpose()
}

/// Projection can wait for PackageManager. Recheck root facts immediately before
/// publishing Zygisk so a newly live kernel/KPM owner prevents conflicting policy.
pub fn check_zygisk_dispatch(ctl: &Path) -> Result<(), String> {
    if observe_kernel(ctl)?.is_some() {
        return Err("kernel backend became active before Zygisk publication".into());
    }
    // Only a definitely live KPM blocks: the boot decision already settled
    // KernelPatch, and an unauthenticated runtime must not block Zygisk.
    if matches!(vpnhide_activator::observed_kpm_loaded(), Ok(true)) {
        return Err("KPM became active before Zygisk publication".into());
    }
    Ok(())
}

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

    NativeBackendStatus {
        active,
        installed,
        selected: None,
        applied: None,
        boot: None,
    }
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
        assert_eq!(
            status.installed,
            vec![NativeBackend::Builtin, NativeBackend::Zygisk]
        );
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
        assert_eq!(
            status.active, None,
            "a foreign live backend is never mapped to ours"
        );
    }
}
