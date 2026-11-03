//! Fixed sys/proc-sys path lookup observations; no verdict or privileged effects.
use std::{
    ffi::{CStr, CString},
    io,
    os::fd::{FromRawFd, OwnedFd},
};
use vpnhide_diagnostic_model::{
    FilesystemCheck, FilesystemMeasurement, FilesystemObservation, FilesystemVector,
    NativeReadActor, ProbeError, Sample,
};

fn outcome(result: libc::c_long) -> Sample<bool> {
    if result == 0 {
        return Sample::Value(true);
    }
    if result > 0 {
        return Sample::Error(ProbeError::Malformed);
    }
    let errno = io::Error::last_os_error()
        .raw_os_error()
        .filter(|e| *e > 0)
        .unwrap_or(libc::EIO);
    if errno == libc::ENOENT {
        Sample::Value(false)
    } else {
        Sample::Error(ProbeError::Errno(errno))
    }
}
// Linux UAPI include/uapi/linux/stat.h: statx is exactly 256 bytes on both
// ARM ABIs. No raw kernel stat/stat64 layout or private offsets are guessed.
#[repr(C, align(8))]
struct StatxBytes([u8; 256]);
fn lookup(path: &CStr, actor: NativeReadActor, vector: FilesystemVector) -> Sample<bool> {
    match vector {
        FilesystemVector::Stat => {
            let result = unsafe {
                match actor {
                    NativeReadActor::Libc => {
                        let mut value: libc::stat = std::mem::zeroed();
                        libc::stat(path.as_ptr(), &mut value) as libc::c_long
                    }
                    NativeReadActor::Syscall => {
                        let mut value = StatxBytes([0; 256]);
                        libc::syscall(
                            libc::SYS_statx,
                            libc::AT_FDCWD,
                            path.as_ptr(),
                            0,
                            1u32,
                            value.0.as_mut_ptr(),
                        )
                    }
                }
            };
            outcome(result)
        }
        FilesystemVector::Access => outcome(unsafe {
            match actor {
                NativeReadActor::Libc => libc::access(path.as_ptr(), libc::F_OK) as libc::c_long,
                NativeReadActor::Syscall => libc::syscall(
                    libc::SYS_faccessat,
                    libc::AT_FDCWD,
                    path.as_ptr(),
                    libc::F_OK,
                ),
            }
        }),
        FilesystemVector::OpenDirectory => {
            let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NONBLOCK;
            let raw = unsafe {
                match actor {
                    NativeReadActor::Libc => libc::open(path.as_ptr(), flags) as libc::c_long,
                    NativeReadActor::Syscall => {
                        libc::syscall(libc::SYS_openat, libc::AT_FDCWD, path.as_ptr(), flags, 0)
                    }
                }
            };
            if raw < 0 {
                return outcome(raw);
            }
            let _fd = unsafe { OwnedFd::from_raw_fd(raw as libc::c_int) };
            Sample::Value(true)
        }
    }
}
pub fn observe(name: &str) -> FilesystemMeasurement {
    let valid = name.len() == 15
        && name.starts_with("tun")
        && name[3..]
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase());
    FilesystemMeasurement {
        entries: FilesystemCheck::required()
            .into_iter()
            .map(|check| {
                let sample = |name: &str| {
                    if !valid {
                        return Sample::Error(ProbeError::Malformed);
                    }
                    let mut path = crate::sys_interfaces::path(check.surface)
                        .to_bytes()
                        .to_vec();
                    path.push(b'/');
                    path.extend_from_slice(name.as_bytes());
                    match CString::new(path) {
                        Ok(path) => lookup(&path, check.actor, check.vector),
                        Err(_) => Sample::Error(ProbeError::Malformed),
                    }
                };
                FilesystemObservation {
                    check,
                    fixture: sample(name),
                    ordinary: sample("lo"),
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_host_lookup_actors_see_loopback_and_absent_owned_name() {
        assert_eq!(std::mem::size_of::<StatxBytes>(), 256);
        assert_eq!(std::mem::align_of::<StatxBytes>(), 8);
        let measured = observe("tunabcdef123456");
        assert!(measured.bounded());
        assert_eq!(measured.entries.len(), 18);
        for e in measured.entries {
            assert_eq!(e.fixture, Sample::Value(false), "{e:?}");
            assert_eq!(e.ordinary, Sample::Value(true), "{e:?}");
        }
    }
    #[test]
    fn arbitrary_or_malformed_names_never_turn_into_lookup_requests() {
        for name in [
            "../etc/passwd",
            "tun00000000000/",
            "lo",
            "tunABCDEF123456",
            "tun0000000000\0x",
        ] {
            let measured = observe(name);
            assert!(measured.bounded());
            assert!(
                measured
                    .entries
                    .iter()
                    .all(|e| e.fixture == Sample::Error(ProbeError::Malformed)
                        && e.ordinary == Sample::Error(ProbeError::Malformed))
            );
        }
    }
}
