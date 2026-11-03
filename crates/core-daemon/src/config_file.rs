//! The durable canonical config file (core-2.0 §4 tier-1).
//!
//! The file at `/data/system/vpnhide_config.json` **is** the canonical config, not a
//! write-only projection: core writes it *and* reads it back on restart (reboot recovery),
//! and LSPosed reads it directly (no RPC — it hooks in `system_server`). So this module is
//! the disk codec between that on-disk JSON and core's clean [`CanonicalConfig`].
//!
//! Schema-1 normalization and strict import parsing live in the shared
//! `core-rpc::canonical` codec, also exposed to the APK through typed local UniFFI.
//! This module owns filesystem reads, atomic replacement, reader metadata and
//! post-rename uncertainty; it is not a second format implementation.
//!
//! The path is injectable for isolated tests; production uses the fixed canonical
//! location. The tolerant Kotlin system_server hook reader remains a scoped
//! read-only compatibility adapter; it cannot validate imports or authorize writes.

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use vpnhide_core_rpc::CanonicalConfig;

pub use codec::{ConfigLoad, parse_status, to_json_string};
use vpnhide_core_rpc::canonical as codec;

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
    let mut text = codec::to_pretty_json_string(config);
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
    use super::*;

    fn fixture_config() -> CanonicalConfig {
        match parse_status(include_str!(
            "../../../fixtures/migrations/canonical-v1.json"
        ))
        .unwrap()
        {
            ConfigLoad::Present(config) => config,
            _ => panic!("fixture missing"),
        }
    }

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

    #[test]
    fn write_then_read_file_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vpnhide_config.json");
        let cfg = fixture_config();
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
        let error = write_atomic_with_metadata(&path, &fixture_config(), |_| {
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
        let candidate = fixture_config();
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
        assert!(write_atomic(&path, &fixture_config()).is_err());
        assert_eq!(fs::read_to_string(victim).unwrap(), "untouched");
        assert!(!path.exists());
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
