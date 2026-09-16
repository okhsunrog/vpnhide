//! Privileged, read-only observations owned by `vhhelper`.
//!
//! These paths never acquire the mutation lane or write canonical/runtime
//! state. Their payloads use the shared app/helper observation envelope from
//! `vpnhide_checks`.

use std::ffi::CString;
use std::io::ErrorKind;
use std::os::raw::{c_long, c_void};
use std::{fs, process};
use vpnhide_apatch_abi::{
    APATCH_SUPERCALL_NR, command_candidates, encode_command, parse_kernel_version_hint,
};
use vpnhide_checks::observation;

const SUPERKEY_FILE: &str = "/data/adb/vpnhide/superkey";
const MAX_KPM_LIST_BYTES: usize = 4096;
const KPATCH_CANDIDATES: &[&str] = &[
    "kpatch",
    "/data/adb/modules/KPatch-Next/bin/kpatch",
    "/data/adb/modules/kpatch-next/bin/kpatch",
];
// i64, not c_long: the supercall command word is a 64-bit encoding shared with
// `vpnhide_apatch_abi::encode_command` (version << 32 | magic | cmd). c_long is
// 32-bit on armv7, which would both truncate the encoding and fail to typecheck
// against the i64 API; on arm64 c_long is already i64, so behaviour is unchanged.
const SUPERCALL_KPM_LIST: i64 = 0x1031;
const SUPERCALL_KPM_NUMS: i64 = 0x1030;

pub fn kpm_list(args: &[String]) -> i32 {
    if !args.is_empty() {
        eprintln!("usage: vhhelper observe kpm-list");
        return 2;
    }
    println!("{}", kpm_list_response(read_kpm_list()));
    0
}

#[derive(Debug, PartialEq, Eq)]
enum KpmListRead {
    Available { raw: Vec<u8>, expected_count: usize },
    Unavailable,
    Malformed,
}

#[derive(serde::Serialize)]
struct KpmListData {
    available: bool,
    modules: Vec<String>,
}

fn read_kpm_list() -> KpmListRead {
    kpatch_kpm_list().unwrap_or_else(apatch_kpm_list)
}

/// KPatch-Next's `kpatch kpm list` prints the exact KernelPatch list buffer:
/// module names separated by newlines, with no heading or status columns.
fn kpatch_kpm_list() -> Option<KpmListRead> {
    for candidate in KPATCH_CANDIDATES {
        let list_output = match process::Command::new(candidate)
            .args(["kpm", "list"])
            .output()
        {
            Ok(output) => output,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(_) => return Some(KpmListRead::Unavailable),
        };
        if !list_output.status.success() {
            return Some(KpmListRead::Unavailable);
        }
        if list_output.stdout.len() >= MAX_KPM_LIST_BYTES - 1 {
            return Some(KpmListRead::Malformed);
        }
        let count_output = match process::Command::new(candidate)
            .args(["kpm", "num"])
            .output()
        {
            Ok(output) if output.status.success() => output,
            Ok(_) | Err(_) => return Some(KpmListRead::Unavailable),
        };
        let Some(expected_count) = parse_module_count(&count_output.stdout) else {
            return Some(KpmListRead::Malformed);
        };
        return Some(KpmListRead::Available {
            raw: list_output.stdout,
            expected_count,
        });
    }
    None
}

fn apatch_kpm_list() -> KpmListRead {
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
        let Ok(key) = CString::new(key) else {
            continue;
        };
        for &style in &commands {
            let count = unsafe {
                libc::syscall(
                    APATCH_SUPERCALL_NR as c_long,
                    key.as_ptr(),
                    encode_command(style, SUPERCALL_KPM_NUMS),
                )
            };
            let Ok(expected_count) = usize::try_from(count) else {
                continue;
            };
            if let Some(read) = read_apatch_modules(expected_count, |buffer| {
                let buffer_len =
                    c_long::try_from(buffer.len()).expect("KPM list buffer fits c_long");
                unsafe {
                    libc::syscall(
                        APATCH_SUPERCALL_NR as c_long,
                        key.as_ptr(),
                        encode_command(style, SUPERCALL_KPM_LIST),
                        buffer.as_mut_ptr().cast::<c_void>(),
                        buffer_len,
                    )
                }
            }) {
                return read;
            }
        }
    }
    KpmListRead::Unavailable
}

/// A successful count of zero is already an empty observation. Older
/// KernelPatch list calls copy an uninitialized kernel buffer in this case.
/// Count and list are separate observations, not an atomic snapshot.
fn read_apatch_modules(
    expected_count: usize,
    list: impl FnOnce(&mut [u8]) -> c_long,
) -> Option<KpmListRead> {
    if expected_count == 0 {
        return Some(KpmListRead::Available {
            raw: Vec::new(),
            expected_count,
        });
    }
    let mut buffer = [0_u8; MAX_KPM_LIST_BYTES];
    if list(&mut buffer) < 0 {
        return None; // Let the caller try the next authentication/ABI candidate.
    }
    // Nonempty upstream lists are NUL-terminated. The return value may be a
    // copy length rather than a string length, so inspect the buffer itself.
    let Some(length) = buffer.iter().position(|byte| *byte == 0) else {
        return Some(KpmListRead::Malformed);
    };
    Some(KpmListRead::Available {
        raw: buffer[..length].to_vec(),
        expected_count,
    })
}

fn kpm_list_response(read: KpmListRead) -> String {
    match read {
        KpmListRead::Available {
            raw,
            expected_count,
        } => match parse_module_names(&raw) {
            Ok(modules) if modules.len() == expected_count => observation::kpm_list(&KpmListData {
                available: true,
                modules,
            }),
            Ok(_) | Err(()) => observation::kpm_list_error("malformed"),
        },
        KpmListRead::Unavailable => observation::kpm_list_error("unavailable"),
        KpmListRead::Malformed => observation::kpm_list_error("malformed"),
    }
}

fn parse_module_count(raw: &[u8]) -> Option<usize> {
    let text = std::str::from_utf8(raw).ok()?;
    let digits = text.strip_suffix('\n').unwrap_or(text);
    (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| digits.parse().ok())
        .flatten()
}

fn parse_module_names(raw: &[u8]) -> Result<Vec<String>, ()> {
    let text = std::str::from_utf8(raw).map_err(|_| ())?;
    if text.is_empty() {
        return Ok(Vec::new());
    }

    let mut modules = Vec::new();
    for name in text.split('\n') {
        let safe = !name.is_empty()
            && !name.bytes().all(|byte| byte.is_ascii_digit())
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'));
        if !safe || modules.iter().any(|existing| existing == name) {
            return Err(());
        }
        modules.push(name.to_owned());
    }
    Ok(modules)
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
    use serde_json::Value;

    fn fixture(name: &str) -> Value {
        let raw = match name {
            "kpm-empty.json" => include_str!("../../../fixtures/app-helper/kpm-empty.json"),
            "kpm-vpnhide.json" => include_str!("../../../fixtures/app-helper/kpm-vpnhide.json"),
            "kpm-other.json" => include_str!("../../../fixtures/app-helper/kpm-other.json"),
            "kpm-next.json" => include_str!("../../../fixtures/app-helper/kpm-next.json"),
            "kpm-unavailable.json" => {
                include_str!("../../../fixtures/app-helper/kpm-unavailable.json")
            }
            "kpm-malformed.json" => {
                include_str!("../../../fixtures/app-helper/kpm-malformed.json")
            }
            _ => panic!("unknown fixture"),
        };
        serde_json::from_str(raw).unwrap()
    }

    fn response(read: KpmListRead) -> Value {
        serde_json::from_str(&kpm_list_response(read)).unwrap()
    }

    #[test]
    fn zero_apatch_count_never_enumerates_and_matches_empty_fixture() {
        let read = read_apatch_modules(0, |_| panic!("empty count must not call KPM_LIST"));
        assert_eq!(response(read.unwrap()), fixture("kpm-empty.json"));
    }

    #[test]
    fn nonzero_apatch_count_enumerates_and_preserves_failures() {
        let read = read_apatch_modules(1, |buffer| {
            buffer[..8].copy_from_slice(b"vpnhide\0");
            buffer.len() as c_long
        });
        assert_eq!(response(read.unwrap()), fixture("kpm-vpnhide.json"));
        assert_eq!(read_apatch_modules(1, |_| -1), None);
        assert_eq!(
            read_apatch_modules(1, |buffer| {
                buffer.fill(b'x');
                buffer.len() as c_long
            }),
            Some(KpmListRead::Malformed)
        );
        // A concurrent unload must not turn an expected nonempty list into a
        // successful empty observation.
        assert_eq!(
            response(read_apatch_modules(1, |_| 0).unwrap()),
            fixture("kpm-malformed.json")
        );
    }

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

    #[test]
    fn parses_only_the_upstream_newline_separated_name_format() {
        assert_eq!(parse_module_names(b""), Ok(Vec::new()));
        assert_eq!(
            parse_module_names(b"vpnhide\nother_module"),
            Ok(vec!["vpnhide".to_owned(), "other_module".to_owned()])
        );
        for malformed in [
            b"123".as_slice(),
            b"error: unsupported output".as_slice(),
            b"vpnhide\n".as_slice(),
            b"vpnhide\n\nother".as_slice(),
            b"vpnhide\nvpnhide".as_slice(),
            b"vpnhide\0other".as_slice(),
            b"\xff".as_slice(),
        ] {
            assert_eq!(parse_module_names(malformed), Err(()));
        }
        assert_eq!(parse_module_count(b"0\n"), Some(0));
        assert_eq!(parse_module_count(b"2"), Some(2));
        assert_eq!(parse_module_count(b"2\nextra"), None);
    }

    #[test]
    fn rust_kpm_producer_matches_shared_fixtures() {
        for (raw, name) in [
            (b"".as_slice(), "kpm-empty.json"),
            (b"vpnhide".as_slice(), "kpm-vpnhide.json"),
            (b"other_module".as_slice(), "kpm-other.json"),
            (b"vpnhide_next".as_slice(), "kpm-next.json"),
        ] {
            assert_eq!(
                response(KpmListRead::Available {
                    raw: raw.to_vec(),
                    expected_count: usize::from(!raw.is_empty()),
                }),
                fixture(name)
            );
        }
        assert_eq!(
            response(KpmListRead::Unavailable),
            fixture("kpm-unavailable.json")
        );
        assert_eq!(
            response(KpmListRead::Available {
                raw: b"error: unsupported output".to_vec(),
                expected_count: 1,
            }),
            fixture("kpm-malformed.json")
        );
        assert_eq!(
            response(KpmListRead::Available {
                raw: b"vpnhide".to_vec(),
                expected_count: 2,
            }),
            fixture("kpm-malformed.json")
        );
        assert_eq!(
            response(KpmListRead::Malformed),
            fixture("kpm-malformed.json")
        );
    }
}
