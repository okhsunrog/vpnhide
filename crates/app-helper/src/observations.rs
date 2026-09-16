//! Privileged, read-only observations owned by `vhhelper`.
//!
//! These paths never acquire the mutation lane or write canonical/runtime
//! state. Their payloads use the shared app/helper observation envelope from
//! `vpnhide_checks`.

use std::ffi::CString;
use std::os::raw::{c_long, c_void};
use std::{fs, process};
use vpnhide_apatch_abi::{
    APATCH_SUPERCALL_NR, command_candidates, encode_command, parse_kernel_version_hint,
};
use vpnhide_checks::observation;

const SUPERKEY_FILE: &str = "/data/adb/vpnhide/superkey";
// i64, not c_long: the supercall command word is a 64-bit encoding shared with
// `vpnhide_apatch_abi::encode_command` (version << 32 | magic | cmd). c_long is
// 32-bit on armv7, which would both truncate the encoding and fail to typecheck
// against the i64 API; on arm64 c_long is already i64, so behaviour is unchanged.
const SUPERCALL_KPM_LIST: i64 = 0x1031;

pub fn kpm_list(args: &[String]) -> i32 {
    if !args.is_empty() {
        eprintln!("usage: vhhelper observe kpm-list");
        return 2;
    }
    println!("{}", observation::kpm_list(apatch_kpm_list().as_deref()));
    0
}

fn apatch_kpm_list() -> Option<String> {
    let mut keys = Vec::new();
    if let Ok(saved) = fs::read_to_string(SUPERKEY_FILE) {
        let saved = saved.trim();
        if !saved.is_empty() {
            keys.push(saved.to_owned());
        }
    }
    if !keys.iter().any(|key| key == "su") {
        keys.push("su".to_owned());
    }

    let commands = command_candidates(apatch_kernel_version_hint());
    for key in keys {
        let key = CString::new(key).ok()?;
        for &style in &commands {
            let mut buffer = [0_u8; 4096];
            let buffer_len = c_long::try_from(buffer.len()).expect("KPM list buffer fits c_long");
            let rc = unsafe {
                libc::syscall(
                    APATCH_SUPERCALL_NR as c_long,
                    key.as_ptr(),
                    encode_command(style, SUPERCALL_KPM_LIST),
                    buffer.as_mut_ptr().cast::<c_void>(),
                    buffer_len,
                )
            };
            if rc >= 0 {
                let length = if rc > 0 {
                    usize::try_from(rc)
                        .unwrap_or(buffer.len())
                        .min(buffer.len())
                } else {
                    buffer.iter().position(|byte| *byte == 0).unwrap_or(0)
                };
                return Some(String::from_utf8_lossy(&buffer[..length]).into_owned());
            }
        }
    }
    None
}

fn apatch_kernel_version_hint() -> Option<i64> {
    let output = process::Command::new("dmesg").output().ok()?;
    output
        .status
        .success()
        .then(|| parse_kernel_version_hint(&String::from_utf8_lossy(&output.stdout)))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_arguments_before_attempting_a_privileged_observation() {
        assert_eq!(kpm_list(&["extra".to_owned()]), 2);
    }

    #[test]
    fn parses_latest_apatch_version_hint_from_dmesg() {
        let log =
            "old\nKP KernelPatch Version: 000d02\nnoise\nKP KernelPatch Version: 000d03-extra\n";
        assert_eq!(parse_kernel_version_hint(log), Some(0x0000_0d03));
    }
}
