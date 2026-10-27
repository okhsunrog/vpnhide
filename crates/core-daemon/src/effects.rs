//! `root_effect`: the fixed root writes outside the canonical config (protocol v5).
//!
//! The app used to run these as shell lines on its own root lane. Each is now a typed effect
//! with paths fixed here; nothing is caller-chosen. Effects that touch the config (full reset)
//! go through [`CoreState`](crate::CoreState) so the published state follows.

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

use vpnhide_core_rpc::{EffectError, RootEffect};

/// The APatch superkey core's KPM control reads (`activator::kpm`).
pub const SUPERKEY_FILE: &str = "/data/adb/vpnhide/superkey";
/// A superkey is a short secret; anything longer is a caller bug.
pub const MAX_SUPERKEY_BYTES: usize = 4096;

/// Pre-1.0 per-component config lists, retired after an import (`LegacyConfigImport`).
pub const LEGACY_FILES: &[&str] = &[
    "/data/adb/vpnhide_kmod/targets.txt",
    "/data/adb/vpnhide_kpm/targets.txt",
    "/data/adb/vpnhide_zygisk/targets.txt",
    "/data/adb/vpnhide_lsposed/targets.txt",
    "/data/adb/vpnhide_ports/observers.txt",
    "/data/system/vpnhide_hidden_pkgs.txt",
    "/data/system/vpnhide_observer_uids.txt",
    "/data/system/vpnhide_uids.txt",
    "/data/system/vpnhide_hook_active",
];
/// Dirs that only ever held legacy files; removed only when empty.
pub const LEGACY_ORPHAN_DIRS: &[&str] = &["/data/adb/vpnhide_zygisk", "/data/adb/vpnhide_lsposed"];

/// Files a full reset removes besides the legacy set (the config is removed through core).
pub const RESET_FILES: &[&str] = &["/data/system/vpnhide_lsposed_state"];
/// Backend state dirs a full reset removes whole. Never a module install dir.
pub const RESET_DIRS: &[&str] = &[
    "/data/adb/vpnhide_kmod",
    "/data/adb/vpnhide_builtin",
    "/data/adb/vpnhide_kpm",
    "/data/adb/vpnhide_ports",
];
/// The shared state dir: a full reset empties it except core's own state.
pub const STATE_DIR: &str = "/data/adb/vpnhide";
const STATE_DIR_KEEP: &[&str] = &["core"];

/// Paths the effects act on; production uses the constants, tests a temp root.
#[derive(Debug, Clone)]
pub struct EffectPaths {
    pub root: std::path::PathBuf,
}

impl Default for EffectPaths {
    fn default() -> Self {
        EffectPaths {
            root: std::path::PathBuf::from("/"),
        }
    }
}

/// Env: `1` enables root effects on the real filesystem (set by the core module only).
pub const ROOT_EFFECTS_ENV: &str = "VPNHIDE_ROOT_EFFECTS";

pub fn effect_paths_from_env() -> Option<EffectPaths> {
    (std::env::var(ROOT_EFFECTS_ENV).as_deref() == Ok("1")).then(EffectPaths::default)
}

impl EffectPaths {
    fn at(&self, absolute: &str) -> std::path::PathBuf {
        self.root.join(absolute.trim_start_matches('/'))
    }
}

/// Apply one effect that does not touch the config. [`RootEffect::FullReset`]'s config part
/// is done by the caller (`CoreState`); this removes everything else.
pub fn apply(paths: &EffectPaths, effect: &RootEffect) -> Result<(), EffectError> {
    match effect {
        RootEffect::SetSuperkey { superkey } => {
            let key = superkey.trim();
            if key.is_empty() || key.len() > MAX_SUPERKEY_BYTES || key.contains(['\n', '\0']) {
                return Err(EffectError::Invalid("superkey".into()));
            }
            write_secret(&paths.at(SUPERKEY_FILE), &format!("{key}\n"))
                .map_err(|e| EffectError::Failed(format!("write superkey: {}", e.kind())))
        }
        RootEffect::ClearSuperkey => remove_file(&paths.at(SUPERKEY_FILE)),
        RootEffect::DeleteLegacyConfig => {
            for file in LEGACY_FILES {
                remove_file(&paths.at(file))?;
            }
            for dir in LEGACY_ORPHAN_DIRS {
                // Only when empty: a non-empty one is left alone on purpose.
                let _ = fs::remove_dir(paths.at(dir));
            }
            Ok(())
        }
        RootEffect::FullReset => {
            for file in LEGACY_FILES.iter().chain(RESET_FILES) {
                remove_file(&paths.at(file))?;
            }
            for dir in RESET_DIRS.iter().chain(LEGACY_ORPHAN_DIRS) {
                remove_dir_all(&paths.at(dir))?;
            }
            let state = paths.at(STATE_DIR);
            match fs::read_dir(&state) {
                Ok(entries) => {
                    for entry in entries {
                        let entry = entry.map_err(|error| {
                            EffectError::Failed(format!("reset inventory: {}", error.kind()))
                        })?;
                        let name = entry.file_name();
                        if STATE_DIR_KEEP.iter().any(|keep| name == *keep) {
                            continue;
                        }
                        let path = entry.path();
                        let removed = if entry.file_type().is_ok_and(|t| t.is_dir()) {
                            remove_dir_all(&path)
                        } else {
                            remove_file(&path)
                        };
                        removed?;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(EffectError::Failed(format!(
                        "reset inventory: {}",
                        error.kind()
                    )));
                }
            }
            Ok(())
        }
    }
}

fn remove_file(path: &Path) -> Result<(), EffectError> {
    match fs::remove_file(path) {
        Ok(()) => sync_parent(path),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(EffectError::Failed(format!(
            "remove {}: {}",
            path.display(),
            e.kind()
        ))),
    }
}

fn remove_dir_all(path: &Path) -> Result<(), EffectError> {
    match fs::remove_dir_all(path) {
        Ok(()) => sync_parent(path),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(EffectError::Failed(format!(
            "remove {}: {}",
            path.display(),
            e.kind()
        ))),
    }
}

/// Root-only (0600, parent 0700) atomic write: tmp + fsync + rename.
fn write_secret(path: &Path, contents: &str) -> io::Result<()> {
    let parent = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    fs::create_dir_all(parent)?;
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    let tmp = parent.join(format!(
        ".{}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("secret")
    ));
    {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&tmp)?;
        f.write_all(contents.as_bytes())?;
        f.sync_all()?;
    }
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    fs::rename(&tmp, path)?;
    crate::config_file::sync_after_rename(parent)
}

fn sync_parent(path: &Path) -> Result<(), EffectError> {
    crate::config_file::sync_after_rename(path.parent().unwrap_or_else(|| Path::new(".")))
        .map_err(|error| EffectError::Failed(format!("directory durability: {}", error.kind())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> (tempfile::TempDir, EffectPaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = EffectPaths {
            root: dir.path().to_path_buf(),
        };
        (dir, paths)
    }

    fn touch(paths: &EffectPaths, abs: &str) {
        let p = paths.at(abs);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, "x").unwrap();
    }

    #[test]
    fn superkey_is_written_root_only_and_cleared() {
        let (_d, paths) = root();
        apply(
            &paths,
            &RootEffect::SetSuperkey {
                superkey: "  s3cret \n".into(),
            },
        )
        .unwrap();
        let file = paths.at(SUPERKEY_FILE);
        assert_eq!(fs::read_to_string(&file).unwrap(), "s3cret\n");
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        apply(&paths, &RootEffect::ClearSuperkey).unwrap();
        assert!(!file.exists());
        apply(&paths, &RootEffect::ClearSuperkey).unwrap();
    }

    #[test]
    fn malformed_superkeys_are_refused() {
        let (_d, paths) = root();
        for bad in ["", "   ", "a\nb", &"k".repeat(MAX_SUPERKEY_BYTES + 1)] {
            assert!(matches!(
                apply(
                    &paths,
                    &RootEffect::SetSuperkey {
                        superkey: bad.into()
                    }
                ),
                Err(EffectError::Invalid(_))
            ));
        }
    }

    #[test]
    fn legacy_cleanup_keeps_live_backend_dirs_and_non_empty_orphans() {
        let (_d, paths) = root();
        for f in LEGACY_FILES {
            touch(&paths, f);
        }
        touch(&paths, "/data/adb/vpnhide_kmod/load_status");
        touch(&paths, "/data/adb/vpnhide_lsposed/other");
        apply(&paths, &RootEffect::DeleteLegacyConfig).unwrap();
        for f in LEGACY_FILES {
            assert!(!paths.at(f).exists(), "{f}");
        }
        assert!(paths.at("/data/adb/vpnhide_kmod/load_status").exists());
        assert!(paths.at("/data/adb/vpnhide_lsposed/other").exists());
        assert!(!paths.at("/data/adb/vpnhide_zygisk").exists());
    }

    #[test]
    fn full_reset_keeps_core_state_and_module_dirs() {
        let (_d, paths) = root();
        touch(&paths, "/data/adb/vpnhide/superkey");
        touch(&paths, "/data/adb/vpnhide/app-state/lane");
        touch(&paths, "/data/adb/vpnhide/core/operations.json");
        touch(&paths, "/data/adb/vpnhide_kpm/load_status");
        touch(&paths, "/data/adb/modules/vpnhide_kpm/module.prop");
        touch(&paths, "/data/system/vpnhide_lsposed_state");
        apply(&paths, &RootEffect::FullReset).unwrap();
        assert!(!paths.at("/data/adb/vpnhide/superkey").exists());
        assert!(!paths.at("/data/adb/vpnhide/app-state").exists());
        assert!(paths.at("/data/adb/vpnhide/core/operations.json").exists());
        assert!(!paths.at("/data/adb/vpnhide_kpm").exists());
        assert!(
            paths
                .at("/data/adb/modules/vpnhide_kpm/module.prop")
                .exists()
        );
        assert!(!paths.at("/data/system/vpnhide_lsposed_state").exists());
    }
}
