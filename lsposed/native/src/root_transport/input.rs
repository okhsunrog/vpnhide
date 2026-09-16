use std::io;
use std::time::{Duration, Instant};

const MAX_SCRIPT: usize = 2 * 1024 * 1024;
const HEADER_LIMIT: usize = 64;

/// A pipe ending early must not execute a syntactically valid prefix of the intended command.
pub fn read_script() -> io::Result<Vec<u8>> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut bytes = Vec::new();
    loop {
        let millis = deadline
            .saturating_duration_since(Instant::now())
            .as_millis();
        if millis == 0 {
            return Err(io::ErrorKind::TimedOut.into());
        }
        let mut poll = libc::pollfd {
            fd: libc::STDIN_FILENO,
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut poll, 1, millis as libc::c_int) };
        if ready < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if ready == 0 {
            return Err(io::ErrorKind::TimedOut.into());
        }
        let mut buffer = [0_u8; 8192];
        let count =
            unsafe { libc::read(libc::STDIN_FILENO, buffer.as_mut_ptr().cast(), buffer.len()) };
        if count < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count as usize]);
        if bytes.len() > MAX_SCRIPT + HEADER_LIMIT {
            return Err(io::ErrorKind::InvalidInput.into());
        }
    }
    decode_script(bytes)
}

fn decode_script(bytes: Vec<u8>) -> io::Result<Vec<u8>> {
    let newline = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .ok_or(io::ErrorKind::InvalidInput)?;
    if newline >= HEADER_LIMIT {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let length = std::str::from_utf8(&bytes[..newline])
        .ok()
        .and_then(|header| header.strip_prefix("vpnhide-script 1 "))
        .and_then(|length| length.parse::<usize>().ok())
        .ok_or(io::ErrorKind::InvalidInput)?;
    let payload = &bytes[newline + 1..];
    if length == 0 || length > MAX_SCRIPT || length != payload.len() || payload.contains(&0) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    Ok(payload.to_vec())
}
