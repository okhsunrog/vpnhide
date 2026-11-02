//! Stream framing with owned SCM_RIGHTS and request-time SCM_CREDENTIALS.
//! Receive ancillary data with the first byte; never infer child identity from
//! the provider's possibly relayed companion connection's SO_PEERCRED.

use serde::{Serialize, de::DeserializeOwned};
use std::io::{self, Read, Write};
use std::mem::{size_of, zeroed};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use crate::{Credentials, MAX_FRAME};

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub fn connect(path: &Path) -> io::Result<UnixStream> {
    connect_before(path, std::time::Instant::now() + Duration::from_secs(2))
}

fn connect_before(path: &Path, deadline: std::time::Instant) -> io::Result<UnixStream> {
    let mut address: libc::sockaddr_un = unsafe { zeroed() };
    let path = path.as_os_str().as_bytes();
    if path.is_empty() || path.len() >= address.sun_path.len() || path.contains(&0) {
        return Err(invalid("invalid loader socket path"));
    }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    // SAFETY: bounded pathname fits the zeroed sun_path array including its NUL.
    unsafe {
        std::ptr::copy_nonoverlapping(
            path.as_ptr(),
            address.sun_path.as_mut_ptr().cast(),
            path.len(),
        );
    }
    let length =
        (std::mem::offset_of!(libc::sockaddr_un, sun_path) + path.len() + 1) as libc::socklen_t;
    let raw = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            0,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    let stream = unsafe { UnixStream::from_raw_fd(raw) };
    loop {
        let Some(remaining) = deadline
            .checked_duration_since(std::time::Instant::now())
            .filter(|d| !d.is_zero())
        else {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "loader connect deadline",
            ));
        };
        let result =
            unsafe { libc::connect(raw, (&address as *const libc::sockaddr_un).cast(), length) };
        if result == 0 {
            break;
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::EISCONN) => break,
            Some(libc::EAGAIN) => std::thread::sleep(remaining.min(Duration::from_millis(5))),
            Some(libc::EINPROGRESS | libc::EALREADY) => {
                let mut poll = libc::pollfd {
                    fd: raw,
                    events: libc::POLLOUT,
                    revents: 0,
                };
                let timeout = remaining.as_millis().min(i32::MAX as u128).max(1) as i32;
                let ready = unsafe { libc::poll(&mut poll, 1, timeout) };
                if ready < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                    return Err(io::Error::last_os_error());
                }
                if ready > 0 {
                    if let Some(error) = stream.take_error()? {
                        return Err(error);
                    }
                    if stream.peer_addr().is_ok() {
                        break;
                    }
                }
            }
            Some(libc::EINTR) => continue,
            _ => return Err(error),
        }
    }
    stream.set_nonblocking(false)?;
    bounded(&stream)?;
    Ok(stream)
}

pub fn bounded(stream: &UnixStream) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))
}

pub fn pass_credentials(stream: &UnixStream) -> io::Result<()> {
    let enabled: libc::c_int = 1;
    // SAFETY: a valid connected socket and correctly sized option value.
    let result = unsafe {
        libc::setsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PASSCRED,
            (&enabled as *const libc::c_int).cast(),
            size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn peer_credentials(stream: &UnixStream) -> io::Result<Credentials> {
    let mut cred: libc::ucred = unsafe { zeroed() };
    let mut length = size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut length,
        )
    };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    if length as usize != size_of::<libc::ucred>() || cred.pid <= 0 {
        return Err(invalid("invalid peer credentials"));
    }
    Ok(Credentials {
        pid: cred.pid as u32,
        uid: cred.uid,
        gid: cred.gid,
    })
}

pub fn send<T: Serialize>(
    stream: &mut UnixStream,
    value: &T,
    descriptors: &[RawFd],
) -> io::Result<()> {
    bounded(stream)?;
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    if descriptors.len() > 2 {
        return Err(invalid("too many loader descriptors"));
    }
    let body = serde_json::to_vec(value).map_err(io::Error::other)?;
    if body.is_empty() || body.len() > MAX_FRAME {
        return Err(invalid("loader frame exceeds budget"));
    }
    let mut frame = Vec::with_capacity(body.len() + 4);
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(&body);
    let mut vector = libc::iovec {
        iov_base: frame.as_mut_ptr().cast(),
        iov_len: frame.len(),
    };
    let mut control = [0usize; 8];
    let mut message: libc::msghdr = unsafe { zeroed() };
    message.msg_iov = &mut vector;
    message.msg_iovlen = 1;
    if !descriptors.is_empty() {
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen = unsafe { libc::CMSG_SPACE(size_of_val(descriptors) as u32) as _ };
        // SAFETY: aligned control storage fits the header and at most two fds.
        unsafe {
            let header = libc::CMSG_FIRSTHDR(&message);
            (*header).cmsg_level = libc::SOL_SOCKET;
            (*header).cmsg_type = libc::SCM_RIGHTS;
            (*header).cmsg_len = libc::CMSG_LEN(size_of_val(descriptors) as u32) as _;
            std::ptr::copy_nonoverlapping(
                descriptors.as_ptr().cast::<u8>(),
                libc::CMSG_DATA(header),
                size_of_val(descriptors),
            );
        }
    }
    let count = unsafe { libc::sendmsg(stream.as_raw_fd(), &message, libc::MSG_NOSIGNAL) };
    if count < 0 {
        return Err(io::Error::last_os_error());
    }
    if count == 0 {
        return Err(io::Error::new(
            io::ErrorKind::WriteZero,
            "empty loader send",
        ));
    }
    let mut remaining = &frame[count as usize..];
    while !remaining.is_empty() {
        let budget = deadline
            .checked_duration_since(std::time::Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "loader send deadline"))?;
        stream.set_write_timeout(Some(budget))?;
        match stream.write(remaining) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "incomplete loader send",
                ));
            }
            Ok(count) => remaining = &remaining[count..],
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

pub struct Received<T> {
    pub value: T,
    pub descriptors: Vec<OwnedFd>,
    pub credentials: Option<Credentials>,
}

pub fn receive<T: DeserializeOwned>(
    stream: &mut UnixStream,
    expected_descriptors: usize,
    require_credentials: bool,
) -> io::Result<Received<T>> {
    bounded(stream)?;
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    if expected_descriptors > 2 {
        return Err(invalid("invalid descriptor contract"));
    }
    let mut first = [0u8];
    let mut vector = libc::iovec {
        iov_base: first.as_mut_ptr().cast(),
        iov_len: 1,
    };
    // Enough to own even excess received fds before rejecting. Kernel closes
    // descriptors beyond the ancillary buffer on MSG_CTRUNC.
    let mut control = [0usize; 32];
    let mut message: libc::msghdr = unsafe { zeroed() };
    message.msg_iov = &mut vector;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = size_of_val(&control) as _;
    let count = unsafe { libc::recvmsg(stream.as_raw_fd(), &mut message, libc::MSG_CMSG_CLOEXEC) };
    if count < 0 {
        return Err(io::Error::last_os_error());
    }
    if count == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "loader disconnected",
        ));
    }
    let mut descriptors = Vec::new();
    let mut credentials = None;
    let mut invalid_control = false;
    // SAFETY: recvmsg filled kernel-validated headers in aligned owned storage.
    unsafe {
        let mut header = libc::CMSG_FIRSTHDR(&message);
        while !header.is_null() {
            let size = (*header).cmsg_len as usize;
            let base = libc::CMSG_LEN(0) as usize;
            if size < base {
                invalid_control = true;
                break;
            }
            let size = size - base;
            if (*header).cmsg_level == libc::SOL_SOCKET && (*header).cmsg_type == libc::SCM_RIGHTS {
                if !size.is_multiple_of(size_of::<RawFd>()) {
                    invalid_control = true;
                }
                for index in 0..size / size_of::<RawFd>() {
                    let fd = std::ptr::read_unaligned(
                        libc::CMSG_DATA(header).cast::<RawFd>().add(index),
                    );
                    descriptors.push(OwnedFd::from_raw_fd(fd));
                }
            } else if (*header).cmsg_level == libc::SOL_SOCKET
                && (*header).cmsg_type == libc::SCM_CREDENTIALS
                && size == size_of::<libc::ucred>()
            {
                let cred = std::ptr::read_unaligned(libc::CMSG_DATA(header).cast::<libc::ucred>());
                if credentials.is_some() || cred.pid <= 0 {
                    invalid_control = true;
                }
                credentials = Some(Credentials {
                    pid: cred.pid as u32,
                    uid: cred.uid,
                    gid: cred.gid,
                });
            } else {
                invalid_control = true;
            }
            header = libc::CMSG_NXTHDR(&message, header);
        }
    }
    if invalid_control
        || message.msg_flags & (libc::MSG_CTRUNC | libc::MSG_TRUNC) != 0
        || descriptors.len() != expected_descriptors
        || (require_credentials && credentials.is_none())
    {
        return Err(invalid("loader ancillary contract mismatch"));
    }
    let mut length = [first[0], 0, 0, 0];
    read_before(stream, &mut length[1..], deadline)?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > MAX_FRAME {
        return Err(invalid("loader frame exceeds budget"));
    }
    let mut body = vec![0; length];
    read_before(stream, &mut body, deadline)?;
    let value = serde_json::from_slice(&body).map_err(io::Error::other)?;
    Ok(Received {
        value,
        descriptors,
        credentials,
    })
}

fn read_before(
    stream: &mut UnixStream,
    mut output: &mut [u8],
    deadline: std::time::Instant,
) -> io::Result<()> {
    while !output.is_empty() {
        let remaining = deadline
            .checked_duration_since(std::time::Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "loader frame deadline"))?;
        stream.set_read_timeout(Some(remaining))?;
        match stream.read(output) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "incomplete loader frame",
                ));
            }
            Ok(count) => output = &mut output[count..],
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::process::CommandExt;

    #[test]
    fn credential_sender_child() {
        let Ok(raw) = std::env::var("VPNHIDE_LOADER_TEST_FD") else {
            return;
        };
        let raw = raw.parse::<RawFd>().unwrap();
        // Only the helper process receives this explicitly inherited descriptor.
        let mut stream = unsafe { UnixStream::from_raw_fd(raw) };
        send(&mut stream, &true, &[]).unwrap();
    }

    #[test]
    fn request_credentials_identify_child_on_a_socketpair_created_by_parent() {
        let (sender, mut receiver) = UnixStream::pair().unwrap();
        pass_credentials(&receiver).unwrap();
        let descriptor = sender.as_raw_fd();
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "transport::tests::credential_sender_child"])
            .env("VPNHIDE_LOADER_TEST_FD", descriptor.to_string())
            .stdout(std::process::Stdio::null());
        // SAFETY: fcntl is async-signal-safe; no allocation/locks in pre_exec.
        unsafe {
            command.pre_exec(move || {
                if libc::fcntl(descriptor, libc::F_SETFD, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn().unwrap();
        let received = receive::<bool>(&mut receiver, 0, true).unwrap();
        assert!(received.value);
        assert_eq!(received.credentials.unwrap().pid, child.id());
        assert_ne!(received.credentials.unwrap().pid, std::process::id());
        assert!(child.wait().unwrap().success());
    }

    #[test]
    fn saturated_listener_backlog_cannot_block_private_connect() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("observer.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        assert_eq!(unsafe { libc::listen(listener.as_raw_fd(), 1) }, 0);
        let _first = connect(&path).unwrap();
        let _second = connect(&path).unwrap();
        let started = std::time::Instant::now();
        assert_eq!(
            connect_before(&path, started + Duration::from_millis(50))
                .unwrap_err()
                .kind(),
            io::ErrorKind::TimedOut
        );
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn actual_sender_credentials_and_owned_descriptors_cross_the_socket() {
        let (mut sender, mut receiver) = UnixStream::pair().unwrap();
        pass_credentials(&receiver).unwrap();
        let file = tempfile::tempfile().unwrap();
        let sent = file.metadata().unwrap();
        send(&mut sender, &vec!["observation"], &[file.as_raw_fd()]).unwrap();
        let received = receive::<Vec<String>>(&mut receiver, 1, true).unwrap();
        assert_eq!(received.credentials.unwrap().pid, std::process::id());
        assert_eq!(received.credentials.unwrap().uid, unsafe {
            libc::geteuid()
        });
        assert_eq!(received.value, ["observation"]);
        let received = File::from(received.descriptors.into_iter().next().unwrap());
        assert_eq!(received.metadata().unwrap().ino(), sent.ino());
        assert_ne!(
            unsafe { libc::fcntl(received.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC,
            0
        );
    }

    use std::fs::File;

    #[test]
    fn missing_credentials_wrong_fd_count_and_oversized_prefix_are_errors() {
        let (mut sender, mut receiver) = UnixStream::pair().unwrap();
        send(&mut sender, &true, &[]).unwrap();
        assert!(receive::<bool>(&mut receiver, 0, true).is_err());
        let (mut sender, mut receiver) = UnixStream::pair().unwrap();
        let file = tempfile::tempfile().unwrap();
        send(&mut sender, &true, &[file.as_raw_fd()]).unwrap();
        assert!(receive::<bool>(&mut receiver, 0, false).is_err());
        let (mut sender, mut receiver) = UnixStream::pair().unwrap();
        sender.write_all(&u32::MAX.to_be_bytes()).unwrap();
        assert!(receive::<bool>(&mut receiver, 0, false).is_err());
        assert!(send(&mut sender, &"x".repeat(MAX_FRAME), &[]).is_err());
    }

    fn inode_fds(file: &File) -> usize {
        let inode = file.metadata().unwrap().ino();
        std::fs::read_dir("/proc/self/fd")
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| std::fs::metadata(e.path()).is_ok_and(|m| m.ino() == inode))
            .count()
    }

    #[test]
    fn ancillary_truncation_closes_all_received_descriptors() {
        let (sender, mut receiver) = UnixStream::pair().unwrap();
        let file = tempfile::tempfile().unwrap();
        let original = inode_fds(&file);
        let descriptors = [file.as_raw_fd(); 100];
        let mut first = [0u8];
        let mut vector = libc::iovec {
            iov_base: first.as_mut_ptr().cast(),
            iov_len: 1,
        };
        let mut control = [0usize; 128];
        let mut message: libc::msghdr = unsafe { zeroed() };
        message.msg_iov = &mut vector;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen = unsafe { libc::CMSG_SPACE(size_of_val(&descriptors) as u32) as _ };
        unsafe {
            let header = libc::CMSG_FIRSTHDR(&message);
            (*header).cmsg_level = libc::SOL_SOCKET;
            (*header).cmsg_type = libc::SCM_RIGHTS;
            (*header).cmsg_len = libc::CMSG_LEN(size_of_val(&descriptors) as u32) as _;
            std::ptr::copy_nonoverlapping(
                descriptors.as_ptr().cast::<u8>(),
                libc::CMSG_DATA(header),
                size_of_val(&descriptors),
            );
            assert_eq!(
                libc::sendmsg(sender.as_raw_fd(), &message, libc::MSG_NOSIGNAL),
                1
            );
        }
        assert!(receive::<bool>(&mut receiver, 2, false).is_err());
        assert_eq!(inode_fds(&file), original);
    }
}
