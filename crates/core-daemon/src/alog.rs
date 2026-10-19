//! Outcome logging: every boot-selection and apply outcome goes to core's stderr log
//! (`core.log`) and, on Android, to logcat under one tag — what each activator's
//! `log_android` used to do.

use std::path::Path;
use std::process::{Command, Stdio};

pub const TAG: &str = "vpnhide_core";
const LOG_BIN: &str = "/system/bin/log";

/// Log `message` to stderr and, when the Android `log` tool exists, to logcat.
pub fn info(message: &str) {
    eprintln!("core: {message}");
    if Path::new(LOG_BIN).is_file() {
        let _ = Command::new(LOG_BIN)
            .args(["-t", TAG, message])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}
