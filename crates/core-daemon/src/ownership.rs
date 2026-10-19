//! Core resource inventory markers (core-2.0 §4 / §11).
//!
//! After the activator cutover, core is the **only** writer of canonical config,
//! `/proc/vpnhide_ctl`, Zygisk `targets.txt`, and Ports rules. Markers record that
//! core has claimed/applied a resource for diagnostics and adopt-on-start — they
//! are not a dual-writer fencing protocol.
//!
//! Claiming Zygisk/Native does **not** unload kernel hooks or rewrite already-
//! injected process policy; it only records ownership for the next apply.

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use vpnhide_activator::{
    CONFIG_CORE_OWNER_MARKER, CORE_OWNER_DIR_ENV, NATIVE_CORE_OWNER_MARKER,
    ZYGISK_CORE_OWNER_MARKER, config_owned_by_core, native_owned_by_core,
    ports_owned_by_core, ports_owner_marker_path, zygisk_owned_by_core,
};

use crate::config_file::{self, ConfigLoad};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OwnedResource {
    Config,
    Native,
    Zygisk,
    Ports,
}

pub fn owner_dir() -> PathBuf {
    match std::env::var_os(CORE_OWNER_DIR_ENV) {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => PathBuf::from("/data/adb/vpnhide/core"),
    }
}

pub fn marker_path(resource: OwnedResource) -> PathBuf {
    match resource {
        OwnedResource::Ports => ports_owner_marker_path(),
        OwnedResource::Config => {
            if std::env::var_os(CORE_OWNER_DIR_ENV).is_some() {
                owner_dir().join("config_owned")
            } else {
                PathBuf::from(CONFIG_CORE_OWNER_MARKER)
            }
        }
        OwnedResource::Native => {
            if std::env::var_os(CORE_OWNER_DIR_ENV).is_some() {
                owner_dir().join("native_owned")
            } else {
                PathBuf::from(NATIVE_CORE_OWNER_MARKER)
            }
        }
        OwnedResource::Zygisk => {
            if std::env::var_os(CORE_OWNER_DIR_ENV).is_some() {
                owner_dir().join("zygisk_owned")
            } else {
                PathBuf::from(ZYGISK_CORE_OWNER_MARKER)
            }
        }
    }
}

pub fn is_owned(resource: OwnedResource) -> bool {
    match resource {
        OwnedResource::Config => config_owned_by_core(),
        OwnedResource::Native => native_owned_by_core(),
        OwnedResource::Zygisk => zygisk_owned_by_core(),
        OwnedResource::Ports => ports_owned_by_core(),
    }
}

/// Claim exclusive ownership of a migrated resource before core's first write.
pub fn claim(resource: OwnedResource) -> io::Result<()> {
    let path = marker_path(resource);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(b"owned-by-core\n")?;
        f.sync_all()?;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o644))?;
    }
    fs::rename(&tmp, &path)?;
    Ok(())
}

/// When set (`1`/`true`/`yes`), [`adopt_config_on_start`] runs at core load.
pub const ADOPT_ON_START_ENV: &str = "VPNHIDE_CORE_ADOPT";

pub fn adopt_on_start_enabled() -> bool {
    match std::env::var(ADOPT_ON_START_ENV) {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        // Claiming whenever an explicit owner dir is configured (tests / staged cutover).
        Err(_) => std::env::var_os(CORE_OWNER_DIR_ENV).is_some(),
    }
}

/// Adopt durable config on start: load with Absent≠Unsupported≠Invalid, and
/// claim Config ownership when the file is Present or still Absent so the first
/// Persist is fenced against legacy writers once core is running.
///
/// Does **not** claim on Unsupported — core must not take ownership of a file
/// it cannot safely interpret.
pub fn adopt_config_on_start(path: &Path) -> io::Result<ConfigLoad> {
    let status = config_file::read_status(path)?;
    match &status {
        ConfigLoad::Present(_) | ConfigLoad::Absent => {
            if let Err(e) = claim(OwnedResource::Config) {
                eprintln!("core: failed to claim config ownership: {e}");
            }
        }
        ConfigLoad::UnsupportedVersion { version } => {
            eprintln!(
                "core: refusing config ownership claim; unsupported version {version}"
            );
        }
    }
    Ok(status)
}

/// Claim activation ownership for the backend family about to be written.
pub fn claim_before_native(backend: vpnhide_core_rpc::NativeBackend) -> io::Result<()> {
    match backend {
        vpnhide_core_rpc::NativeBackend::Zygisk => claim(OwnedResource::Zygisk),
        vpnhide_core_rpc::NativeBackend::Kmod
        | vpnhide_core_rpc::NativeBackend::Builtin
        | vpnhide_core_rpc::NativeBackend::Kpm => claim(OwnedResource::Native),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpnhide_activator::PORTS_OWNER_MARKER_ENV;
    #[test]
    fn claim_and_is_owned_roundtrip() {
        let _g = crate::ports::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var(CORE_OWNER_DIR_ENV, dir.path());
            std::env::set_var(PORTS_OWNER_MARKER_ENV, dir.path().join("ports_owned"));
        }
        assert!(!is_owned(OwnedResource::Config));
        claim(OwnedResource::Config).unwrap();
        assert!(is_owned(OwnedResource::Config));
        claim(OwnedResource::Native).unwrap();
        assert!(is_owned(OwnedResource::Native));
        claim(OwnedResource::Zygisk).unwrap();
        assert!(is_owned(OwnedResource::Zygisk));
        claim(OwnedResource::Ports).unwrap();
        assert!(is_owned(OwnedResource::Ports));
        unsafe {
            std::env::remove_var(CORE_OWNER_DIR_ENV);
            std::env::remove_var(PORTS_OWNER_MARKER_ENV);
        }
    }

    #[test]
    fn adopt_claims_on_absent_and_present_not_unsupported() {
        let _g = crate::ports::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var(CORE_OWNER_DIR_ENV, dir.path()) };

        let absent = dir.path().join("missing.json");
        assert_eq!(
            adopt_config_on_start(&absent).unwrap(),
            ConfigLoad::Absent
        );
        assert!(is_owned(OwnedResource::Config));
        let _ = fs::remove_file(marker_path(OwnedResource::Config));

        let present = dir.path().join("ok.json");
        fs::write(
            &present,
            r#"{"version":1,"debug":false,"apps":{},"settings":{}}"#,
        )
        .unwrap();
        match adopt_config_on_start(&present).unwrap() {
            ConfigLoad::Present(_) => {}
            other => panic!("expected Present, got {other:?}"),
        }
        assert!(is_owned(OwnedResource::Config));
        let _ = fs::remove_file(marker_path(OwnedResource::Config));

        let foreign = dir.path().join("v2.json");
        fs::write(&foreign, r#"{"version":99,"apps":{},"settings":{}}"#).unwrap();
        match adopt_config_on_start(&foreign).unwrap() {
            ConfigLoad::UnsupportedVersion { version: 99 } => {}
            other => panic!("expected Unsupported, got {other:?}"),
        }
        assert!(
            !is_owned(OwnedResource::Config),
            "must not claim ownership of an uninterpretable file"
        );

        unsafe { std::env::remove_var(CORE_OWNER_DIR_ENV) };
    }
}
