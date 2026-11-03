//! Closed-set real directory readers. No root operation or caller-selected path.
use std::{
    collections::BTreeSet,
    ffi::CStr,
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    time::{Duration, Instant},
};
use vpnhide_diagnostic_model::{
    MAX_INTERFACES, NativeReadActor, ProbeError, Sample, SysInterfaceCheck,
    SysInterfaceMeasurement, SysInterfaceObservation, SysInterfaceSurface,
};

pub(crate) fn path(surface: SysInterfaceSurface) -> &'static CStr {
    match surface {
        SysInterfaceSurface::SysClassNet => c"/sys/class/net",
        SysInterfaceSurface::ProcIpv4Conf => c"/proc/sys/net/ipv4/conf",
        SysInterfaceSurface::ProcIpv6Conf => c"/proc/sys/net/ipv6/conf",
    }
}
fn error() -> ProbeError {
    ProbeError::Errno(
        io::Error::last_os_error()
            .raw_os_error()
            .filter(|e| *e > 0)
            .unwrap_or(libc::EIO),
    )
}
fn insert(names: &mut BTreeSet<String>, bytes: &[u8]) -> Result<(), ProbeError> {
    if matches!(bytes, b"." | b"..") {
        return Ok(());
    }
    if bytes.is_empty()
        || bytes.len() > 15
        || !bytes.is_ascii()
        || bytes.iter().any(|b| b.is_ascii_control() || *b == b'/')
    {
        return Err(ProbeError::Malformed);
    }
    let name = std::str::from_utf8(bytes).map_err(|_| ProbeError::Malformed)?;
    if !names.insert(name.to_owned()) {
        return Err(ProbeError::Malformed);
    }
    if names.len() > MAX_INTERFACES + 2 {
        return Err(ProbeError::TooLarge);
    }
    Ok(())
}
// Linux getdents64 has a fixed wire ABI on ARM32 and ARM64: ino64/off64,
// reclen16, type8, then the terminated name. Never cast it to libc::dirent.
fn consume(names: &mut BTreeSet<String>, mut bytes: &[u8]) -> Result<(), ProbeError> {
    while !bytes.is_empty() {
        if bytes.len() < 20 {
            return Err(ProbeError::Malformed);
        }
        let size = usize::from(u16::from_ne_bytes([bytes[16], bytes[17]]));
        if size < 20 || size % 8 != 0 || size > bytes.len() {
            return Err(ProbeError::Malformed);
        }
        let body = &bytes[19..size];
        let end = body
            .iter()
            .position(|b| *b == 0)
            .ok_or(ProbeError::Malformed)?;
        insert(names, &body[..end])?;
        bytes = &bytes[size..];
    }
    Ok(())
}
fn direct(path: &CStr) -> Result<Vec<String>, ProbeError> {
    let fd = unsafe {
        libc::syscall(
            libc::SYS_openat,
            libc::AT_FDCWD,
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NONBLOCK,
            0,
        )
    };
    if fd < 0 {
        return Err(error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd as libc::c_int) };
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut names = BTreeSet::new();
    let mut bytes = [0u8; 4096];
    for _ in 0..64 {
        if Instant::now() >= deadline {
            return Err(ProbeError::Timeout);
        }
        let size = unsafe {
            libc::syscall(
                libc::SYS_getdents64,
                fd.as_raw_fd(),
                bytes.as_mut_ptr(),
                bytes.len(),
            )
        };
        if size < 0 {
            if io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(error());
        }
        if size == 0 {
            return Ok(names.into_iter().collect());
        }
        let batch = bytes.get(..size as usize).ok_or(ProbeError::Malformed)?;
        consume(&mut names, batch)?;
    }
    Err(ProbeError::TooLarge)
}
fn clear_errno() {
    // readdir's null return distinguishes EOF from failure only via errno.
    unsafe {
        #[cfg(target_os = "android")]
        {
            *libc::__errno() = 0;
        }
        #[cfg(not(target_os = "android"))]
        {
            *libc::__errno_location() = 0;
        }
    }
}
fn library(path: &CStr) -> Result<Vec<String>, ProbeError> {
    let dir = unsafe { libc::opendir(path.as_ptr()) };
    if dir.is_null() {
        return Err(error());
    }
    struct Directory(*mut libc::DIR);
    impl Drop for Directory {
        fn drop(&mut self) {
            unsafe {
                libc::closedir(self.0);
            }
        }
    }
    let _owner = Directory(dir);
    let flags = unsafe { libc::fcntl(libc::dirfd(dir), libc::F_GETFD) };
    if flags < 0 {
        return Err(error());
    }
    if flags & libc::FD_CLOEXEC == 0 {
        return Err(ProbeError::Malformed);
    }
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut names = BTreeSet::new();
    for _ in 0..MAX_INTERFACES + 5 {
        if Instant::now() >= deadline {
            return Err(ProbeError::Timeout);
        }
        clear_errno();
        let entry = unsafe { libc::readdir(dir) };
        if entry.is_null() {
            return if io::Error::last_os_error().raw_os_error() == Some(0) {
                Ok(names.into_iter().collect())
            } else {
                Err(error())
            };
        }
        // readdir may return a short variable-length record in its DIR buffer.
        // Do not form/read an entire libc::dirent or a 256-byte d_name array.
        let name = unsafe { std::ptr::addr_of!((*entry).d_name).cast::<libc::c_char>() };
        let length = unsafe { libc::strnlen(name, 16) };
        if length >= 16 {
            return Err(ProbeError::Malformed);
        }
        insert(&mut names, unsafe {
            std::slice::from_raw_parts(name.cast::<u8>(), length)
        })?;
    }
    Err(ProbeError::TooLarge)
}
pub fn observe() -> SysInterfaceMeasurement {
    SysInterfaceMeasurement {
        entries: SysInterfaceCheck::required()
            .into_iter()
            .map(|check| {
                let value = match check.actor {
                    NativeReadActor::Libc => library(path(check.surface)),
                    NativeReadActor::Syscall => direct(path(check.surface)),
                };
                SysInterfaceObservation {
                    check,
                    names: match value {
                        Ok(names) => Sample::Value(names),
                        Err(e) => Sample::Error(e),
                    },
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_getdents64_wire_and_invalid_records() {
        // Manually encoded Linux record, including nonzero padding (not a name).
        let mut record = [0u8; 24];
        record[16..18].copy_from_slice(&24u16.to_ne_bytes());
        record[18] = 10; // symlink in /sys/class/net is still visible evidence.
        record[19..22].copy_from_slice(b"lo\0");
        record[23] = 0xff;
        let mut names = BTreeSet::new();
        consume(&mut names, &record).unwrap();
        assert_eq!(names.into_iter().collect::<Vec<_>>(), ["lo"]);
        for size in [0u16, 19, 20, 25, 32] {
            let mut bad = record;
            bad[16..18].copy_from_slice(&size.to_ne_bytes());
            assert_eq!(
                consume(&mut BTreeSet::new(), &bad),
                Err(ProbeError::Malformed)
            );
        }
        let duplicate = [record.as_slice(), record.as_slice()].concat();
        assert_eq!(
            consume(&mut BTreeSet::new(), &duplicate),
            Err(ProbeError::Malformed)
        );
        record[19..].fill(b'x');
        assert_eq!(
            consume(&mut BTreeSet::new(), &record),
            Err(ProbeError::Malformed)
        );
        assert_eq!(
            consume(&mut BTreeSet::new(), &[0; 19]),
            Err(ProbeError::Malformed)
        );
    }
    #[test]
    fn actual_host_loopback_is_visible_to_both_directory_actors() {
        let measured = observe();
        assert!(measured.bounded());
        for e in measured.entries {
            assert!(
                e.names
                    .value()
                    .expect("real host sys/proc reference")
                    .iter()
                    .any(|name| name == "lo"),
                "{e:?}"
            );
        }
    }
}
