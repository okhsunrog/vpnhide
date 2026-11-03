//! Bounded unprivileged route-netlink reads. The closed request set cannot add
//! an interface, address, route or rule. Core fixture writes have their own owner.
use std::{
    io,
    mem::zeroed,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    time::{Duration, Instant},
};
use vpnhide_diagnostic_model::{AddressActor, ProbeError};

fn error() -> ProbeError {
    ProbeError::Errno(
        io::Error::last_os_error()
            .raw_os_error()
            .filter(|e| *e > 0)
            .unwrap_or(libc::EIO),
    )
}

pub fn bound_socket(actor: AddressActor) -> Result<OwnedFd, ProbeError> {
    let kind = libc::SOCK_RAW | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC;
    let raw = unsafe {
        match actor {
            AddressActor::Libc => {
                libc::socket(libc::AF_NETLINK, kind, libc::NETLINK_ROUTE) as libc::c_long
            }
            AddressActor::Syscall => libc::syscall(
                libc::SYS_socket,
                libc::AF_NETLINK,
                kind,
                libc::NETLINK_ROUTE,
            ),
        }
    };
    if raw < 0 {
        return Err(error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(raw as i32) };
    let mut address: libc::sockaddr_nl = unsafe { zeroed() };
    address.nl_family = libc::AF_NETLINK as _;
    let pointer = (&address as *const libc::sockaddr_nl).cast::<libc::sockaddr>();
    let size = size_of::<libc::sockaddr_nl>() as libc::socklen_t;
    let result = unsafe {
        match actor {
            AddressActor::Libc => libc::bind(fd.as_raw_fd(), pointer, size) as libc::c_long,
            AddressActor::Syscall => libc::syscall(libc::SYS_bind, fd.as_raw_fd(), pointer, size),
        }
    };
    if result != 0 {
        return Err(error());
    }
    Ok(fd)
}

pub fn receive(
    fd: &OwnedFd,
    actor: AddressActor,
    deadline: Instant,
) -> Result<Vec<u8>, ProbeError> {
    let mut bytes = vec![0; 64 * 1024];
    for _ in 0..128 {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or(ProbeError::Timeout)?;
        let mut poll = libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ready =
            unsafe { libc::poll(&mut poll, 1, remaining.as_millis().clamp(1, 1000) as i32) };
        if ready < 0 {
            if io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(error());
        }
        if ready == 0 {
            return Err(ProbeError::Timeout);
        }
        let mut sender: libc::sockaddr_nl = unsafe { zeroed() };
        let mut iov = libc::iovec {
            iov_base: bytes.as_mut_ptr().cast(),
            iov_len: bytes.len(),
        };
        let mut message: libc::msghdr = unsafe { zeroed() };
        message.msg_name = (&mut sender as *mut libc::sockaddr_nl).cast();
        message.msg_namelen = size_of::<libc::sockaddr_nl>() as _;
        message.msg_iov = &mut iov;
        message.msg_iovlen = 1;
        let size = unsafe {
            match actor {
                AddressActor::Libc => {
                    libc::recvmsg(fd.as_raw_fd(), &mut message, 0) as libc::c_long
                }
                AddressActor::Syscall => {
                    libc::syscall(libc::SYS_recvmsg, fd.as_raw_fd(), &mut message, 0)
                }
            }
        };
        if size < 0 {
            if matches!(
                io::Error::last_os_error().raw_os_error(),
                Some(libc::EAGAIN | libc::EINTR)
            ) {
                continue;
            }
            return Err(error());
        }
        if size == 0
            || size as usize > bytes.len()
            || sender.nl_pid != 0
            || sender.nl_groups != 0
            || sender.nl_family != libc::AF_NETLINK as libc::sa_family_t
            || message.msg_namelen as usize != size_of::<libc::sockaddr_nl>()
            || message.msg_flags & libc::MSG_TRUNC != 0
        {
            return Err(ProbeError::Malformed);
        }
        bytes.truncate(size as usize);
        return Ok(bytes);
    }
    Err(ProbeError::TooLarge)
}

#[derive(Clone, Copy)]
pub enum DumpKind {
    Links,
    Addresses,
}
pub fn dump(
    kind: DumpKind,
    actor: AddressActor,
    mut consume: impl FnMut(&[u8]) -> Result<bool, ProbeError>,
) -> Result<(), ProbeError> {
    let fd = bound_socket(actor)?;
    let mut address: libc::sockaddr_nl = unsafe { zeroed() };
    address.nl_family = libc::AF_NETLINK as _;
    let (request_kind, payload_length) = match kind {
        DumpKind::Links => (18u16, 16),
        DumpKind::Addresses => (22u16, 8),
    };
    let mut request = vec![0u8; 16 + payload_length];
    let length = request.len();
    request[..4].copy_from_slice(&(length as u32).to_ne_bytes());
    request[4..6].copy_from_slice(&request_kind.to_ne_bytes());
    request[6..8].copy_from_slice(&0x301u16.to_ne_bytes());
    request[8..12].copy_from_slice(&1u32.to_ne_bytes());
    let destination = (&address as *const libc::sockaddr_nl).cast::<libc::sockaddr>();
    let sent = unsafe {
        match actor {
            AddressActor::Libc => libc::sendto(
                fd.as_raw_fd(),
                request.as_ptr().cast(),
                request.len(),
                0,
                destination,
                size_of::<libc::sockaddr_nl>() as _,
            ) as libc::c_long,
            AddressActor::Syscall => libc::syscall(
                libc::SYS_sendto,
                fd.as_raw_fd(),
                request.as_ptr(),
                request.len(),
                0,
                destination,
                size_of::<libc::sockaddr_nl>(),
            ),
        }
    };
    if sent != length as libc::c_long {
        return Err(error());
    }
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut total = 0usize;
    for _ in 0..128 {
        let bytes = receive(&fd, actor, deadline)?;
        total += bytes.len();
        if total > 1024 * 1024 {
            return Err(ProbeError::TooLarge);
        }
        if consume(&bytes)? {
            return Ok(());
        }
    }
    Err(ProbeError::TooLarge)
}
