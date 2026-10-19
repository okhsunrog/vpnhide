//! Outcome logging: every boot-selection and apply outcome goes to core's stderr log
//! (`core.log`) and, on Android, to logcat under one tag (liblog, not the `log` tool) —
//! what each activator's `log_android` used to do.

pub const TAG: &str = "vpnhide_core";

/// Log `message` to stderr and, on Android, to logcat.
pub fn info(message: &str) {
    eprintln!("core: {message}");
    vpnhide_activator::sys::android_log(TAG, message);
}
