//! Select an installed companion from live kernel identity, never from wire text
//! in the app shell. Execution remains a child of the app's mutation supervisor.
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{BUILTIN_MODULE_DIR, KMOD_CTL, KMOD_MODULE_DIR, Result, kernel_control};
use vpnhide_protocol::hook_ids::Backend;

fn companion(dir: &Path) -> bool {
    dir.join("module.prop").is_file() && !dir.join("disable").exists()
}

fn select(ctl: &Path, modules: &[PathBuf; 4]) -> Result<Option<PathBuf>> {
    let owner = kernel_control::observed_backend(ctl)?;
    let required = match owner {
        Some(id) if id == Backend::Kmod as u32 => Some(0),
        Some(id) if id == Backend::Builtin as u32 => Some(1),
        Some(_) => return Err("unknown live kernel backend; activation refused".into()),
        None => None,
    };
    if let Some(index) = required {
        if !companion(&modules[index]) {
            return Err(format!(
                "live kernel requires enabled companion at {}",
                modules[index].display()
            )
            .into());
        }
        return Ok(Some(modules[index].join("activator")));
    }
    Ok(modules
        .iter()
        .find(|dir| companion(dir))
        .map(|dir| dir.join("activator")))
}

/// Do not inline module activation: companions ship independently of the APK.
/// Their exit status and output remain visible to the mutation supervisor.
pub fn activate_native_companion() -> Result<i32> {
    let modules = [
        KMOD_MODULE_DIR,
        BUILTIN_MODULE_DIR,
        "/data/adb/modules/vpnhide_kpm",
        "/data/adb/modules/vpnhide_zygisk",
    ]
    .map(PathBuf::from);
    let Some(executable) = select(Path::new(KMOD_CTL), &modules)? else {
        return Ok(0);
    };
    let status = Command::new(executable).status()?;
    Ok(status.code().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use vpnhide_protocol::{Status, format_status};

    #[test]
    fn live_owner_precedes_companion_priority_and_refuses_unavailable_owner() {
        let root = std::env::temp_dir().join(format!("vh-selection-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let ctl = root.join("ctl");
        let modules = ["kmod", "builtin", "kpm", "zygisk"].map(|name| root.join(name));
        for dir in &modules {
            fs::create_dir_all(dir).unwrap();
            fs::write(dir.join("module.prop"), "id=test").unwrap();
        }
        assert_eq!(
            select(&ctl, &modules).unwrap(),
            Some(modules[0].join("activator"))
        );
        for (backend, index) in [(Backend::Builtin, 1), (Backend::Kmod, 0)] {
            fs::write(
                &ctl,
                format_status(&Status {
                    backend: backend as u32,
                    kver: 0,
                    hooks: 0,
                    error: 0,
                }),
            )
            .unwrap();
            assert_eq!(
                select(&ctl, &modules).unwrap(),
                Some(modules[index].join("activator"))
            );
            fs::write(modules[index].join("disable"), "").unwrap();
            assert!(select(&ctl, &modules).is_err());
            fs::remove_file(modules[index].join("disable")).unwrap();
            fs::remove_file(modules[index].join("module.prop")).unwrap();
            assert!(select(&ctl, &modules).is_err());
            fs::write(modules[index].join("module.prop"), "id=test").unwrap();
        }
        for invalid in [
            "garbage",
            "vpnhide 1 status\nbackend 0x4\n",
            "vpnhide 1 status\nbackend 0xff\nkver 0x0\nhooks 0x0\nerror 0x0\n",
        ] {
            fs::write(&ctl, invalid).unwrap();
            assert!(select(&ctl, &modules).is_err());
        }
        fs::remove_dir_all(root).unwrap();
    }
}
