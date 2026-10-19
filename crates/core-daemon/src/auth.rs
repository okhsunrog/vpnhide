//! Production IPC authorization (§9 / step 7).
//!
//! Primary gate remains DAC on the socket file. This module is defense in depth:
//! `SO_PEERCRED` uid checks against an allowlist. Empty allowlist = allow any peer
//! (dev/host default). Production writes allowlist via the app's one-shot su grant
//! or `VPNHIDE_CORE_ALLOW_UIDS`.
//!
//! Package/signature mapping via packages.xml is a follow-up; uid allowlist is the
//! fail-closed gate available on every Magisk/KSU/APatch host today.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use libc::{SOL_SOCKET, SO_PEERCRED, getsockopt, socklen_t, ucred};

/// Env: comma-separated UIDs allowed to connect.
pub const ALLOW_UIDS_ENV: &str = "VPNHIDE_CORE_ALLOW_UIDS";
/// Env: path to allowlist file (one uid per line, `#` comments).
pub const ALLOW_UIDS_FILE_ENV: &str = "VPNHIDE_CORE_ALLOW_UIDS_FILE";
/// Default allowlist path next to the production socket.
pub const DEFAULT_ALLOW_UIDS_FILE: &str = "/data/adb/vpnhide/core/allowed_uids";
/// Env: socket owner uid after bind (`chown`); unset leaves creator (root) ownership.
pub const SOCKET_OWNER_UID_ENV: &str = "VPNHIDE_CORE_SOCKET_OWNER_UID";
/// Env: socket mode octal (default `0660` when owner set, else `0666` for host tests).
pub const SOCKET_MODE_ENV: &str = "VPNHIDE_CORE_SOCKET_MODE";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Allowlist {
    /// No allowlist configured — any peer uid is accepted (dev sandbox).
    Open,
    /// Only these uids may connect (production).
    Uids(BTreeSet<u32>),
}

impl Allowlist {
    pub fn load() -> Self {
        if let Ok(raw) = std::env::var(ALLOW_UIDS_ENV) {
            let set = parse_uid_list(&raw);
            if !set.is_empty() {
                return Allowlist::Uids(set);
            }
        }
        let path = std::env::var_os(ALLOW_UIDS_FILE_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_ALLOW_UIDS_FILE));
        match load_uid_file(&path) {
            Ok(set) if !set.is_empty() => Allowlist::Uids(set),
            _ => Allowlist::Open,
        }
    }

    pub fn permits(&self, uid: u32) -> bool {
        match self {
            Allowlist::Open => true,
            Allowlist::Uids(set) => set.contains(&uid),
        }
    }
}

fn parse_uid_list(raw: &str) -> BTreeSet<u32> {
    raw.split(|c: char| c == ',' || c.is_whitespace())
        .filter_map(|tok| tok.trim().parse().ok())
        .collect()
}

fn load_uid_file(path: &Path) -> io::Result<BTreeSet<u32>> {
    let text = fs::read_to_string(path)?;
    let mut set = BTreeSet::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(uid) = line.parse::<u32>() {
            set.insert(uid);
        }
    }
    Ok(set)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerCred {
    pub uid: u32,
    pub gid: u32,
    pub pid: u32,
}

/// Read `SO_PEERCRED` for an accepted connected Unix stream.
pub fn peer_cred<S: AsRawFd>(stream: &S) -> io::Result<PeerCred> {
    let mut cred = unsafe { std::mem::zeroed::<ucred>() };
    let mut len = std::mem::size_of::<ucred>() as socklen_t;
    let rc = unsafe {
        getsockopt(
            stream.as_raw_fd(),
            SOL_SOCKET,
            SO_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(PeerCred {
        uid: cred.uid,
        gid: cred.gid,
        pid: cred.pid as u32,
    })
}

pub fn authorize(stream: &impl AsRawFd, allowlist: &Allowlist) -> io::Result<PeerCred> {
    let cred = peer_cred(stream)?;
    if allowlist.permits(cred.uid) {
        Ok(cred)
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("peer uid {} not on core allowlist", cred.uid),
        ))
    }
}

/// Apply DAC on the bound socket path (chown/chmod). Best-effort; errors are logged by caller.
pub fn apply_socket_permissions(path: &Path) -> io::Result<()> {
    let mode = match std::env::var(SOCKET_MODE_ENV) {
        Ok(s) => {
            let s = s.trim().trim_start_matches("0o");
            u32::from_str_radix(s, 8).unwrap_or(0o660)
        }
        Err(_) => {
            if std::env::var_os(SOCKET_OWNER_UID_ENV).is_some() {
                0o660
            } else {
                // Host tests: world-connectable path under /tmp is fine.
                0o666
            }
        }
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
        if let Ok(uid_s) = std::env::var(SOCKET_OWNER_UID_ENV) {
            if let Ok(uid) = uid_s.parse::<u32>() {
                let gid = uid; // app primary gid ≈ uid on Android
                let rc = unsafe { libc::chown(path_cstr(path)?.as_ptr(), uid, gid) };
                if rc < 0 {
                    return Err(io::Error::last_os_error());
                }
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn path_cstr(path: &Path) -> io::Result<std::ffi::CString> {
    let s = path
        .to_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "socket path is not UTF-8"))?;
    std::ffi::CString::new(s).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_allowlist_permits_anyone() {
        assert!(Allowlist::Open.permits(0));
        assert!(Allowlist::Open.permits(10123));
    }

    #[test]
    fn uid_allowlist_is_fail_closed() {
        let list = Allowlist::Uids(BTreeSet::from([10123u32, 10124]));
        assert!(list.permits(10123));
        assert!(!list.permits(0));
        assert!(!list.permits(99999));
    }

    #[test]
    fn parse_env_list() {
        let set = parse_uid_list("10123, 10124\n10125");
        assert_eq!(set, BTreeSet::from([10123, 10124, 10125]));
    }

    #[test]
    fn load_file_skips_comments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("uids");
        fs::write(&path, "# app\n10123\n\n10124 # secondary\n").unwrap();
        let set = load_uid_file(&path).unwrap();
        assert_eq!(set, BTreeSet::from([10123, 10124]));
    }

    #[test]
    fn load_from_file_env_is_fail_closed() {
        let _guard = crate::ports::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("uids");
        fs::write(&path, "4242
").unwrap();
        // SAFETY: held under test_env_lock.
        unsafe {
            std::env::remove_var(ALLOW_UIDS_ENV);
            std::env::set_var(ALLOW_UIDS_FILE_ENV, &path);
        }
        let list = Allowlist::load();
        unsafe {
            std::env::remove_var(ALLOW_UIDS_FILE_ENV);
        }
        assert_eq!(list, Allowlist::Uids(BTreeSet::from([4242u32])));
        assert!(!list.permits(0));
        assert!(list.permits(4242));
    }
}
