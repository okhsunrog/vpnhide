//! Descriptor-relative, bounded policy reads before specialization. Bundled Core
//! never falls back to a legacy targets file when its authorization is absent.
use std::ffi::CStr;
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::{FromRawFd, RawFd};
use vpnhide_protocol::{Config, hook_ids::ZYGISK_HOOK_MASK, zygisk_activation};

fn read_at(directory: RawFd, name: &CStr, limit: usize) -> io::Result<Vec<u8>> {
    let raw = unsafe {
        libc::openat(
            directory,
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    let file = unsafe { File::from_raw_fd(raw) };
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("policy is not a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::other("policy exceeds budget"));
    }
    Ok(bytes)
}

pub struct Snapshot {
    pub config: Config,
    /// Absent for explicit legacy recipients; never synthesize a bundled generation.
    pub authorization: Option<zygisk_activation::Authorization>,
}

pub fn read_snapshot(directory: RawFd) -> Option<Snapshot> {
    // Missing boot evidence denies bundled mode; legacy compatibility does not use it.
    let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap_or_default();
    read_snapshot_for_boot(directory, boot.trim())
}

#[cfg(test)]
fn read_config_for_boot(directory: RawFd, boot: &str) -> Option<Config> {
    read_snapshot_for_boot(directory, boot).map(|snapshot| snapshot.config)
}

fn read_snapshot_for_boot(directory: RawFd, boot: &str) -> Option<Snapshot> {
    let manifest = read_at(directory, c"module.prop", 8192).ok()?;
    let manifest = std::str::from_utf8(&manifest).ok()?;
    let (mut id, mut activation, mut control) = (None, None, None);
    for line in manifest
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
    {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let slot = match key {
            "id" => &mut id,
            "vpnhideActivation" => &mut activation,
            "vpnhideControl" => &mut control,
            _ => continue,
        };
        if slot.replace(value).is_some() {
            return None;
        }
    }
    match (id, activation, control) {
        (Some("vpnhide_core"), Some("1"), _) => {
            let bytes = read_at(
                directory,
                c"zygisk-activation.txt",
                zygisk_activation::MAX_BYTES,
            )
            .ok()?;
            let authorization = zygisk_activation::Authorization::decode(&bytes, boot)?;
            let config = authorization.policy(ZYGISK_HOOK_MASK)?;
            Some(Snapshot {
                config,
                authorization: Some(authorization),
            })
        }
        // Explicit compatibility boundary for separate legacy module installation.
        // A Core module never consumes this file, even if it is left from an old boot.
        (Some("vpnhide_zygisk"), None, Some("2")) => {
            let bytes = read_at(directory, c"targets.txt", zygisk_activation::MAX_BYTES).ok()?;
            Some(Snapshot {
                config: vpnhide_protocol::parse_config(&bytes)?,
                authorization: None,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::symlink;
    const BOOT: &str = "12345678-1234-1234-1234-123456789abc";
    const WIRE: &str = "vpnhide 2 config\ndebug 0\ndefault 0\nend 0\n";

    fn authorized(root: &std::path::Path, boot: &str, generation: u64, wire: &str) {
        let auth = zygisk_activation::Authorization {
            boot_id: boot.into(),
            generation,
            wire: wire.into(),
        };
        fs::write(
            root.join(zygisk_activation::FILE_NAME),
            auth.encode().unwrap(),
        )
        .unwrap();
    }
    #[test]
    fn consumed_generation_does_not_follow_replacement_or_revocation() {
        let root = tempfile::tempdir().unwrap();
        let dir = File::open(root.path()).unwrap();
        fs::write(
            root.path().join("module.prop"),
            "id=vpnhide_core\nvpnhideActivation=1\n",
        )
        .unwrap();
        authorized(root.path(), BOOT, 7, WIRE);
        let old_process = read_snapshot_for_boot(dir.as_raw_fd(), BOOT).unwrap();
        authorized(root.path(), BOOT, 8, WIRE);
        let new_process = read_snapshot_for_boot(dir.as_raw_fd(), BOOT).unwrap();
        assert_eq!(old_process.authorization.as_ref().unwrap().generation, 7);
        assert_eq!(new_process.authorization.as_ref().unwrap().generation, 8);
        fs::remove_file(root.path().join(zygisk_activation::FILE_NAME)).unwrap();
        assert!(read_snapshot_for_boot(dir.as_raw_fd(), BOOT).is_none());
        assert_eq!(old_process.authorization.unwrap().generation, 7);
        assert_eq!(new_process.authorization.unwrap().generation, 8);
    }

    #[test]
    fn bundled_mode_requires_current_boot_authorization_never_legacy_targets() {
        let root = tempfile::tempdir().unwrap();
        let dir = File::open(root.path()).unwrap();
        fs::write(
            root.path().join("module.prop"),
            "id=vpnhide_core\nvpnhideActivation=1\n",
        )
        .unwrap();
        fs::write(root.path().join("targets.txt"), WIRE).unwrap();
        assert!(read_config_for_boot(dir.as_raw_fd(), BOOT).is_none());
        authorized(root.path(), BOOT, 7, WIRE);
        assert!(read_config_for_boot(dir.as_raw_fd(), BOOT).is_some());
        assert!(
            read_config_for_boot(dir.as_raw_fd(), "87654321-1234-1234-1234-123456789abc").is_none()
        );
        fs::remove_file(root.path().join(zygisk_activation::FILE_NAME)).unwrap();
        assert!(read_config_for_boot(dir.as_raw_fd(), BOOT).is_none());
    }
    #[test]
    fn unknown_manifest_symlink_nonregular_and_oversized_policy_fail_closed() {
        let root = tempfile::tempdir().unwrap();
        let dir = File::open(root.path()).unwrap();
        let manifest = root.path().join("module.prop");
        authorized(root.path(), BOOT, 7, WIRE);
        for text in [
            "id=vpnhide_core\n",
            "id=vpnhide_core\nvpnhideActivation=2\n",
            "id=vpnhide_core\nid=vpnhide_core\nvpnhideActivation=1\n",
        ] {
            fs::write(&manifest, text).unwrap();
            assert!(read_config_for_boot(dir.as_raw_fd(), BOOT).is_none());
        }
        fs::write(&manifest, "id=vpnhide_core\nvpnhideActivation=1\n").unwrap();
        let auth = root.path().join(zygisk_activation::FILE_NAME);
        let old = root.path().join("old");
        fs::rename(&auth, &old).unwrap();
        symlink(&old, &auth).unwrap();
        assert!(read_config_for_boot(dir.as_raw_fd(), BOOT).is_none());
        fs::remove_file(&auth).unwrap();
        fs::create_dir(&auth).unwrap();
        assert!(read_config_for_boot(dir.as_raw_fd(), BOOT).is_none());
        fs::remove_dir(&auth).unwrap();
        fs::write(&auth, vec![b'x'; zygisk_activation::MAX_BYTES + 1]).unwrap();
        assert!(read_config_for_boot(dir.as_raw_fd(), BOOT).is_none());
    }
    #[test]
    fn separate_legacy_module_is_not_confused_with_bundled_mode() {
        let root = tempfile::tempdir().unwrap();
        let dir = File::open(root.path()).unwrap();
        fs::write(
            root.path().join("module.prop"),
            "id=vpnhide_zygisk\nvpnhideControl=2\n",
        )
        .unwrap();
        fs::write(root.path().join("targets.txt"), WIRE).unwrap();
        assert!(read_config_for_boot(dir.as_raw_fd(), BOOT).is_some());
        assert!(read_config_for_boot(dir.as_raw_fd(), "").is_some());
        fs::write(
            root.path().join("module.prop"),
            "id=unknown\nvpnhideControl=2\n",
        )
        .unwrap();
        assert!(read_config_for_boot(dir.as_raw_fd(), BOOT).is_none());
    }
}
