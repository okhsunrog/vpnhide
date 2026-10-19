//! IPC authorization.
//!
//! Production (2.0): the socket lives in the app's data dir with the app's owner, mode and
//! SELinux label ([`apply_app_socket_permissions`]), and every accept is checked with
//! `SO_PEERCRED` against [`Allowlist::AppOwner`]: root or the uid owning that dir. The app
//! writes nothing to configure this. See `app_socket`.
//!
//! Host/dev: a plain path socket; `VPNHIDE_CORE_ALLOW_UIDS` restricts peers, unset = open.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use libc::{SO_PEERCRED, SOL_SOCKET, getsockopt, socklen_t, ucred};

/// Env: comma-separated UIDs allowed to connect.
pub const ALLOW_UIDS_ENV: &str = "VPNHIDE_CORE_ALLOW_UIDS";
/// Env: socket owner uid after bind (`chown`); unset leaves creator (root) ownership.
pub const SOCKET_OWNER_UID_ENV: &str = "VPNHIDE_CORE_SOCKET_OWNER_UID";
/// Env: socket mode octal (default `0660` when owner set, else `0666` for host tests).
pub const SOCKET_MODE_ENV: &str = "VPNHIDE_CORE_SOCKET_MODE";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Allowlist {
    /// No allowlist configured — any peer uid is accepted (dev sandbox).
    Open,
    /// Only these uids may connect.
    Uids(BTreeSet<u32>),
    /// Production (2.0): root, and the uid that owns the app's data dir. Re-resolved on
    /// every accept, so an APK reinstall (new uid) is followed without a restart.
    AppOwner(PathBuf),
}

impl Allowlist {
    pub fn load() -> Self {
        if let Ok(raw) = std::env::var(ALLOW_UIDS_ENV) {
            let set = parse_uid_list(&raw);
            if !set.is_empty() {
                return Allowlist::Uids(set);
            }
        }
        Allowlist::Open
    }

    pub fn permits(&self, uid: u32) -> bool {
        match self {
            Allowlist::Open => true,
            Allowlist::Uids(set) => set.contains(&uid),
            Allowlist::AppOwner(dir) => uid == 0 || owner_uid(dir).is_ok_and(|owner| owner == uid),
        }
    }
}

fn parse_uid_list(raw: &str) -> BTreeSet<u32> {
    raw.split(|c: char| c == ',' || c.is_whitespace())
        .filter_map(|tok| tok.trim().parse().ok())
        .collect()
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

fn owner_uid(path: &Path) -> io::Result<u32> {
    use std::os::unix::fs::MetadataExt;
    Ok(fs::metadata(path)?.uid())
}

/// Lock the app socket down to the app: owner = the app data dir's uid/gid, mode `0600`,
/// and the dir's SELinux label (app_data_file with the app's MLS categories) so the app
/// domain may `write` it and no other app can even traverse to it. Root (core) bypasses DAC.
/// The label copy is best-effort (hosts without SELinux xattrs).
pub fn apply_app_socket_permissions(socket: &Path, app_dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let meta = fs::metadata(app_dir)?;
    let sock = path_cstr(socket)?;
    if unsafe { libc::geteuid() } == 0 {
        let rc = unsafe { libc::lchown(sock.as_ptr(), meta.uid(), meta.gid()) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;
    if let Err(e) = copy_selinux_label(app_dir, socket) {
        eprintln!("core: socket label copy skipped: {e}");
    }
    Ok(())
}

fn copy_selinux_label(from: &Path, to: &Path) -> io::Result<()> {
    const NAME: &[u8] = b"security.selinux\0";
    let src = path_cstr(from)?;
    let dst = path_cstr(to)?;
    let mut buf = [0u8; 256];
    let len = unsafe {
        libc::getxattr(
            src.as_ptr(),
            NAME.as_ptr().cast(),
            buf.as_mut_ptr().cast(),
            buf.len(),
        )
    };
    if len < 0 {
        return Err(io::Error::last_os_error());
    }
    let rc = unsafe {
        libc::lsetxattr(
            dst.as_ptr(),
            NAME.as_ptr().cast(),
            buf.as_ptr().cast(),
            len as usize,
            0,
        )
    };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
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
    fn app_owner_admits_root_and_the_dir_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let me = owner_uid(dir.path()).unwrap();
        let list = Allowlist::AppOwner(dir.path().to_path_buf());
        assert!(list.permits(0));
        assert!(list.permits(me));
        assert!(!list.permits(me.wrapping_add(1)));
        // A missing app dir (APK uninstalled) admits only root.
        let gone = Allowlist::AppOwner(dir.path().join("absent"));
        assert!(gone.permits(0));
        assert!(!gone.permits(me) || me == 0);
    }

    #[test]
    fn app_socket_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("s.sock");
        let _l = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        apply_app_socket_permissions(&sock, dir.path()).unwrap();
        let mode = fs::symlink_metadata(&sock).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn parse_env_list() {
        let set = parse_uid_list("10123, 10124\n10125");
        assert_eq!(set, BTreeSet::from([10123, 10124, 10125]));
    }
}
