//! Backend activation (core-2.0 §4/§9.1, step D): core absorbs the activator and applies
//! the native control-v2 wire to the kernel backend itself, instead of shelling out to the
//! per-backend activator binaries. It **links** the activator's projection and delivery
//! (additive pub surface on `vpnhide_activator`) rather than reimplementing hook masks.
//!
//! Scope: kmod/built-in (the shared `/proc/vpnhide_ctl` node), plus direct KPM
//! control through the selected authenticated transport and explicit lock path.
//! Zygisk receives its existing policy file through a verified module directory;
//! injection and process-local measurements retain their existing owners.
//!
//! # Isolation
//!
//! Delivery targets an **overridable** control-node path ([`kmod_ctl_path_from_env`], env
//! `VPNHIDE_KMOD_CTL`). It has **no default**: with the env unset there is no native write
//! at all, so the dev sandbox core cannot touch the real `/proc/vpnhide_ctl` (a write there
//! replaces the live backend's entire state). Production sets it explicitly; the dev core
//! points it at an isolated node only when exercising the phase. Delivery also refuses a
//! node that does not report the expected backend id, so a mistargeted write is rejected.

use std::path::{Path, PathBuf};

use vpnhide_activator::{
    BUILTIN_BACKEND_ID, KMOD_BACKEND_ID, NativeCapacityExceeded, NativeHookFamily, PackageUidMap,
    deliver_kernel_config, parse_canonical, project_native_for_family,
    project_native_with_resolver_for_family,
};
use vpnhide_core_rpc::{
    CanonicalConfig, NativeBackend, NativeCapacityPackage, NativeCapacityReport,
};
use vpnhide_protocol::{Status, parse_status_fields};

use crate::config_file;

/// Deliver `wire` to this boot's selected backend. The outcome is the phase outcome; the
/// error text (when any) is what the published status reports.
pub fn deliver_to_selected(
    env: &crate::native_boot::NativeEnv,
    backend: NativeBackend,
    wire: &str,
) -> (vpnhide_core_rpc::PhaseOutcome, Result<(), String>) {
    use vpnhide_core_rpc::PhaseOutcome;
    let failed = |e: String| (PhaseOutcome::FailedKnown, Err(e));
    match backend {
        NativeBackend::Kmod | NativeBackend::Builtin => {
            match deliver_native(&env.ctl, backend, wire) {
                Ok(()) => (PhaseOutcome::Confirmed, Ok(())),
                Err(e) => failed(e),
            }
        }
        NativeBackend::Kpm => {
            let Some(lock) = env.kpm_lock.as_deref() else {
                return failed("KPM management is not enabled".into());
            };
            // Selection is fixed for the boot; a kernel backend that appeared since
            // (manual insmod) would make two live hook sets, so refuse rather than add one.
            if let Ok(status) = read_status(&env.ctl) {
                return failed(format!(
                    "kernel backend 0x{:x} appeared after this boot's KPM selection",
                    status.backend
                ));
            }
            let client = match vpnhide_activator::CoreKpm::connect(lock) {
                Ok(Some(client)) => client,
                Ok(None) => return failed("KernelPatch runtime is gone".into()),
                Err(e) => return failed(e.to_string()),
            };
            match client.configure_once(&env.ctl, wire) {
                Ok(()) => (PhaseOutcome::Confirmed, Ok(())),
                Err(vpnhide_activator::KpmConfigError::BeforeDispatch(e)) => failed(e),
                Err(vpnhide_activator::KpmConfigError::Unknown(e)) => {
                    (PhaseOutcome::Unknown, Err(format!("outcome unknown: {e}")))
                }
            }
        }
        NativeBackend::Zygisk => {
            let Some(dir) = env.module_dir(NativeBackend::Zygisk) else {
                return failed("no Zygisk module directory".into());
            };
            let target = match crate::zygisk::ZygiskTarget::open(&dir) {
                Ok(target) => target,
                Err(e) => return failed(e),
            };
            if let Err(e) = crate::backend::check_zygisk_dispatch(&env.ctl) {
                return failed(e);
            }
            match target.publish(wire) {
                Ok(()) => (PhaseOutcome::Confirmed, Ok(())),
                Err(crate::zygisk::PublishError::BeforeWrite(e)) => failed(e),
                Err(crate::zygisk::PublishError::Unknown(e)) => {
                    (PhaseOutcome::Unknown, Err(format!("outcome unknown: {e}")))
                }
            }
        }
    }
}

/// The activator's hook-mask family for a backend (projection depends on it).
fn family_of(backend: NativeBackend) -> NativeHookFamily {
    match backend {
        NativeBackend::Kmod => NativeHookFamily::Kmod,
        NativeBackend::Builtin => NativeHookFamily::Builtin,
        NativeBackend::Kpm => NativeHookFamily::Kpm,
        NativeBackend::Zygisk => NativeHookFamily::Zygisk,
    }
}

/// The kernel control-node backend id for a backend, or `None` for families that do not
/// deliver through `/proc/vpnhide_ctl` (KPM has its own CLI, Zygisk uses `targets.txt`).
fn kernel_backend_id(backend: NativeBackend) -> Option<u32> {
    match backend {
        NativeBackend::Builtin => Some(BUILTIN_BACKEND_ID),
        NativeBackend::Kmod => Some(KMOD_BACKEND_ID),
        NativeBackend::Kpm | NativeBackend::Zygisk => None,
    }
}

/// Env override for the kernel control node. **No default** — unset means "do not write".
pub const KMOD_CTL_ENV: &str = "VPNHIDE_KMOD_CTL";

/// The control-node path to deliver to, or `None` when unset (no native write). Safe by
/// default: the dev sandbox core leaves this unset and never reaches the real node.
pub fn kmod_ctl_path_from_env() -> Option<PathBuf> {
    match std::env::var(KMOD_CTL_ENV) {
        Ok(p) if !p.is_empty() => Some(PathBuf::from(p)),
        _ => None,
    }
}

/// Why a config could not be projected into a backend's wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionError {
    /// The target set does not fit the backend. Refused whole — never truncated.
    Capacity(NativeCapacityReport),
    /// The config or the package lookup failed.
    Failed(String),
}

impl std::fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Capacity(r) => write!(
                f,
                "native target capacity exceeded: {} uids, capacity {}",
                r.total, r.capacity
            ),
            Self::Failed(e) => f.write_str(e),
        }
    }
}

/// The wire form of an activator capacity refusal.
pub fn capacity_report(exceeded: &NativeCapacityExceeded) -> NativeCapacityReport {
    let clamp = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    NativeCapacityReport {
        total: clamp(exceeded.total),
        capacity: clamp(exceeded.capacity),
        packages: exceeded
            .packages
            .iter()
            .map(|p| NativeCapacityPackage {
                package: p.package.clone(),
                uids: p.uids.clone(),
            })
            .collect(),
    }
}

/// Project a config into a backend family's control-v2 wire, resolving package uids from the
/// live PackageManager. Device-only (needs `cmd package`); host tests use the resolver variant.
pub fn project_native_wire(
    config: &CanonicalConfig,
    backend: NativeBackend,
) -> Result<String, ProjectionError> {
    let json = config_file::to_json_string(config);
    project_native_for_family(&json, family_of(backend)).map_err(|e| {
        match e.downcast_ref::<NativeCapacityExceeded>() {
            Some(exceeded) => ProjectionError::Capacity(capacity_report(exceeded)),
            None => ProjectionError::Failed(e.to_string()),
        }
    })
}

/// Project with an explicit uid resolver — deterministic and PackageManager-free, for tests.
pub fn project_native_wire_with_resolver(
    config: &CanonicalConfig,
    backend: NativeBackend,
    resolver: &PackageUidMap,
) -> Result<String, ProjectionError> {
    let json = config_file::to_json_string(config);
    let cfg = parse_canonical(&json).map_err(|e| ProjectionError::Failed(e.to_string()))?;
    project_native_with_resolver_for_family(&cfg, resolver, family_of(backend))
        .map_err(|e| ProjectionError::Capacity(capacity_report(&e)))
}

/// Deliver a control-v2 `wire` to the kernel control node at `path` for `backend`, refusing
/// unless the node reports that backend (never configure a foreign owner). Errors for a
/// backend that does not deliver through the kernel node (KPM/Zygisk).
pub fn deliver_native(path: &Path, backend: NativeBackend, wire: &str) -> Result<(), String> {
    let expected = kernel_backend_id(backend)
        .ok_or_else(|| format!("{backend:?} does not deliver via the kernel control node"))?;
    deliver_kernel_config(path, expected, wire).map_err(|e| e.to_string())
}

/// Read the backend's telemetry-v1 **status** (backend id, kernel version, installed-hook
/// mask, error code) from `path`. Read-only — safe against the live node. Core uses it to
/// confirm liveness/errors; parsing is the same as the app's dashboard read.
pub fn read_status(path: &Path) -> Result<Status, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    parse_status_fields(text.as_bytes())
        .and_then(|f| f.complete())
        .ok_or_else(|| "invalid or incomplete telemetry status".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpnhide_core_rpc::{AppConfig, NativeRole};

    #[test]
    fn env_override_is_none_by_default_and_some_when_set() {
        // SAFETY: env access is process-global; this test owns the var name.
        unsafe { std::env::remove_var(KMOD_CTL_ENV) };
        assert!(
            kmod_ctl_path_from_env().is_none(),
            "unset -> no native write"
        );
        unsafe { std::env::set_var(KMOD_CTL_ENV, "/data/local/tmp/vpnhide-dev/fake_ctl") };
        assert_eq!(
            kmod_ctl_path_from_env(),
            Some(PathBuf::from("/data/local/tmp/vpnhide-dev/fake_ctl")),
        );
        unsafe { std::env::remove_var(KMOD_CTL_ENV) };
    }

    #[test]
    fn projects_a_native_target_deterministically() {
        // A config with one native-enabled app, resolved to a known uid, projects to a
        // control-v2 wire that names that uid — proving core drives the real projection.
        let resolver = vpnhide_activator::parse_pm_packages("package:com.example uid:10234\n");
        let mut config = CanonicalConfig::default();
        config.apps.insert(
            "com.example".into(),
            AppConfig {
                native: NativeRole {
                    enabled: true,
                    kernel: None,
                    zygisk: None,
                },
                ..Default::default()
            },
        );
        let wire = project_native_wire_with_resolver(&config, NativeBackend::Builtin, &resolver)
            .expect("projects");
        // The wire is a control-v2 config snapshot; the resolved uid appears as bare hex
        // (10234 == 0x27fa), per the protocol's control-side numeric rule.
        assert!(
            wire.contains("27fa"),
            "projected wire must target the resolved uid: {wire}"
        );
    }

    #[test]
    fn empty_config_projects_without_targets() {
        let resolver = PackageUidMap::default();
        let wire = project_native_wire_with_resolver(
            &CanonicalConfig::default(),
            NativeBackend::Builtin,
            &resolver,
        )
        .expect("projects");
        assert!(
            wire.contains("config"),
            "still a valid config snapshot: {wire}"
        );
    }

    #[test]
    fn reads_a_live_shaped_status() {
        let text = "vpnhide 1 status\nbackend 0x4\nkver 0x6019d\nhooks 0x20003ff\nerror 0x0\n\
                    vpnhide 1 stats\n0x2877 0x3:0x9\n";
        let status = read_status_from_text(text).expect("parses");
        assert_eq!(status.backend, 0x4);
        assert_eq!(status.error, 0x0);
    }

    // A text-driven twin of read_status, so the parse is covered without a file.
    fn read_status_from_text(text: &str) -> Result<Status, String> {
        parse_status_fields(text.as_bytes())
            .and_then(|f| f.complete())
            .ok_or_else(|| "invalid".to_string())
    }
}
