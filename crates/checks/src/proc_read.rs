//! Closed-set bounded proc reads shared by independent collectors. No arbitrary
//! paths, writes or subprocesses; direct actors use openat/read syscalls.
use std::{
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    time::{Duration, Instant},
};
use vpnhide_diagnostic_model::{NativeReadActor, ProbeError};
pub(crate) enum ProcFile {
    InterfaceStats,
    Ipv6Addresses,
    RoutesIpv4,
    RoutesIpv6,
}
fn error() -> ProbeError {
    ProbeError::Errno(
        io::Error::last_os_error()
            .raw_os_error()
            .filter(|e| *e > 0)
            .unwrap_or(libc::EIO),
    )
}
pub(crate) struct ProcBytes {
    pub bytes: Vec<u8>,
    pub error: Option<ProbeError>,
}
pub(crate) fn read(file: ProcFile, actor: NativeReadActor) -> Result<Vec<u8>, ProbeError> {
    let value = read_partial(file, actor);
    match value.error {
        Some(e) => Err(e),
        None => Ok(value.bytes),
    }
}
pub(crate) fn read_partial(file: ProcFile, actor: NativeReadActor) -> ProcBytes {
    let mut bytes = Vec::new();
    let error = read_into(file, actor, &mut bytes).err();
    ProcBytes { bytes, error }
}
fn read_into(
    file: ProcFile,
    actor: NativeReadActor,
    bytes: &mut Vec<u8>,
) -> Result<(), ProbeError> {
    let path = match file {
        ProcFile::InterfaceStats => c"/proc/net/dev",
        ProcFile::Ipv6Addresses => c"/proc/net/if_inet6",
        ProcFile::RoutesIpv4 => c"/proc/net/route",
        ProcFile::RoutesIpv6 => c"/proc/net/ipv6_route",
    };
    let flags = libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NONBLOCK;
    let raw = unsafe {
        match actor {
            NativeReadActor::Libc => libc::open(path.as_ptr(), flags) as libc::c_long,
            NativeReadActor::Syscall => {
                libc::syscall(libc::SYS_openat, libc::AT_FDCWD, path.as_ptr(), flags, 0)
            }
        }
    };
    if raw < 0 {
        return Err(error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(raw as libc::c_int) };
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut buffer = [0u8; 4096];
    for _ in 0..128 {
        if Instant::now() >= deadline {
            return Err(ProbeError::Timeout);
        }
        let size = unsafe {
            match actor {
                NativeReadActor::Libc => {
                    libc::read(fd.as_raw_fd(), buffer.as_mut_ptr().cast(), buffer.len())
                        as libc::c_long
                }
                NativeReadActor::Syscall => libc::syscall(
                    libc::SYS_read,
                    fd.as_raw_fd(),
                    buffer.as_mut_ptr(),
                    buffer.len(),
                ),
            }
        };
        if size < 0 {
            if io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(error());
        }
        if size == 0 {
            return Ok(());
        }
        if size as usize > buffer.len() || bytes.len() + size as usize > 256 * 1024 {
            return Err(ProbeError::TooLarge);
        }
        bytes.extend_from_slice(&buffer[..size as usize]);
    }
    Err(ProbeError::TooLarge)
}
