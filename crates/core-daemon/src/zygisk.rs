//! Publish the existing Zygisk control-v2 file through a verified module directory.
//! Injection and per-process hook measurements remain in their existing contexts.
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

/// No default before ownership cutover; merely installing the artifact does not
/// authorize a second writer next to the legacy service.
pub const MODULE_ENV: &str = "VPNHIDE_ZYGISK_MODULE";

pub fn module_from_env() -> Option<PathBuf> {
    std::env::var_os(MODULE_ENV)
        .filter(|p| !p.is_empty())
        .map(Into::into)
}

pub struct ZygiskTarget {
    path: PathBuf,
    directory: File,
}

#[derive(Debug)]
pub enum PublishError {
    BeforeWrite(String),
    /// Publication/durability was attempted without established completion.
    Unknown(String),
}

fn open_directory(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
}

fn entry_exists(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}

impl ZygiskTarget {
    pub fn open(path: &Path) -> Result<Self, String> {
        let target = Self {
            path: path.to_owned(),
            directory: open_directory(path).map_err(|e| e.to_string())?,
        };
        target.verify()?;
        Ok(target)
    }

    fn pinned_path(&self) -> PathBuf {
        PathBuf::from(format!("/proc/self/fd/{}", self.directory.as_raw_fd()))
    }

    fn verify(&self) -> Result<(), String> {
        let current = open_directory(&self.path).map_err(|e| e.to_string())?;
        let old = self.directory.metadata().map_err(|e| e.to_string())?;
        let new = current.metadata().map_err(|e| e.to_string())?;
        if (old.dev(), old.ino()) != (new.dev(), new.ino()) {
            return Err("Zygisk module directory changed during activation".into());
        }
        let root = self.pinned_path();
        if entry_exists(&root.join("disable"))? || entry_exists(&root.join("remove"))? {
            return Err("Zygisk module is disabled or scheduled for removal".into());
        }
        let manifest = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(root.join("module.prop"))
            .map_err(|e| e.to_string())?;
        if !manifest.metadata().map_err(|e| e.to_string())?.is_file() {
            return Err("Zygisk module manifest is not a regular file".into());
        }
        let mut text = String::new();
        manifest
            .take(8193)
            .read_to_string(&mut text)
            .map_err(|e| e.to_string())?;
        if text.len() > 8192 {
            return Err("Zygisk module manifest is too large".into());
        }
        let mut id = None;
        let mut version = None;
        for line in text
            .lines()
            .map(str::trim)
            .filter(|line| !line.starts_with('#'))
        {
            if let Some((key, value)) = line.split_once('=') {
                let slot = match key {
                    "id" => &mut id,
                    "vpnhideControl" => &mut version,
                    _ => continue,
                };
                if slot.replace(value).is_some() {
                    return Err("ambiguous Zygisk module manifest".into());
                }
            }
        }
        if id != Some("vpnhide_zygisk") || version != Some("2") {
            return Err("Zygisk recipient identity or control protocol is incompatible".into());
        }
        let libraries = root.join("zygisk");
        if !fs::symlink_metadata(&libraries)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_dir()
        {
            return Err("Zygisk library directory is not a directory".into());
        }
        let mut found = false;
        for (name, class, machine, size) in [
            ("arm64-v8a.so", 2, 183u16, 64),
            ("armeabi-v7a.so", 1, 40u16, 52),
        ] {
            let path = libraries.join(name);
            if !entry_exists(&path)? {
                continue;
            }
            let mut file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(path)
                .map_err(|e| e.to_string())?;
            let metadata = file.metadata().map_err(|e| e.to_string())?;
            if !metadata.is_file() || metadata.len() < size {
                return Err("invalid Zygisk library artifact".into());
            }
            let mut header = [0; 20];
            file.read_exact(&mut header).map_err(|e| e.to_string())?;
            if &header[..4] != b"\x7fELF"
                || header[4] != class
                || header[5] != 1
                || header[6] != 1
                || u16::from_le_bytes([header[16], header[17]]) != 3
                || u16::from_le_bytes([header[18], header[19]]) != machine
            {
                return Err("incompatible Zygisk ELF class, type or architecture".into());
            }
            found = true;
        }
        if !found {
            return Err("Zygisk module has no supported library artifact".into());
        }
        Ok(())
    }

    pub fn publish(&self, wire: &str) -> Result<(), PublishError> {
        let preflight = || -> Result<PathBuf, String> {
            vpnhide_activator::validate_zygisk_config_wire(wire).map_err(|e| e.to_string())?;
            self.verify()?;
            let path = self.pinned_path().join("targets.txt");
            if entry_exists(&path)?
                && !fs::symlink_metadata(&path)
                    .map_err(|e| e.to_string())?
                    .file_type()
                    .is_file()
            {
                return Err("Zygisk policy destination is not a regular file".into());
            }
            // The shared atomic writer uses this per-process staging name. A
            // pre-existing symlink must never redirect its privileged write.
            let temp = path.with_extension(format!("tmp.{}", std::process::id()));
            if entry_exists(&temp)? {
                let metadata = fs::symlink_metadata(temp).map_err(|e| e.to_string())?;
                if !metadata.file_type().is_file() || metadata.nlink() != 1 {
                    return Err(
                        "Zygisk policy staging destination is not a private regular file".into(),
                    );
                }
            }
            Ok(path)
        };
        let path = preflight().map_err(PublishError::BeforeWrite)?;
        vpnhide_activator::write_atomic(&path, wire.as_bytes(), 0o644)
            .and_then(|()| Ok(self.directory.sync_all()?))
            .map_err(|e| PublishError::Unknown(e.to_string()))
    }
}

#[cfg(test)]
pub(crate) use tests::fixture as test_fixture;

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    const OLD: &str = "previous policy";
    const WIRE: &str = "vpnhide 2 config\ndebug 0\ndefault 0\nend 0\n";

    pub(crate) fn library(path: &Path, class: u8, machine: u16) {
        let mut bytes = vec![0; if class == 1 { 52 } else { 64 }];
        bytes[..4].copy_from_slice(b"\x7fELF");
        bytes[4] = class;
        bytes[5] = 1;
        bytes[6] = 1;
        bytes[16..18].copy_from_slice(&3u16.to_le_bytes());
        bytes[18..20].copy_from_slice(&machine.to_le_bytes());
        fs::write(path, bytes).unwrap();
    }

    pub(crate) fn fixture(path: &Path) {
        fs::create_dir_all(path.join("zygisk")).unwrap();
        fs::write(
            path.join("module.prop"),
            include_str!("../../../zygisk/module/module.prop"),
        )
        .unwrap();
        library(&path.join("zygisk/arm64-v8a.so"), 2, 183);
        fs::write(path.join("targets.txt"), OLD).unwrap();
    }

    #[test]
    fn publishes_atomically_for_supported_abis_without_claiming_injection() {
        for abi in ["64", "32", "both"] {
            let dir = tempfile::tempdir().unwrap();
            fixture(dir.path());
            if abi != "64" {
                library(&dir.path().join("zygisk/armeabi-v7a.so"), 1, 40);
            }
            if abi == "32" {
                fs::remove_file(dir.path().join("zygisk/arm64-v8a.so")).unwrap();
            }
            let mut old_reader = File::open(dir.path().join("targets.txt")).unwrap();
            ZygiskTarget::open(dir.path())
                .unwrap()
                .publish(WIRE)
                .unwrap();
            let mut text = String::new();
            old_reader.read_to_string(&mut text).unwrap();
            assert_eq!(text, OLD, "existing reader retains its whole old snapshot");
            assert_eq!(
                fs::read_to_string(dir.path().join("targets.txt")).unwrap(),
                WIRE
            );
            assert_eq!(
                fs::metadata(dir.path().join("targets.txt"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o644
            );
        }
    }

    #[test]
    fn rejects_foreign_incompatible_or_ambiguous_manifest_without_writing() {
        for profile in [
            "id=other\nvpnhideControl=2\n",
            "id=vpnhide_zygisk\n",
            "id=vpnhide_zygisk\nvpnhideControl=3\n",
            "id=vpnhide_zygisk\nvpnhideControl=2\nvpnhideControl=2\n",
        ] {
            let dir = tempfile::tempdir().unwrap();
            fixture(dir.path());
            fs::write(dir.path().join("module.prop"), profile).unwrap();
            assert!(ZygiskTarget::open(dir.path()).is_err());
            assert_eq!(
                fs::read_to_string(dir.path().join("targets.txt")).unwrap(),
                OLD
            );
        }
    }

    #[test]
    fn verifies_artifact_architecture_and_manifest_file_type() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        library(&dir.path().join("zygisk/arm64-v8a.so"), 1, 40);
        assert!(ZygiskTarget::open(dir.path()).is_err());
        library(&dir.path().join("zygisk/arm64-v8a.so"), 2, 183);
        fs::rename(
            dir.path().join("module.prop"),
            dir.path().join("other.prop"),
        )
        .unwrap();
        symlink("other.prop", dir.path().join("module.prop")).unwrap();
        assert!(ZygiskTarget::open(dir.path()).is_err());
    }

    #[test]
    fn rechecks_disable_removal_and_protocol_after_selection() {
        for change in ["disable", "remove", "protocol", "library"] {
            let dir = tempfile::tempdir().unwrap();
            fixture(dir.path());
            let target = ZygiskTarget::open(dir.path()).unwrap();
            match change {
                "protocol" => fs::write(dir.path().join("module.prop"), "id=foreign\n").unwrap(),
                "library" => fs::remove_file(dir.path().join("zygisk/arm64-v8a.so")).unwrap(),
                marker => fs::write(dir.path().join(marker), "").unwrap(),
            }
            assert!(matches!(
                target.publish(WIRE),
                Err(PublishError::BeforeWrite(_))
            ));
            assert_eq!(
                fs::read_to_string(dir.path().join("targets.txt")).unwrap(),
                OLD
            );
        }
    }

    #[test]
    fn refuses_replaced_module_directory() {
        let dir = tempfile::tempdir().unwrap();
        let module = dir.path().join("module");
        fixture(&module);
        let target = ZygiskTarget::open(&module).unwrap();
        let moved = dir.path().join("moved");
        fs::rename(&module, &moved).unwrap();
        fixture(&module);
        assert!(matches!(
            target.publish(WIRE),
            Err(PublishError::BeforeWrite(_))
        ));
        for path in [&module, &moved] {
            assert_eq!(fs::read_to_string(path.join("targets.txt")).unwrap(), OLD);
        }
    }

    #[test]
    fn refuses_redirected_destinations_and_staging_hardlinks() {
        for redirected in ["destination", "staging", "hardlink"] {
            let dir = tempfile::tempdir().unwrap();
            fixture(dir.path());
            let outside = dir.path().join("outside");
            fs::write(&outside, "untouched").unwrap();
            let destination = if redirected == "destination" {
                dir.path().join("targets.txt")
            } else {
                dir.path()
                    .join(format!("targets.tmp.{}", std::process::id()))
            };
            let _ = fs::remove_file(&destination);
            if redirected == "hardlink" {
                fs::hard_link(&outside, &destination).unwrap();
            } else {
                symlink(&outside, &destination).unwrap();
            }
            let target = ZygiskTarget::open(dir.path()).unwrap();
            assert!(matches!(
                target.publish(WIRE),
                Err(PublishError::BeforeWrite(_))
            ));
            assert_eq!(fs::read_to_string(outside).unwrap(), "untouched");
        }
    }

    #[test]
    fn rejects_invalid_or_whitelist_payload_before_publication() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let target = ZygiskTarget::open(dir.path()).unwrap();
        for wire in [
            "invalid",
            "vpnhide 2 config\ndebug 0\ndefault 40000\nend 0\n",
        ] {
            assert!(matches!(
                target.publish(wire),
                Err(PublishError::BeforeWrite(_))
            ));
            assert_eq!(
                fs::read_to_string(dir.path().join("targets.txt")).unwrap(),
                OLD
            );
        }
    }
}
