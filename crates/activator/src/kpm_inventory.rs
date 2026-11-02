//! Read-only native KPM inventory. Executed before threads in a bounded child:
//! seccomp/SIGSYS or a vendor ioctl cannot kill or hang the Core writer.
//! SukiSU ABI source: 48b5e405271caf2d6a0fe9cf5c787090c4f9d67e;
//! KPatch-Next: 456744b29efb9989445463ab29e368fa59a103c4.
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    pub runtime_present: bool,
    pub driver_unrecognized: bool,
    pub vpnhide_loaded: bool,
}

pub(crate) fn observe(next: bool) -> crate::Result<Inventory> {
    if cfg!(target_os = "android") && !cfg!(target_arch = "aarch64") {
        // A compat userspace syscall45 is not the ARM64 KernelPatch syscall.
        // This cannot establish absence of a resident ARM64 KPM runtime.
        return Ok(Inventory {
            driver_unrecognized: true,
            ..Default::default()
        });
    }
    if !cfg!(all(target_os = "android", target_arch = "aarch64")) {
        // Host/test engines cannot probe Android syscall numbers.
        return Ok(Inventory::default());
    }
    let out = crate::output_with_timeout(
        std::process::Command::new(std::env::current_exe()?).args([
            "probe",
            "native-kpm-inventory",
            if next { "next" } else { "no-next" },
        ]),
        crate::CHILD_COMMAND_TIMEOUT,
    )?;
    if !out.status.success() || out.stdout.len() > 1024 {
        return Err(format!("native KPM inventory unavailable: {}", out.status).into());
    }
    Ok(serde_json::from_slice(&out.stdout)?)
}

/// Called only by the daemon's early, single-threaded probe dispatcher.
pub fn child(next: bool) -> i32 {
    #[cfg(all(target_os = "android", target_arch = "aarch64"))]
    let result = native::probe(next);
    #[cfg(not(all(target_os = "android", target_arch = "aarch64")))]
    let result: crate::Result<Inventory> = {
        let _ = next;
        Err("native KPM inventory requires Android ARM64".into())
    };
    match result {
        Ok(value) => {
            println!("{}", serde_json::to_string(&value).expect("inventory JSON"));
            0
        }
        Err(error) => {
            eprintln!("native KPM inventory: {error}");
            1
        }
    }
}

/// NUM is read on both sides of LIST. Never turn a truncated/racing list into absence.
pub(crate) fn contains(count: i64, after: i64, list: &[u8]) -> crate::Result<bool> {
    if !(0..=128).contains(&count) || count != after || list.len() >= 4096 {
        return Err("KPM inventory count or size is invalid/unstable".into());
    }
    let text = std::str::from_utf8(list)?;
    let names: Vec<_> = text.split_whitespace().collect();
    let unique: std::collections::HashSet<_> = names.iter().copied().collect();
    if names.len() as i64 != count
        || unique.len() != names.len()
        || names.iter().any(|name| {
            name.is_empty()
                || name.len() >= 32
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
        })
    {
        return Err("KPM inventory is malformed, truncated or changed".into());
    }
    Ok(names.contains(&crate::KPM_NAME))
}

#[cfg(all(target_os = "android", target_arch = "aarch64"))]
mod native {
    use super::{Inventory, contains};
    use crate::Result;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    #[repr(C)]
    #[derive(Default)]
    struct Extension {
        id: u32,
        op: u32,
        supported: u32,
        api_version: u32,
        value: u64,
        user_ptr: u64,
    }
    #[repr(C)]
    #[derive(Default)]
    struct Desc {
        id: u32,
        caps: u32,
        min_api_version: u32,
        forced: u32,
        name: [u8; 32],
    }
    #[repr(C)]
    struct Kpm {
        control_code: u64,
        arg1: u64,
        arg2: u64,
        result_code: u64,
    }
    const EXT: libc::c_int = ((3u32 << 30) | (32 << 16) | (0x4b << 8) | 300) as libc::c_int;
    fn extension(fd: &OwnedFd, command: &mut Extension) -> Result<()> {
        if unsafe { libc::ioctl(fd.as_raw_fd(), EXT, command) } < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
    fn kpm(fd: &OwnedFd, op: u64, arg1: u64, arg2: u64) -> Result<i32> {
        let mut result = i32::MIN;
        let mut command = Kpm {
            control_code: op,
            arg1,
            arg2,
            result_code: (&raw mut result) as u64,
        };
        extension(
            fd,
            &mut Extension {
                op: 3,
                api_version: 2,
                user_ptr: (&raw mut command) as u64,
                ..Default::default()
            },
        )?;
        if result < 0 {
            return Err(format!("SukiSU KPM operation {op} failed: {result}").into());
        }
        Ok(result)
    }
    fn ctext(buf: &[u8]) -> Result<&[u8]> {
        let end = buf
            .iter()
            .position(|&b| b == 0)
            .ok_or("unterminated KPM output")?;
        Ok(&buf[..end])
    }
    fn suki() -> Result<Inventory> {
        let mut raw_fd: i32 = -1;
        // These are install-driver cookies, NOT Linux reboot magic. On an ordinary
        // kernel the invalid magic returns EINVAL and cannot reboot the device.
        let rc = unsafe {
            libc::syscall(
                libc::SYS_reboot,
                0xdead_beefu32 as libc::c_ulong,
                0xcafe_babeu32 as libc::c_ulong,
                0,
                &raw mut raw_fd,
            )
        };
        if rc != 0 || raw_fd < 0 {
            let error = std::io::Error::last_os_error();
            return if rc < 0 && error.raw_os_error() == Some(libc::EINVAL) {
                Ok(Inventory::default())
            } else {
                Err(format!("KSU driver discovery unavailable: {error}").into())
            };
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };
        if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut info = [0u32; 4];
        if unsafe { libc::ioctl(fd.as_raw_fd(), 0x8010_4b02u32 as libc::c_int, &raw mut info) } < 0
            || info[0] == 0
            || info[3] == 0
        {
            return Ok(Inventory {
                driver_unrecognized: true,
                ..Default::default()
            });
        }
        let mut abi = Extension {
            id: u32::MAX,
            op: 5,
            ..Default::default()
        };
        if extension(&fd, &mut abi).is_err() || abi.api_version != 2 {
            return Ok(Inventory {
                driver_unrecognized: true,
                ..Default::default()
            });
        }
        let mut found = None;
        let mut ended = false;
        for index in 0..32u64 {
            let mut desc = Desc::default();
            let mut cmd = Extension {
                id: u32::MAX,
                op: 6,
                api_version: 2,
                value: index,
                user_ptr: (&raw mut desc) as u64,
                ..Default::default()
            };
            match extension(&fd, &mut cmd) {
                Ok(()) => {
                    if desc.id == 0 {
                        if ctext(&desc.name)? != b"kpm"
                            || desc.caps & 1 == 0
                            || desc.min_api_version > 2
                        {
                            return Err("unrecognized KSU KPM handler".into());
                        }
                        if found.replace(desc).is_some() {
                            return Err("duplicate KPM handler".into());
                        }
                    }
                }
                Err(error)
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|e| e.raw_os_error() == Some(libc::ENOENT)) =>
                {
                    ended = true;
                    break;
                }
                Err(error) => return Err(error),
            }
        }
        if !ended {
            return Err("KSU handler enumeration exceeded bound".into());
        }
        if found.is_none() {
            return Ok(Inventory::default());
        }
        let mut version = [0u8; 256];
        kpm(&fd, 7, version.as_mut_ptr() as u64, version.len() as u64)?;
        if ctext(&version)?.is_empty() {
            return Err("empty SukiSU KPM version".into());
        }
        let count = kpm(&fd, 3, 0, 0)? as i64;
        let mut list = [0u8; 1024];
        let length = kpm(&fd, 4, list.as_mut_ptr() as u64, list.len() as u64)?;
        let list = ctext(&list)?;
        if length as usize != list.len() {
            return Err("SukiSU KPM LIST length mismatch".into());
        }
        let after = kpm(&fd, 3, 0, 0)? as i64;
        Ok(Inventory {
            runtime_present: true,
            driver_unrecognized: false,
            vpnhide_loaded: contains(count, after, list)?,
        })
    }
    fn next() -> Result<Inventory> {
        let command = |op: u64| (0x000d05u64 << 32) | (0x2026 << 16) | op;
        let call = |op: u64, ptr: *mut u8, size: usize| unsafe {
            libc::syscall(45, std::ptr::null::<u8>(), command(op), ptr, size)
        };
        let hello = call(0x1000, std::ptr::null_mut(), 0);
        if hello != 0x2026_2026 {
            return if hello < 0
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::EFAULT)
            {
                Ok(Inventory::default())
            } else {
                Err("unrecognized null-key KernelPatch handshake".into())
            };
        }
        let version = call(0x1008, std::ptr::null_mut(), 0);
        if !(0x0a05..=0x00ff_ffff).contains(&version) {
            return Err("invalid KernelPatch version".into());
        }
        let count = call(0x1030, std::ptr::null_mut(), 0);
        let mut list = [0u8; 4096];
        let length = call(0x1031, list.as_mut_ptr(), list.len());
        if length < 0 {
            return Err("KernelPatch LIST failed".into());
        }
        let list = ctext(&list)?;
        // Pinned Next returns the copied buffer extent, not the string length.
        if length != 4096 {
            return Err("KernelPatch LIST copy was incomplete".into());
        }
        let after = call(0x1030, std::ptr::null_mut(), 0);
        Ok(Inventory {
            runtime_present: true,
            driver_unrecognized: false,
            vpnhide_loaded: contains(count, after, list)?,
        })
    }
    pub(super) fn probe(include_next: bool) -> Result<Inventory> {
        if unsafe { libc::geteuid() } != 0 {
            return Err("root required".into());
        }
        let suki = suki()?;
        if !include_next {
            return Ok(suki);
        }
        let next = next()?;
        Ok(Inventory {
            runtime_present: suki.runtime_present || next.runtime_present,
            driver_unrecognized: suki.driver_unrecognized,
            vpnhide_loaded: suki.vpnhide_loaded || next.vpnhide_loaded,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::contains;
    #[test]
    fn enumeration_requires_stable_counts_and_complete_distinct_names() {
        assert!(!contains(0, 0, b"").unwrap());
        assert!(contains(2, 2, b"other\nvpnhide\n").unwrap());
        assert!(!contains(1, 1, b"vpnhide_other\n").unwrap());
        for (before, after, list) in [
            (1, 1, b"".as_slice()),
            (-1, -1, b""),
            (1, 2, b"vpnhide"),
            (2, 2, b"vpnhide vpnhide"),
            (1, 1, b"error: denied"),
            (129, 129, b"vpnhide"),
        ] {
            assert!(contains(before, after, list).is_err());
        }
    }
}
