//! Backend activation (core-2.0 §4/§9.1, step D): core absorbs the activator and applies
//! the native control-v2 wire to the kernel backend itself, instead of shelling out to the
//! per-backend activator binaries. It **links** the activator's projection and delivery
//! (additive pub surface on `vpnhide_activator`) rather than reimplementing hook masks.
//!
//! Scope: the kmod/built-in family (the shared `/proc/vpnhide_ctl` node), which the device
//! runs. KPM (its own CLI + lock) and Zygisk (`targets.txt` via the companion) join later.
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
    BUILTIN_BACKEND_ID, NativeHookFamily, PackageUidMap, deliver_kernel_config, parse_canonical,
    project_native_for_family, project_native_with_resolver_for_family,
};
use vpnhide_core_rpc::CanonicalConfig;
use vpnhide_protocol::{Status, parse_status_fields};

use crate::config_file;

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

/// Project a config into the built-in family's control-v2 wire, resolving package uids from
/// the live PackageManager. Device-only (needs `pm`); host tests use the resolver variant.
pub fn project_builtin_wire(config: &CanonicalConfig) -> Result<String, String> {
    let json = config_file::to_json_string(config);
    project_native_for_family(&json, NativeHookFamily::Builtin).map_err(|e| e.to_string())
}

/// Project with an explicit uid resolver — deterministic and PackageManager-free, for tests.
pub fn project_builtin_wire_with_resolver(
    config: &CanonicalConfig,
    resolver: &PackageUidMap,
) -> Result<String, String> {
    let json = config_file::to_json_string(config);
    let cfg = parse_canonical(&json).map_err(|e| e.to_string())?;
    Ok(project_native_with_resolver_for_family(
        &cfg,
        resolver,
        NativeHookFamily::Builtin,
    ))
}

/// Deliver a control-v2 `wire` to the built-in control node at `path`, refusing unless the
/// node reports the built-in backend (never configure a foreign owner).
pub fn deliver_builtin(path: &Path, wire: &str) -> Result<(), String> {
    deliver_kernel_config(path, BUILTIN_BACKEND_ID, wire).map_err(|e| e.to_string())
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
        assert!(kmod_ctl_path_from_env().is_none(), "unset -> no native write");
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
                native: NativeRole { enabled: true, kernel: None, zygisk: None },
                ..Default::default()
            },
        );
        let wire = project_builtin_wire_with_resolver(&config, &resolver).expect("projects");
        // The wire is a control-v2 config snapshot; the resolved uid appears as bare hex
        // (10234 == 0x27fa), per the protocol's control-side numeric rule.
        assert!(wire.contains("27fa"), "projected wire must target the resolved uid: {wire}");
    }

    #[test]
    fn empty_config_projects_without_targets() {
        let resolver = PackageUidMap::default();
        let wire = project_builtin_wire_with_resolver(&CanonicalConfig::default(), &resolver)
            .expect("projects");
        assert!(wire.contains("config"), "still a valid config snapshot: {wire}");
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
