//! Core inventory markers (paths under `/data/adb/vpnhide/core/`).
//!
//! These files record that core has claimed a resource for its own diagnostics /
//! inventory. They are **not** a dual-writer fencing protocol — legacy activator
//! binaries no longer exist. Only core-daemon writes backends.

use std::path::PathBuf;

use crate::{
    CONFIG_CORE_OWNER_MARKER, CORE_OWNER_DIR_ENV, NATIVE_CORE_OWNER_MARKER,
    ZYGISK_CORE_OWNER_MARKER,
};

fn owner_dir() -> PathBuf {
    match std::env::var_os(CORE_OWNER_DIR_ENV) {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => PathBuf::from("/data/adb/vpnhide/core"),
    }
}

fn marker_in_dir(default_path: &str, file_name: &str) -> PathBuf {
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

/// Test/diagnostic helper: path core writes for a marker file name.
pub fn core_marker_path(file_name: &str) -> PathBuf {
    owner_dir().join(file_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn native_marker_detects_file() {
        let _g = LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!(
            "vpnhide-own-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var(crate::CORE_OWNER_DIR_ENV, &dir) };
        assert!(!native_owned_by_core());
        std::fs::write(dir.join("native_owned"), b"owned-by-core\n").unwrap();
        assert!(native_owned_by_core());
        assert!(!zygisk_owned_by_core());
        unsafe { std::env::remove_var(crate::CORE_OWNER_DIR_ENV) };
        let _ = std::fs::remove_dir_all(&dir);
    }
}
