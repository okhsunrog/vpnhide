//! IPC authorization.
//!
//! Production (2.0): the socket lives in the app's data dir with the app's owner, mode and
//! SELinux label ([`apply_app_socket_permissions`]), and every accept is checked with
//! `SO_PEERCRED` against [`Allowlist::AppOwner`]: root or the uid owning that dir. The app
//! writes nothing to configure this. See `app_socket`.
//!
//! Private sockets default to the current UID and 0600; explicit policies are
//! validated by `ipc_config` before bind. Open exists only for explicit library tests.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use libc::{SO_PEERCRED, SOL_SOCKET, getsockopt, socklen_t, ucred};

/// Env: comma-separated UIDs allowed to connect.
pub const ALLOW_UIDS_ENV: &str = "VPNHIDE_CORE_ALLOW_UIDS";
/// Env: socket owner uid after bind (`chown`); unset leaves creator ownership.
pub const SOCKET_OWNER_UID_ENV: &str = "VPNHIDE_CORE_SOCKET_OWNER_UID";
/// Env: explicitly requested socket mode octal (default `0600`).
pub const SOCKET_MODE_ENV: &str = "VPNHIDE_CORE_SOCKET_MODE";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Allowlist {
    /// Explicit unrestricted library policy for tests; never selected by environment parsing.
    Open,
    /// Only these uids may connect.
    Uids(BTreeSet<u32>),
    /// Production (2.0): root, and the uid that owns the app's data dir. Re-resolved on
    /// every accept, so an APK reinstall (new uid) is followed without a restart.
    AppOwner(PathBuf),
}

impl Allowlist {
    pub fn permits(&self, uid: u32) -> bool {
        match self {
            Allowlist::Open => true,
            Allowlist::Uids(set) => set.contains(&uid),
            Allowlist::AppOwner(dir) => uid == 0 || owner_uid(dir).is_ok_and(|owner| owner == uid),
        }
    }
}

pub(crate) fn parse_uid_list(raw: &str) -> io::Result<BTreeSet<u32>> {
    let set: BTreeSet<_> = raw
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|token| !token.is_empty())
        .map(parse_uid)
        .collect::<io::Result<_>>()?;
    if set.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "empty UID allowlist",
        ));
    }
    Ok(set)
}

pub(crate) fn parse_uid(raw: &str) -> io::Result<u32> {
    // uid_t::MAX is the chown "leave unchanged" sentinel, never an owner.
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid UID"));
    }
    raw.parse::<u32>()
        .ok()
        .filter(|uid| *uid != u32::MAX)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid UID"))
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
/// Missing SELinux xattrs are allowed on hosts/disabled kernels; a known app
/// label must match, and already-correct labels do not require relabel permission.
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
    let actual = fs::metadata(socket)?;
    if actual.uid() != meta.uid() || actual.gid() != meta.gid() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "socket owner does not match app directory",
        ));
    }
    copy_selinux_label(app_dir, socket)
}

/// Give `path` the uid/gid and SELinux label of `owner_dir` (an app data dir), so only that
/// app can read what core wrote for it. Ownership needs root; the label copy is best-effort.
pub fn hand_to_owner_of(path: &Path, owner_dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let meta = fs::metadata(owner_dir)?;
    if unsafe { libc::geteuid() } == 0 {
        let c = path_cstr(path)?;
        if unsafe { libc::lchown(c.as_ptr(), meta.uid(), meta.gid()) } < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    let _ = copy_selinux_label(owner_dir, path);
    Ok(())
}

fn read_selinux_label(path: &Path) -> io::Result<Vec<u8>> {
    let path = path_cstr(path)?;
    let mut buffer = vec![0u8; 4096];
    let len = unsafe {
        libc::getxattr(
            path.as_ptr(),
            c"security.selinux".as_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
        )
    };
    if len < 0 {
        return Err(io::Error::last_os_error());
    }
    buffer.truncate(len as usize);
    Ok(buffer)
}

fn copy_selinux_label(from: &Path, to: &Path) -> io::Result<()> {
    // Missing xattr support is normal on hosts or an Android kernel without
    // SELinux. A mounted Android SELinux policy (also permissive) keeps labels.
    let optional = !cfg!(target_os = "android")
        || matches!(
            fs::metadata("/sys/fs/selinux/enforce"), Err(error) if error.kind() == io::ErrorKind::NotFound
        );
    ensure_label(
        read_selinux_label(from),
        optional,
        || read_selinux_label(to),
        |label| {
            let dst = path_cstr(to)?;
            let rc = unsafe {
                libc::lsetxattr(
                    dst.as_ptr(),
                    c"security.selinux".as_ptr(),
                    label.as_ptr().cast(),
                    label.len(),
                    0,
                )
            };
            if rc < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        },
    )
}

fn unavailable_label(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::ENODATA | libc::EOPNOTSUPP | libc::ENOSYS)
    )
}

fn ensure_label(
    source: io::Result<Vec<u8>>,
    optional: bool,
    mut read_target: impl FnMut() -> io::Result<Vec<u8>>,
    write: impl FnOnce(&[u8]) -> io::Result<()>,
) -> io::Result<()> {
    let source = match source {
        Ok(label) => label,
        Err(error) if optional && unavailable_label(&error) => return Ok(()),
        Err(error) => return Err(error),
    };
    match read_target() {
        Ok(current) if current == source => return Ok(()),
        Ok(_) => {}
        Err(error) if unavailable_label(&error) => {}
        Err(error) => return Err(error),
    }
    write(&source)?;
    if read_target()? != source {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "socket SELinux label did not match app directory after preparation",
        ));
    }
    Ok(())
}

/// Apply the already validated private endpoint settings. No ambient env reads.
pub(crate) fn apply_private_socket_permissions(
    path: &Path,
    mode: u32,
    owner: Option<u32>,
) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(uid) = owner {
        if unsafe { libc::chown(path_cstr(path)?.as_ptr(), uid, uid) } < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
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
        let set = parse_uid_list("10123, 10124\n10125").unwrap();
        assert_eq!(set, BTreeSet::from([10123, 10124, 10125]));
    }
}

#[cfg(test)]
mod label_tests {
    use super::*;
    #[test]
    fn missing_host_xattrs_are_optional_but_android_policy_is_not() {
        for errno in [libc::ENODATA, libc::EOPNOTSUPP, libc::ENOSYS] {
            assert!(
                ensure_label(
                    Err(io::Error::from_raw_os_error(errno)),
                    true,
                    || panic!("no target read"),
                    |_| panic!("no write")
                )
                .is_ok()
            );
            assert!(
                ensure_label(
                    Err(io::Error::from_raw_os_error(errno)),
                    false,
                    || panic!("no target read"),
                    |_| panic!("no write")
                )
                .is_err()
            );
        }
        assert!(
            ensure_label(
                Err(io::Error::from_raw_os_error(libc::EACCES)),
                true,
                || panic!("no target read"),
                |_| panic!("no write")
            )
            .is_err()
        );
    }
    #[test]
    fn already_correct_label_does_not_require_relabel_permission() {
        assert!(
            ensure_label(
                Ok(b"app-label".to_vec()),
                false,
                || Ok(b"app-label".to_vec()),
                |_| panic!("already correct label must not be written")
            )
            .is_ok()
        );
    }
    #[test]
    fn incorrect_label_requires_successful_verified_repair() {
        assert!(
            ensure_label(
                Ok(b"app".to_vec()),
                false,
                || Ok(b"wrong".to_vec()),
                |_| Err(io::Error::from_raw_os_error(libc::EPERM))
            )
            .is_err()
        );
        assert!(
            ensure_label(
                Ok(b"app".to_vec()),
                false,
                || Ok(b"wrong".to_vec()),
                |_| Ok(())
            )
            .is_err()
        );
        let label = std::cell::RefCell::new(b"wrong".to_vec());
        assert!(
            ensure_label(
                Ok(b"app".to_vec()),
                false,
                || Ok(label.borrow().clone()),
                |bytes| {
                    *label.borrow_mut() = bytes.to_vec();
                    Ok(())
                }
            )
            .is_ok()
        );
    }
    #[test]
    fn app_owner_follows_changed_uid_without_rebuilding_policy() {
        use std::os::unix::fs::MetadataExt;
        if unsafe { libc::geteuid() } != 0 {
            // Exercise real metadata resolution without requiring host chown:
            // replace a fixture directory link with a root-owned directory.
            let dir = tempfile::tempdir().unwrap();
            let app = dir.path().join("app");
            std::os::unix::fs::symlink(dir.path(), &app).unwrap();
            let uid = fs::metadata(dir.path()).unwrap().uid();
            let policy = Allowlist::AppOwner(app.clone());
            assert!(policy.permits(uid));
            fs::remove_file(&app).unwrap();
            std::os::unix::fs::symlink("/", &app).unwrap();
            assert!(!policy.permits(uid));
            assert!(policy.permits(0));
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let old = fs::metadata(dir.path()).unwrap();
        let policy = Allowlist::AppOwner(dir.path().into());
        let c = path_cstr(dir.path()).unwrap();
        assert_eq!(unsafe { libc::chown(c.as_ptr(), 12345, 12345) }, 0);
        assert!(policy.permits(12345));
        assert!(!policy.permits(12346));
        assert_eq!(unsafe { libc::chown(c.as_ptr(), 12346, 12346) }, 0);
        assert!(!policy.permits(12345));
        assert!(policy.permits(12346));
        assert!(policy.permits(0));
        assert_eq!(unsafe { libc::chown(c.as_ptr(), old.uid(), old.gid()) }, 0);
    }
}
