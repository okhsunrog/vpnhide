//! LSPosed / Vector: the framework's lines of this boot's log, and a copy of its module
//! scope database for the app's read-only inspection.

use std::path::Path;
use std::time::Duration;

use super::run;

const NEEDLES: &[&str] = &[
    "vpnhide-lsposed",
    "lsposed-bridge",
    "vectornative",
    "vectorbridge",
    "lsposedservice",
    "lsplt",
    "lsphooker",
    "lsposedbridge",
    "xposed",
    "org.lsposed",
    "lspd",
    "modules_config",
    "dev.okhsunrog.vpnhide",
];

/// `logcat -d … | grep -Ei <framework> | tail -2000`.
pub fn boot_log() -> (i32, String) {
    let ran = run(
        "logcat",
        &["-d", "-b", "all", "-v", "threadtime"],
        Duration::from_secs(15),
    );
    let lines: Vec<&str> = ran
        .stdout
        .lines()
        .filter(|l| {
            let lower = l.to_ascii_lowercase();
            NEEDLES.iter().any(|n| lower.contains(n))
        })
        .collect();
    let tail = &lines[lines.len().saturating_sub(2000)..];
    let mut text = tail.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    (0, text)
}

/// Copy `modules_config.db` (+ `-wal` / `-shm` when present) into `out_dir`.
pub fn copy_db(out_dir: &Path) -> (i32, String) {
    copy_db_from(
        Path::new("/data/adb/lspd/config/modules_config.db"),
        out_dir,
    )
}

pub fn copy_db_from(src: &Path, out_dir: &Path) -> (i32, String) {
    let name = |suffix: &str| format!("modules_config.db{suffix}");
    if let Err(e) = std::fs::copy(src, out_dir.join(name(""))) {
        return (1, format!("copy {}: {e}\n", src.display()));
    }
    for suffix in ["-wal", "-shm"] {
        let from = src.with_file_name(name(suffix));
        if from.is_file() {
            let _ = std::fs::copy(&from, out_dir.join(name(suffix)));
        }
    }
    (0, super::ls_dir(out_dir, 10) + "\n")
}
