//! Same-thread registration only. Root arms and activates the retained identity
//! separately; successful ioctl return is never proof of RAW authorization.
use std::io;
use std::mem::zeroed;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

pub fn register(nonce: u64) -> io::Result<()> {
    let name = vpnhide_protocol::diagnostic_raw::registration_name(nonce)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid RAW token"))?;
    // Loopback is an ordinary object. No VPN interface needs to be visible yet.
    let index = unsafe { libc::if_nametoindex(c"lo".as_ptr()) };
    if index == 0 || index > i32::MAX as u32 {
        return Err(io::Error::other("loopback index unavailable"));
    }
    let raw = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let mut request: libc::ifreq = unsafe { zeroed() };
    for (target, byte) in request.ifr_name.iter_mut().zip(name) {
        *target = byte as libc::c_char;
    }
    request.ifr_ifru.ifru_ifindex = index as i32;
    if unsafe { libc::ioctl(fd.as_raw_fd(), libc::SIOCGIFNAME as _, &mut request) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // Registration must preserve the ordinary ioctl's real result.
    if request.ifr_name[..3] != [b'l' as libc::c_char, b'o' as libc::c_char, 0] {
        return Err(io::Error::other(
            "loopback result changed during registration",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn zero_token_cannot_issue_registration() {
        assert_eq!(
            super::register(0).unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );
    }
    #[test]
    fn ordinary_kernel_ignores_registration_payload_and_preserves_loopback() {
        // A clean host has no RAW provider. This verifies transport preservation,
        // explicitly not a grant or hiding proof.
        super::register(0x0123456789abcdef).unwrap();
    }
}
