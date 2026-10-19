//! THROWAWAY SPIKE — core-2.0 §12 step 1b (SELinux socket plumbing).
//!
//! Under enforcing SELinux on the device, prove the app<->daemon channel:
//!   - `server <path>`: a root daemon in its own domain binds a UDS *path* socket
//!     (never abstract, per §9), accepts, and prints each peer's `SO_PEERCRED`
//!     (uid/pid/gid — the forgery-proof identity the daemon would authenticate).
//!   - `client <path>`: connects, prints CONNECTED (with the reply) or the exact
//!     errno of the denial.
//!
//! Drive it as: server via `su -c` (domain `ksu`), client via
//! `su -Z u:r:untrusted_app:s0:c512,c768 -c ...` (a real app domain). Baseline is
//! deny; a targeted `ksud sepolicy patch 'allow ...'` should open exactly this path.

use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};

/// This process's SELinux context, e.g. `u:r:ksu:s0`.
fn my_context() -> String {
    std::fs::read_to_string("/proc/self/attr/current")
        .unwrap_or_default()
        .trim_end_matches('\0')
        .trim()
        .to_owned()
}

/// The connected peer's kernel-vouched credentials (SO_PEERCRED) — pid/uid/gid.
fn peer_cred(fd: i32) -> Option<(u32, u32, u32)> {
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    (rc == 0).then_some((cred.pid as u32, cred.uid, cred.gid))
}

fn server(path: &str) {
    let _ = std::fs::remove_file(path);
    let listener = match UnixListener::bind(path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "BIND FAILED as {} err={} ({:?})",
                my_context(),
                e,
                e.raw_os_error()
            );
            std::process::exit(1);
        }
    };
    // Open the DAC gate wide so the SELinux layer is the only thing under test.
    let _ = std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o666));
    println!(
        "server listening at {path} as {} — waiting for clients",
        my_context()
    );
    for stream in listener.incoming() {
        match stream {
            Ok(mut s) => {
                let cred = peer_cred(s.as_raw_fd());
                let mut buf = [0u8; 128];
                let n = s.read(&mut buf).unwrap_or(0);
                let _ = s.write_all(format!("ok from {}", my_context()).as_bytes());
                println!(
                    "ACCEPTED peer_cred(pid,uid,gid)={cred:?} said={:?}",
                    String::from_utf8_lossy(&buf[..n])
                );
            }
            Err(e) => println!("accept error: {e}"),
        }
    }
}

fn client(path: &str) {
    match UnixStream::connect(path) {
        Ok(mut s) => {
            let _ = s.write_all(format!("ping from {}", my_context()).as_bytes());
            let mut buf = [0u8; 128];
            let n = s.read(&mut buf).unwrap_or(0);
            println!(
                "CONNECTED as {} reply={:?}",
                my_context(),
                String::from_utf8_lossy(&buf[..n])
            );
        }
        Err(e) => println!(
            "DENIED/FAILED as {} err=\"{}\" errno={:?}",
            my_context(),
            e,
            e.raw_os_error()
        ),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(2)
        .map(String::as_str)
        .unwrap_or("/data/local/tmp/vh.sock");
    match args.get(1).map(String::as_str) {
        Some("server") => server(path),
        Some("client") => client(path),
        _ => {
            eprintln!("usage: uds-probe server|client [path]");
            std::process::exit(2);
        }
    }
}
