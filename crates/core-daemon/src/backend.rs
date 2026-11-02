//! Central native-backend observation. Core owns boot selection and delivery;
//! these read-only runtime observations qualify liveness without replacing those facts.
//!
//! `active` comes from **runtime identity**, not a module directory merely existing: the
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
use vpnhide_core_rpc::{
    NativeBackend, NativeBackendStatus, NativeRuntimeStatus, ObservationFreshness,
};

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
        let observed = (|| -> Result<Option<NativeRuntimeStatus>, String> {
            if detected.observed_freshness != ObservationFreshness::Current {
                return Err(detected
                    .observation_error
                    .clone()
                    .unwrap_or_else(|| "kernel observation unavailable".into()));
            }
            let kernel = detected.runtime_status.clone();
            let client = vpnhide_activator::CoreKpm::connect(lock).map_err(|e| e.to_string())?;
            let kpm = client
                .as_ref()
                .map(|client| client.status())
                .transpose()
                .map_err(|e| e.to_string())?
                .flatten();
            combine_runtime(&mut detected, kernel, kpm)
        })();
        set_runtime_observation(&mut detected, observed);
    }
    detected
}

fn combine_runtime(
    detected: &mut NativeBackendStatus,
    kernel: Option<NativeRuntimeStatus>,
    kpm: Option<vpnhide_protocol::Status>,
) -> Result<Option<NativeRuntimeStatus>, String> {
    if let Some(status) = &kpm
        && status.backend != vpnhide_protocol::hook_ids::Backend::Kpm as u32
    {
        return Err("unexpected KPM runtime identity".into());
    }
    match (kernel, kpm) {
        (Some(_), Some(_)) => {
            detected.runtime_conflict = true;
            Err("conflicting kernel and KPM identities".into())
        }
        (None, Some(status)) => Ok(Some(runtime_status(NativeBackend::Kpm, status))),
        (kernel, None) => Ok(kernel),
    }
}

pub fn kernel_modules_from_env() -> Option<std::path::PathBuf> {
    std::env::var_os(KERNEL_MODULES_ENV)
        .filter(|p| !p.is_empty())
        .map(Into::into)
}

fn runtime_status(backend: NativeBackend, status: vpnhide_protocol::Status) -> NativeRuntimeStatus {
    NativeRuntimeStatus {
        backend,
        kernel_version: status.kver,
        installed_hooks: status.hooks,
        status_error: status.error,
    }
}

fn observe_kernel_status(ctl: &Path) -> Result<Option<NativeRuntimeStatus>, String> {
    let text = match std::fs::read(ctl) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let status = vpnhide_protocol::parse_status_fields(&text)
        .and_then(|fields| fields.complete())
        .ok_or("invalid or incomplete kernel status")?;
    let backend = kernel_backend_from_id(status.backend)
        .ok_or_else(|| format!("unknown kernel backend 0x{:x}", status.backend))?;
    Ok(Some(runtime_status(backend, status)))
}
fn observe_kernel(ctl: &Path) -> Result<Option<NativeBackend>, String> {
    observe_kernel_status(ctl).map(|status| status.map(|status| status.backend))
}
fn set_runtime_observation(
    target: &mut NativeBackendStatus,
    observation: Result<Option<NativeRuntimeStatus>, String>,
) {
    match observation {
        Ok(status) => {
            target.observed_freshness = ObservationFreshness::Current;
            target.active = status.as_ref().map(|status| status.backend);
            target.runtime_status = status;
            target.observation_error = None;
        }
        Err(error) => {
            target.observed_freshness = ObservationFreshness::Unknown;
            target.active = None;
            target.runtime_status = None;
            target.observation_error = Some(error);
        }
    }
}

/// Projection can wait for PackageManager. Recheck root facts immediately before
/// publishing Zygisk so a newly live kernel/KPM owner prevents conflicting policy.
pub fn check_zygisk_dispatch(ctl: &Path) -> Result<(), String> {
    if observe_kernel(ctl)?.is_some() {
        return Err("kernel backend became active before Zygisk publication".into());
    }
    match vpnhide_activator::observed_kpm_loaded() {
        Ok(false) => {}
        Ok(true) => return Err("KPM became active before Zygisk publication".into()),
        Err(error) => return Err(format!("KPM ownership could not be rechecked: {error}")),
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
    // One complete status read owns both identity and hook-installation telemetry.
    let observed = observe_kernel_status(ctl_path);
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

    let mut status = NativeBackendStatus {
        installed,
        ..Default::default()
    };
    set_runtime_observation(&mut status, observed);
    status
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
    fn confirmed_competing_owners_are_not_an_unavailable_or_successful_runtime() {
        let mut detected = NativeBackendStatus::default();
        let kernel = NativeRuntimeStatus {
            backend: NativeBackend::Builtin,
            kernel_version: 1,
            installed_hooks: 0x3ff,
            status_error: 0,
        };
        let kpm = vpnhide_protocol::Status {
            backend: 1,
            kver: 1,
            hooks: 0x3ff,
            error: 0,
        };
        let observation = combine_runtime(&mut detected, Some(kernel.clone()), Some(kpm));
        set_runtime_observation(&mut detected, observation);
        assert!(detected.runtime_conflict);
        assert_eq!(detected.active, None);
        assert_eq!(detected.runtime_status, None);
        let mut snapshot = vpnhide_core_rpc::StateSnapshot {
            backend: detected,
            ..Default::default()
        };
        crate::native_status::refresh(&mut snapshot);
        assert_eq!(
            snapshot.native_view.setup,
            vpnhide_core_rpc::NativeSetupHealth::RuntimeConflict
        );
        let mut detected = NativeBackendStatus::default();
        let observation = combine_runtime(&mut detected, Some(kernel.clone()), None);
        set_runtime_observation(&mut detected, observation);
        assert!(!detected.runtime_conflict);
        assert_eq!(detected.runtime_status, Some(kernel));
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
