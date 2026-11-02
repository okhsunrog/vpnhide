//! Socket-only observations. Each actor/target has a new CLOEXEC socket; no
//! interface, route, address, packet or process-wide network binding changes.
use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use vpnhide_diagnostic_model::{BindActor, BindAttempt, BindMeasurement, ProbeError, Sample};
fn errno() -> ProbeError {
    ProbeError::Errno(
        io::Error::last_os_error()
            .raw_os_error()
            .filter(|v| *v > 0)
            .unwrap_or(libc::EIO),
    )
}
fn failed(error: ProbeError) -> BindAttempt {
    BindAttempt {
        initial: Sample::Error(error.clone()),
        set: Sample::Error(error.clone()),
        final_device: Sample::Error(error),
    }
}
fn read_binding(fd: i32, actor: BindActor) -> Sample<String> {
    let mut bytes = [0x7f_u8; libc::IFNAMSIZ];
    let mut length = bytes.len() as libc::socklen_t;
    let result = unsafe {
        match actor {
            BindActor::Libc => libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_BINDTODEVICE,
                bytes.as_mut_ptr().cast(),
                &mut length,
            ) as libc::c_long,
            BindActor::Syscall => libc::syscall(
                libc::SYS_getsockopt,
                fd,
                libc::SOL_SOCKET,
                libc::SO_BINDTODEVICE,
                bytes.as_mut_ptr(),
                &mut length,
            ),
        }
    };
    if result != 0 {
        return Sample::Error(errno());
    }
    match decode_binding(&bytes, length as usize) {
        Ok(name) => Sample::Value(name),
        Err(error) => Sample::Error(error),
    }
}
fn decode_binding(bytes: &[u8], length: usize) -> Result<String, ProbeError> {
    if length == 0 {
        return Ok(String::new());
    }
    let value = bytes.get(..length).ok_or(ProbeError::Malformed)?;
    if length > libc::IFNAMSIZ || value.last() != Some(&0) {
        return Err(ProbeError::Malformed);
    }
    let name = &value[..length - 1];
    if !name.is_ascii() || name.iter().any(u8::is_ascii_control) {
        return Err(ProbeError::Malformed);
    }
    String::from_utf8(name.to_vec()).map_err(|_| ProbeError::Malformed)
}
pub fn attempt(name: &str, actor: BindActor) -> BindAttempt {
    if name.is_empty()
        || name.len() >= libc::IFNAMSIZ
        || !name.is_ascii()
        || name.bytes().any(|b| b.is_ascii_control())
    {
        return failed(ProbeError::Malformed);
    }
    let Ok(name) = CString::new(name) else {
        return failed(ProbeError::Malformed);
    };
    let raw = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
    if raw < 0 {
        return failed(errno());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let initial = read_binding(fd.as_raw_fd(), actor);
    let result = unsafe {
        match actor {
            BindActor::Libc => libc::setsockopt(
                fd.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_BINDTODEVICE,
                name.as_ptr().cast(),
                name.as_bytes_with_nul().len() as libc::socklen_t,
            ) as libc::c_long,
            BindActor::Syscall => libc::syscall(
                libc::SYS_setsockopt,
                fd.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_BINDTODEVICE,
                name.as_ptr(),
                name.as_bytes_with_nul().len() as libc::socklen_t,
            ),
        }
    };
    // Save errno before getsockopt can overwrite it; read even after failure.
    let set = if result == 0 {
        Sample::Value(0)
    } else {
        Sample::Error(errno())
    };
    let final_device = read_binding(fd.as_raw_fd(), actor);
    BindAttempt {
        initial,
        set,
        final_device,
    }
}
pub fn observe(name: &str) -> BindMeasurement {
    BindMeasurement {
        libc_fixture: attempt(name, BindActor::Libc),
        libc_ordinary: attempt("lo", BindActor::Libc),
        syscall_fixture: attempt(name, BindActor::Syscall),
        syscall_ordinary: attempt("lo", BindActor::Syscall),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readback_is_length_and_terminator_checked_without_guessing_from_padding() {
        assert_eq!(decode_binding(&[0x7f; 16], 0), Ok(String::new()));
        assert_eq!(decode_binding(b"lo\0padding", 3), Ok("lo".into()));
        for (bytes, length) in [
            (b"lo".as_slice(), 2),
            (b"lo\0", 4),
            (b"l\0\0", 3),
            (b"\xff\0", 2),
        ] {
            assert_eq!(decode_binding(bytes, length), Err(ProbeError::Malformed));
        }
    }
    #[test]
    fn invalid_names_do_not_create_a_socket_and_each_actor_keeps_real_observation_errors() {
        for actor in BindActor::ALL {
            assert!(matches!(
                attempt("", actor).set,
                Sample::Error(ProbeError::Malformed)
            ));
            let observation = attempt("lo", actor);
            // Sandboxed hosts may deny socket/binding. Such outcomes stay errors,
            // never a fake successful or unbound result.
            if observation.set.value() == Some(&0) {
                assert_eq!(
                    observation.final_device.value().map(String::as_str),
                    Some("lo")
                );
            }
        }
    }
}
