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
//! `detect` is read-only and safe against the live node from the dev sandbox.
//! `ensure_kernel_backend` additionally selects/loads the kernel family for a Native
//! phase, with an explicit module-root opt-in. Delivery remains in `activation`.

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

pub enum SelectedNative {
    Kernel(NativeBackend),
    Kpm(vpnhide_activator::CoreKpm),
    Zygisk(crate::zygisk::ZygiskTarget),
}

impl SelectedNative {
    pub fn family(&self) -> NativeBackend {
        match self {
            Self::Kernel(family) => *family,
            Self::Kpm(_) => NativeBackend::Kpm,
            Self::Zygisk(_) => NativeBackend::Zygisk,
        }
    }
}

/// One central choice. Adopt a compatible live owner first, then choose an
/// eligible kmod artifact before KPM, then the enabled Zygisk recipient. A failed
/// attempt never triggers fallback; definitely unavailable candidates are skipped.
pub fn select_native(
    ctl: &Path,
    modules: Option<&Path>,
    kpm_lock: Option<&Path>,
    zygisk_module: Option<&Path>,
    filesystem_hiding: bool,
) -> Result<SelectedNative, String> {
    if kpm_lock.is_none() && zygisk_module.is_none() {
        return ensure_kernel_backend(ctl, modules, filesystem_hiding).map(SelectedNative::Kernel);
    }
    let kernel = observe_kernel(ctl)?;
    let kpm = kpm_lock
        .map(vpnhide_activator::CoreKpm::connect)
        .transpose()
        .map_err(|e| e.to_string())?
        .flatten();
    if kpm_lock.is_none() && vpnhide_activator::observed_kpm_loaded().map_err(|e| e.to_string())? {
        return Err("live KPM has no enabled core management channel".into());
    }
    let kpm_active = kpm
        .as_ref()
        .map(|client| client.status())
        .transpose()
        .map_err(|e| e.to_string())?
        .flatten()
        .is_some();
    if let Some(kernel) = kernel {
        if kpm_active {
            return Err("conflicting live kernel and KPM backends".into());
        }
        return Ok(SelectedNative::Kernel(kernel));
    }
    if kpm_active {
        return kpm
            .map(SelectedNative::Kpm)
            .ok_or_else(|| "active KPM transport unavailable".into());
    }
    if modules
        .and_then(|root| eligible_artifact(root, NativeBackend::Kmod, "vpnhide_kmod.ko"))
        .is_some()
    {
        return ensure_kernel_backend(ctl, modules, filesystem_hiding).map(SelectedNative::Kernel);
    }
    if let Some(artifact) =
        modules.and_then(|root| eligible_artifact(root, NativeBackend::Kpm, "vpnhide.kpm"))
    {
        if let Some(kpm) = kpm {
            kpm.load_selected(ctl, &artifact, filesystem_hiding)
                .map_err(|e| e.to_string())?;
            return Ok(SelectedNative::Kpm(kpm));
        }
        if zygisk_module.is_none() {
            return Err("KPM artifact is present but its runtime is unavailable".into());
        }
    }
    let module = zygisk_module.ok_or("no eligible native artifact")?;
    crate::zygisk::ZygiskTarget::open(module).map(SelectedNative::Zygisk)
}

fn eligible_artifact(
    root: &Path,
    backend: NativeBackend,
    name: &str,
) -> Option<std::path::PathBuf> {
    let module = root.join(module_dir(backend));
    let artifact = module.join(name);
    (!module.join("disable").exists()
        && !module.join("remove").exists()
        && module.join("module.prop").is_file()
        && artifact.is_file())
    .then_some(artifact)
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

/// The kernel slice of the central policy: a live owner wins over installed
/// artifacts. Built-in needs no companion. Only definite absence permits loading
/// an enabled kmod artifact. The shared selector composes KPM/Zygisk above it.
/// Errors never mean absence and never trigger fallback after a load attempt.
pub fn ensure_kernel_backend(
    ctl: &Path,
    modules: Option<&Path>,
    filesystem_hiding: bool,
) -> Result<NativeBackend, String> {
    ensure_kernel_backend_with(ctl, modules, |artifact| {
        vpnhide_activator::load_kernel_module(ctl, artifact, filesystem_hiding)
            .map_err(|e| e.to_string())
    })
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
    if vpnhide_activator::observed_kpm_loaded().map_err(|e| e.to_string())? {
        return Err("KPM became active before Zygisk publication".into());
    }
    Ok(())
}

fn ensure_kernel_backend_with(
    ctl: &Path,
    modules: Option<&Path>,
    load: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<NativeBackend, String> {
    if let Some(backend) = observe_kernel(ctl)? {
        return Ok(backend);
    }
    let module = modules
        .ok_or("kernel backend absent; loading is not enabled")?
        .join("vpnhide_kmod");
    if module.join("disable").exists() || module.join("remove").exists() {
        return Err("kmod artifact is disabled or scheduled for removal".into());
    }
    let artifact = module.join("vpnhide_kmod.ko");
    if !module.join("module.prop").is_file() || !artifact.is_file() {
        return Err("no usable kmod artifact".into());
    }
    load(&artifact)?;
    match observe_kernel(ctl)? {
        Some(NativeBackend::Kmod) => Ok(NativeBackend::Kmod),
        _ => Err("kmod load did not produce the selected runtime identity".into()),
    }
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::fs;

    fn install_kmod(root: &Path) -> std::path::PathBuf {
        let module = root.join("vpnhide_kmod");
        fs::create_dir_all(&module).unwrap();
        fs::write(module.join("module.prop"), "id=vpnhide_kmod\n").unwrap();
        fs::write(module.join("vpnhide_kmod.ko"), "artifact").unwrap();
        module
    }

    #[test]
    fn live_owner_wins_without_a_bridge_or_module_directory() {
        let dir = tempfile::tempdir().unwrap();
        for (id, expected) in [("4", NativeBackend::Builtin), ("0", NativeBackend::Kmod)] {
            let ctl = write_status(dir.path(), id);
            let selected =
                ensure_kernel_backend_with(&ctl, None, |_| panic!("live owner must not load"));
            assert_eq!(selected.unwrap(), expected);
        }
    }

    #[test]
    fn unreadable_invalid_and_foreign_nodes_never_invoke_loader() {
        let dir = tempfile::tempdir().unwrap();
        install_kmod(dir.path());
        let ctl = dir.path().join("ctl");
        for text in [
            "garbage",
            "vpnhide 2 status\nbackend 0x0\nkver 0x0\nhooks 0x0\nerror 0x0\n",
            "vpnhide 1 status\nbackend 0x99\nkver 0x0\nhooks 0x0\nerror 0x0\n",
        ] {
            fs::write(&ctl, text).unwrap();
            assert!(
                ensure_kernel_backend_with(&ctl, Some(dir.path()), |_| panic!("must not load"))
                    .is_err()
            );
        }
        fs::remove_file(&ctl).unwrap();
        fs::create_dir(&ctl).unwrap();
        assert!(
            ensure_kernel_backend_with(&ctl, Some(dir.path()), |_| panic!("must not load"))
                .is_err()
        );
    }

    #[test]
    fn loading_requires_explicit_opt_in_and_usable_artifact() {
        let dir = tempfile::tempdir().unwrap();
        let ctl = dir.path().join("ctl");
        assert!(ensure_kernel_backend_with(&ctl, None, |_| panic!("must not load")).is_err());
        assert!(
            ensure_kernel_backend_with(&ctl, Some(dir.path()), |_| panic!("must not load"))
                .is_err()
        );
        let module = install_kmod(dir.path());
        for marker in ["disable", "remove"] {
            fs::write(module.join(marker), "").unwrap();
            assert!(
                ensure_kernel_backend_with(&ctl, Some(dir.path()), |_| panic!("must not load"))
                    .is_err()
            );
            fs::remove_file(module.join(marker)).unwrap();
        }
        fs::remove_file(module.join("vpnhide_kmod.ko")).unwrap();
        assert!(
            ensure_kernel_backend_with(&ctl, Some(dir.path()), |_| panic!("must not load"))
                .is_err()
        );
    }

    #[test]
    fn load_is_called_once_and_identity_is_verified_afterwards() {
        let dir = tempfile::tempdir().unwrap();
        let module = install_kmod(dir.path());
        let ctl = dir.path().join("vpnhide_ctl");
        let calls = Cell::new(0);
        let selected = ensure_kernel_backend_with(&ctl, Some(dir.path()), |artifact| {
            assert_eq!(artifact, module.join("vpnhide_kmod.ko"));
            calls.set(calls.get() + 1);
            write_status(dir.path(), "0");
            Ok(())
        });
        assert_eq!(selected.unwrap(), NativeBackend::Kmod);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn failed_load_or_wrong_post_load_owner_never_falls_back() {
        let dir = tempfile::tempdir().unwrap();
        install_kmod(dir.path());
        let ctl = dir.path().join("vpnhide_ctl");
        assert!(
            ensure_kernel_backend_with(&ctl, Some(dir.path()), |_| Err("load failed".into()))
                .is_err()
        );
        assert!(ensure_kernel_backend_with(&ctl, Some(dir.path()), |_| Ok(())).is_err());
        assert!(
            ensure_kernel_backend_with(&ctl, Some(dir.path()), |_| {
                write_status(dir.path(), "4");
                Ok(())
            })
            .is_err()
        );
    }

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
