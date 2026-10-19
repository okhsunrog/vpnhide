//! Legacy-writer fencing markers shared with core (core-2.0 §11).
//!
//! When a marker is present, activator entry points no-op instead of competing
//! with core. Markers are claimed by core before its first write on that
//! resource. Dev sandboxes leave them unset unless tests opt in via env.

use std::path::PathBuf;

use crate::{
    CONFIG_CORE_OWNER_MARKER, CORE_OWNER_DIR_ENV, NATIVE_CORE_OWNER_MARKER,
    ZYGISK_CORE_OWNER_MARKER, ports_owned_by_core,
};

fn owner_dir() -> PathBuf {
    match std::env::var_os(CORE_OWNER_DIR_ENV) {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => PathBuf::from("/data/adb/vpnhide/core"),
    }
}

fn marker_in_dir(default_path: &str, file_name: &str) -> PathBuf {
    // If VPNHIDE_CORE_OWNER_DIR is set, all non-Ports markers live there.
    // Ports keeps its own env override via ports_owner_marker_path.
    if std::env::var_os(CORE_OWNER_DIR_ENV).is_some() {
        owner_dir().join(file_name)
    } else {
        PathBuf::from(default_path)
    }
}

pub fn config_owned_by_core() -> bool {
    marker_in_dir(CONFIG_CORE_OWNER_MARKER, "config_owned").is_file()
}

pub fn native_owned_by_core() -> bool {
    marker_in_dir(NATIVE_CORE_OWNER_MARKER, "native_owned").is_file()
}

pub fn zygisk_owned_by_core() -> bool {
    marker_in_dir(ZYGISK_CORE_OWNER_MARKER, "zygisk_owned").is_file()
}

pub fn activation_fenced_by_core(kind: &str) -> bool {
    match kind {
        "native" | "kmod" | "builtin" | "kpm" => native_owned_by_core(),
        "zygisk" => zygisk_owned_by_core(),
        "ports" => ports_owned_by_core(),
        "config" => config_owned_by_core(),
        _ => false,
    }
}

pub fn log_fenced(tag: &str, action: &str) {
    let msg = format!("{action} skipped: core owns this resource");
    let _ = std::process::Command::new("log")
        .args(["-t", tag, &msg])
        .status();
    eprintln!("vpnhide {tag}: {msg}");
}

/// Test helper: path core should write for a marker file name.
pub fn core_marker_path(file_name: &str) -> PathBuf {
    owner_dir().join(file_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn native_fence_detects_marker() {
        let _g = LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!(
            "vpnhide-own-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: LOCK held.
        unsafe { std::env::set_var(crate::CORE_OWNER_DIR_ENV, &dir) };
        assert!(!native_owned_by_core());
        std::fs::write(dir.join("native_owned"), b"owned-by-core\n").unwrap();
        assert!(native_owned_by_core());
        assert!(activation_fenced_by_core("kmod"));
        assert!(activation_fenced_by_core("builtin"));
        assert!(!activation_fenced_by_core("zygisk"));
        unsafe { std::env::remove_var(crate::CORE_OWNER_DIR_ENV) };
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn activate_kmod_noops_when_core_owns_native() {
        let _g = LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!(
            "vpnhide-own-kmod-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var(crate::CORE_OWNER_DIR_ENV, &dir) };
        std::fs::write(dir.join("native_owned"), b"owned\n").unwrap();
        // Would hang/fail without fence if it tried real activation.
        crate::activate_kmod().unwrap();
        unsafe { std::env::remove_var(crate::CORE_OWNER_DIR_ENV) };
        let _ = std::fs::remove_dir_all(&dir);
    }
}
