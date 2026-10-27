//! Privileged, read-only observations (`vpnhide-core observe kpm-state`).
//!
//! These paths never acquire the mutation lane or write canonical/runtime state.

const DEFAULT_KPM_LOCK: &str = "/data/adb/vpnhide_kpm/ctl.lock";

/// The vpnhide KPM's status reply plus stats pages (the `kpm_state` section).
/// Prints nothing when no KernelPatch runtime exists or vpnhide is not loaded;
/// a transport or authentication error exits 1 with the reason on stderr.
pub(crate) fn collect_kpm_state() -> crate::device_probes::Ran {
    use crate::device_probes::Ran;
    let lock = crate::backend::kpm_lock_from_env()
        .unwrap_or_else(|| std::path::PathBuf::from(DEFAULT_KPM_LOCK));
    let state = vpnhide_activator::CoreKpm::connect(&lock)
        .and_then(|kpm| kpm.map(|kpm| kpm.read_state()).transpose())
        .map(Option::flatten);
    match state {
        Ok(text) => Ran {
            exit: 0,
            stdout: text.unwrap_or_default(),
            stderr: String::new(),
        },
        Err(error) => Ran {
            exit: 1,
            stdout: String::new(),
            stderr: format!("KPM state unavailable: {error}\n"),
        },
    }
}

pub fn kpm_state() -> i32 {
    let result = collect_kpm_state();
    print!("{}", result.stdout);
    eprint!("{}", result.stderr);
    result.exit
}
