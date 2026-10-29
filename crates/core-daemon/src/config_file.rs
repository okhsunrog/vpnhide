//! The durable canonical config file (core-2.0 §4 tier-1).
//!
//! The file at `/data/system/vpnhide_config.json` **is** the canonical config, not a
//! write-only projection: core writes it *and* reads it back on restart (reboot recovery),
//! and LSPosed reads it directly (no RPC — it hooks in `system_server`). So this module is
//! the disk codec between that on-disk JSON and core's clean [`CanonicalConfig`].
//!
//! # The on-disk format is polymorphic; the model is not
//!
//! The legacy format the app writes today (`StorageConfig.kt`) encodes some fields several
//! ways: `java` is a bool *or* a hook-name array; `native` is a bool, an array, *or* an
//! object; `hidden` is written only when true; `portPolicy` only when `ports`. That is a
//! wire-encoding concern, not a data concern — the normalized [`CanonicalConfig`] carries
//! the same information (`java: bool` + `java_hooks: Option`, `native: {enabled,kernel,
//! zygisk}`). This module is a faithful port of `parseCanonicalConfig` /
//! `canonicalConfigJson`, so a file written by the old app reads identically, and a file
//! core writes is read identically by LSPosed and the activator (which must not change).
//!
//! The activator's own `CanonicalConfig` (`crates/activator/model.rs`) is a **lossy,
//! deserialize-only** read model (it collapses `java` to a bool it never uses); it cannot
//! be reused here, where the round-trip must lose nothing.
//!
//! The path is a parameter, so the dev sandbox core points at an isolated file and never
//! touches the real `/data/system/vpnhide_config.json`.

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

#[cfg(test)]
use serde_json::json;
use vpnhide_core_rpc::CanonicalConfig;
#[cfg(test)]
use vpnhide_core_rpc::{AppConfig, NativeRole, PortPolicy, PortProtocol, PortRule, Settings};

mod codec;
pub use codec::{parse_status, to_json_string};

/// The production path of the durable canonical config.
pub const DEFAULT_CONFIG_PATH: &str = "/data/system/vpnhide_config.json";

/// Environment override for the config path — the dev sandbox core sets this to an isolated
/// file so it never writes the real one. Empty/unset falls back to [`DEFAULT_CONFIG_PATH`].
pub const CONFIG_PATH_ENV: &str = "VPNHIDE_CONFIG_PATH";

/// Resolve the config path from the environment, falling back to the production default.
pub fn config_path_from_env() -> PathBuf {
    match std::env::var(CONFIG_PATH_ENV) {
        Ok(p) if !p.is_empty() => PathBuf::from(p),
        _ => PathBuf::from(DEFAULT_CONFIG_PATH),
    }
}

/// Outcome of loading the durable canonical config. Absent, present, and
/// unsupported schema are **distinct** — never collapse unsupported/invalid into
/// an empty default config (core-2.0 §4 / exclusive-ownership gate).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigLoad {
    /// File missing (or empty import input). An existing blank file is an error.
    Absent,
    /// Parsed `version: 1` config.
    Present(CanonicalConfig),
    /// File exists with a schema version this build must not interpret.
    UnsupportedVersion { version: u32 },
}

/// Read with full Absent / Present / Unsupported distinction.
/// Corrupt JSON or IO failures (other than NotFound) are `Err` — also not Absent.
pub fn read_status(path: &Path) -> io::Result<ConfigLoad> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(ConfigLoad::Absent),
        Err(e) => return Err(e),
    };
    if raw.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "blank canonical file",
        ));
    }
    parse_status(&raw)
}

/// Convenience: `Present` → `Some`, `Absent` → `None`.
/// Unsupported schema is an **error** (not silently Absent) so callers cannot
/// treat a foreign file as "no config" and write defaults over it.
pub fn read(path: &Path) -> io::Result<Option<CanonicalConfig>> {
    match read_status(path)? {
        ConfigLoad::Absent => Ok(None),
        ConfigLoad::Present(cfg) => Ok(Some(cfg)),
        ConfigLoad::UnsupportedVersion { version } => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported config version {version}"),
        )),
    }
}

/// Serialize and write the config atomically (temp in the same dir → `rename`), mode `0640`.
/// The rename is the commit point, so a reader (LSPosed, the activator, a restarting core)
/// never sees a half-written file. Android publication requires `root:system` and
/// `system_data_file` so the LSPosed reader in system_server can read the new inode.
pub fn write_atomic(path: &Path, config: &CanonicalConfig) -> io::Result<()> {
    write_atomic_with_metadata(path, config, set_reader_metadata)
}

fn write_atomic_with_metadata(
    path: &Path,
    config: &CanonicalConfig,
    metadata: impl FnOnce(&fs::File) -> io::Result<()>,
) -> io::Result<()> {
    write_atomic_with_sync(path, config, metadata, |dir| {
        fs::File::open(dir)?.sync_all()
    })
}

fn write_atomic_with_sync(
    path: &Path,
    config: &CanonicalConfig,
    metadata: impl FnOnce(&fs::File) -> io::Result<()>,
    sync_directory: impl FnOnce(&Path) -> io::Result<()>,
) -> io::Result<()> {
    let value = codec::to_disk(config);
    let mut text = serde_json::to_string_pretty(&value)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    text.push('\n');

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = path.with_extension("json.tmp");
    let prepared = (|| {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&tmp)?;
        metadata(&f)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        Ok::<_, io::Error>(())
    })();
    if let Err(error) = prepared {
        let _ = fs::remove_file(&tmp);
        return Err(error);
    }
    fs::rename(&tmp, path)?;
    sync_directory(dir).map_err(|error| io::Error::new(error.kind(), PostRenameError(error)))
}

#[derive(Debug)]
struct PostRenameError(io::Error);

impl std::fmt::Display for PostRenameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "publication renamed but directory durability is unconfirmed: {}",
            self.0
        )
    }
}
impl std::error::Error for PostRenameError {}

pub fn is_post_rename_error(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.is::<PostRenameError>())
}

pub fn sync_after_rename(dir: &Path) -> io::Result<()> {
    fs::File::open(dir)
        .and_then(|file| file.sync_all())
        .map_err(|error| io::Error::new(error.kind(), PostRenameError(error)))
}

fn set_reader_metadata(file: &fs::File) -> io::Result<()> {
    #[cfg(target_os = "android")]
    {
        use std::os::fd::AsRawFd;
        // AID_SYSTEM is stable Android ABI. Set metadata before publication and
        // refuse the write if it cannot be made readable by system_server.
        if unsafe { libc::fchown(file.as_raw_fd(), 0, 1000) } != 0 {
            return Err(io::Error::last_os_error());
        }
        const NAME: &[u8] = b"security.selinux\0";
        const LABEL: &[u8] = b"u:object_r:system_data_file:s0\0";
        if unsafe {
            libc::fsetxattr(
                file.as_raw_fd(),
                NAME.as_ptr().cast(),
                LABEL.as_ptr().cast(),
                LABEL.len(),
                0,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    file.set_permissions(fs::Permissions::from_mode(0o640))
}

#[cfg(test)]
mod tests {
    use super::codec::{app_from_disk, from_disk, to_disk};
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn shared_v1_fixture_normalizes_once_and_survives_atomic_roundtrip() {
        let raw = include_str!("../../../fixtures/migrations/canonical-v1.json");
        let ConfigLoad::Present(config) = parse_status(raw).unwrap() else {
            panic!("present");
        };
        assert!(config.apps["com.bank.app"].ports);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        write_atomic(&path, &config).unwrap();
        assert_eq!(read(&path).unwrap(), Some(config.clone()));
        assert_eq!(
            parse_status(&to_json_string(&config)).unwrap(),
            ConfigLoad::Present(config)
        );
    }

    fn app(f: impl FnOnce(&mut AppConfig)) -> AppConfig {
        let mut a = AppConfig::default();
        f(&mut a);
        a
    }

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// native=all (enabled, no per-hook overrides).
    fn native_all() -> NativeRole {
        NativeRole {
            enabled: true,
            kernel: None,
            zygisk: None,
        }
    }

    /// A config exercising every polymorphic shape and conditional field.
    fn rich() -> CanonicalConfig {
        let mut apps = BTreeMap::new();
        // java=all, native=all
        apps.insert(
            "com.a".into(),
            app(|a| {
                a.java = true;
                a.native = native_all();
            }),
        );
        // java per-hook list, native per-hook object with both families
        apps.insert(
            "com.b".into(),
            app(|a| {
                a.java = true;
                a.java_hooks = Some(vec!["hookX".into(), "hookY".into()]);
                a.native = NativeRole {
                    enabled: true,
                    kernel: Some(vec!["k1".into()]),
                    zygisk: Some(vec!["z1".into(), "z2".into()]),
                };
            }),
        );
        // hidden only, app_hiding
        apps.insert(
            "com.c".into(),
            app(|a| {
                a.hidden = true;
                a.app_hiding = true;
            }),
        );
        // ports with a policy (single-port rule + range rule)
        apps.insert(
            "com.d".into(),
            app(|a| {
                a.ports = true;
                a.port_policy = Some(PortPolicy {
                    mode: "custom".into(),
                    preset: Some("torrents".into()),
                    rules: vec![
                        PortRule {
                            protocol: PortProtocol::Tcp,
                            start: 443,
                            end: 443,
                        },
                        PortRule {
                            protocol: PortProtocol::Both,
                            start: 6881,
                            end: 6889,
                        },
                    ],
                });
            }),
        );
        CanonicalConfig {
            version: 1,
            debug: true,
            debug_switch: false,
            apps,
            settings: Settings {
                remember_superkey: true,
                optional_features: set(&["filesystem_iface_paths"]),
                auto_hide_vpn_services: false,
                auto_hide_vpn_name: true,
                auto_hide_excluded_packages: set(&["com.x"]),
                auto_hidden_packages: set(&["com.y", "com.z"]),
            },
        }
    }

    #[test]
    fn round_trips_through_disk() {
        let cfg = rich();
        let back = from_disk(&to_disk(&cfg)).expect("some");
        assert_eq!(back, cfg);
    }

    #[test]
    fn write_then_read_file_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vpnhide_config.json");
        let cfg = rich();
        write_atomic(&path, &cfg).unwrap();
        // Mode is 0640 and the temp file is gone.
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
        assert!(!path.with_extension("json.tmp").exists());
        assert_eq!(read(&path).unwrap(), Some(cfg));
    }

    #[test]
    fn reader_metadata_failure_preserves_previous_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vpnhide_config.json");
        let previous = CanonicalConfig::default();
        write_atomic(&path, &previous).unwrap();
        let error = write_atomic_with_metadata(&path, &rich(), |_| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "reader label denied",
            ))
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(read(&path).unwrap(), Some(previous));
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn directory_sync_failure_after_rename_is_uncertain_and_never_known_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        write_atomic(&path, &CanonicalConfig::default()).unwrap();
        let candidate = rich();
        let error = write_atomic_with_sync(&path, &candidate, set_reader_metadata, |_| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "directory fsync refused",
            ))
        })
        .unwrap_err();
        assert!(is_post_rename_error(&error));
        assert_eq!(read(&path).unwrap(), Some(candidate));
    }

    #[test]
    fn staging_symlink_cannot_redirect_config_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vpnhide_config.json");
        let victim = dir.path().join("other-file");
        fs::write(&victim, "untouched").unwrap();
        std::os::unix::fs::symlink(&victim, path.with_extension("json.tmp")).unwrap();
        assert!(write_atomic(&path, &rich()).is_err());
        assert_eq!(fs::read_to_string(victim).unwrap(), "untouched");
        assert!(!path.exists());
    }

    #[test]
    fn encodes_the_polymorphic_and_conditional_shapes() {
        let v = to_disk(&rich());
        let apps = v["apps"].as_object().unwrap();
        // java: bool true (com.a) vs hook array (com.b)
        assert_eq!(apps["com.a"]["java"], json!(true));
        assert_eq!(apps["com.b"]["java"], json!(["hookX", "hookY"]));
        // native: bool true (com.a) vs object (com.b)
        assert_eq!(apps["com.a"]["native"], json!(true));
        assert_eq!(
            apps["com.b"]["native"],
            json!({"enabled": true, "kernel": ["k1"], "zygisk": ["z1", "z2"]})
        );
        // hidden written only when true; com.a has no hidden key, com.c does
        assert!(apps["com.a"].get("hidden").is_none());
        assert_eq!(apps["com.c"]["hidden"], json!(true));
        // portPolicy present only under ports; single-port rule omits `end`
        assert!(apps["com.a"].get("portPolicy").is_none());
        let rules = apps["com.d"]["portPolicy"]["rules"].as_array().unwrap();
        assert!(rules[0].get("end").is_none()); // 443..443
        assert_eq!(rules[1]["end"], json!(6889)); // range keeps end
    }

    #[test]
    fn reads_a_legacy_app_written_file() {
        // Shaped exactly like StorageConfig.kt's canonicalConfigJson output.
        let raw = r#"{
          "version": 1,
          "debug": false,
          "debugSwitch": true,
          "apps": {
            "com.bool": { "java": false, "native": true, "appHiding": false, "ports": false },
            "com.arr": { "java": ["h1"], "native": ["k1", "k2"], "appHiding": true, "ports": false },
            "com.norole": { "java": false, "native": false, "appHiding": false, "ports": false }
          },
          "settings": {
            "rememberSuperkey": false,
            "optionalFeatures": [],
            "autoHideVpnServices": true,
            "autoHideVpnName": false,
            "autoHideExcludedPackages": [],
            "autoHiddenPackages": []
          }
        }"#;
        let cfg = from_disk(&serde_json::from_str(raw).unwrap()).expect("some");
        assert!(cfg.debug_switch); // explicit, not defaulted from debug
        // no-role app dropped
        assert!(!cfg.apps.contains_key("com.norole"));
        // bool native -> enabled, no overrides
        assert_eq!(cfg.apps["com.bool"].native, native_all());
        // array java -> java=true + hooks; array native -> kernel overrides
        assert_eq!(
            cfg.apps["com.arr"].java_hooks.as_deref(),
            Some(&["h1".to_string()][..])
        );
        assert_eq!(
            cfg.apps["com.arr"].native.kernel.as_deref(),
            Some(&["k1".to_string(), "k2".to_string()][..])
        );
    }

    #[test]
    fn debug_switch_defaults_to_debug_when_absent() {
        let raw = r#"{"version":1,"debug":true,"apps":{},"settings":{}}"#;
        let cfg = from_disk(&serde_json::from_str(raw).unwrap()).unwrap();
        assert!(cfg.debug_switch, "debugSwitch absent must inherit debug");
    }

    #[test]
    fn disabled_native_object_drops_overrides() {
        let raw = r#"{"native":{"enabled":false,"kernel":["k1"]},"appHiding":true}"#;
        let a = app_from_disk(&serde_json::from_str(raw).unwrap());
        assert_eq!(a.native, NativeRole::default());
    }

    #[test]
    fn malformed_port_rule_disables_ports() {
        // start out of range -> whole policy dropped -> ports disabled (present-but-bad).
        let raw = r#"{"ports":true,"portPolicy":{"mode":"custom","rules":[{"start":70000}]}}"#;
        let a = app_from_disk(&serde_json::from_str(raw).unwrap());
        assert!(!a.ports);
        assert!(a.port_policy.is_none());
    }

    #[test]
    fn newer_version_is_refused() {
        let raw = r#"{"version":2,"apps":{},"settings":{}}"#;
        assert_eq!(from_disk(&serde_json::from_str(raw).unwrap()), None);
    }

    #[test]
    fn missing_file_is_absent_but_present_blank_file_is_invalid() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(&dir.path().join("nope.json")).unwrap(), None);
        assert_eq!(
            read_status(&dir.path().join("nope.json")).unwrap(),
            ConfigLoad::Absent
        );
        let blank = dir.path().join("blank.json");
        fs::write(&blank, "   \n").unwrap();
        assert!(read(&blank).is_err());
        assert!(read_status(&blank).is_err());
    }

    #[test]
    fn corrupt_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.json");
        fs::write(&bad, "{ not json").unwrap();
        assert!(read(&bad).is_err());
        assert!(read_status(&bad).is_err());
    }

    #[test]
    fn unsupported_version_is_not_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cfg.json");
        fs::write(&path, r#"{"version":2,"apps":{},"settings":{}}"#).unwrap();
        match read_status(&path).unwrap() {
            ConfigLoad::UnsupportedVersion { version: 2 } => {}
            other => panic!("expected UnsupportedVersion, got {other:?}"),
        }
        let err = read(&path).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("unsupported config version 2"));
    }
}
