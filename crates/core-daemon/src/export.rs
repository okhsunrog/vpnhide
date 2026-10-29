//! File publication across the root/app boundary. Root never reopens a destination
//! by path after giving it to the app; all writes and metadata use pinned fds.
use std::ffi::CString;
use std::fs::{self, File};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};

use crate::diagnostics::{EXPORT_DIR_NAME, ProbeEnv};

fn name(value: &std::ffi::OsStr) -> io::Result<CString> {
    CString::new(value.as_bytes()).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
}

fn open_at(dir: &File, child: &std::ffi::OsStr, flags: i32) -> io::Result<File> {
    let child = name(child)?;
    // SAFETY: valid directory fd and terminated name; returned fd is owned here.
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            child.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn directory(path: &Path) -> io::Result<File> {
    let mut dir = File::open(if path.is_absolute() { "/" } else { "." })?;
    for part in path.components() {
        match part {
            Component::Normal(child) => {
                dir = open_at(&dir, child, libc::O_RDONLY | libc::O_DIRECTORY)?
            }
            Component::RootDir | Component::CurDir => {}
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "invalid export directory",
                ));
            }
        }
    }
    Ok(dir)
}

fn hand_to_app(file: &File, owner: &File, mode: u32) -> io::Result<()> {
    let meta = owner.metadata()?;
    if unsafe { libc::geteuid() } == 0
        && unsafe { libc::fchown(file.as_raw_fd(), meta.uid(), meta.gid()) } != 0
    {
        return Err(io::Error::last_os_error());
    }
    // Android requires the app's MLS categories as well as its DAC ownership.
    #[cfg(target_os = "android")]
    {
        let mut label = [0u8; 256];
        let attr = c"security.selinux";
        let len = unsafe {
            libc::fgetxattr(
                owner.as_raw_fd(),
                attr.as_ptr(),
                label.as_mut_ptr().cast(),
                label.len(),
            )
        };
        if len < 0 {
            return Err(io::Error::last_os_error());
        }
        if unsafe {
            libc::fsetxattr(
                file.as_raw_fd(),
                attr.as_ptr(),
                label.as_ptr().cast(),
                len as usize,
                0,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    file.set_permissions(fs::Permissions::from_mode(mode))
}

pub(crate) struct ExportDir {
    dir: File,
    owner: File,
    path: PathBuf,
}

impl ExportDir {
    pub(crate) fn open(env: &ProbeEnv) -> io::Result<Self> {
        let app = env
            .app_dir
            .as_deref()
            .ok_or_else(|| io::Error::other("no app data dir"))?;
        let owner = directory(app)?;
        let child = CString::new(EXPORT_DIR_NAME).expect("constant name");
        let result = unsafe { libc::mkdirat(owner.as_raw_fd(), child.as_ptr(), 0o700) };
        if result != 0 && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists {
            return Err(io::Error::last_os_error());
        }
        let dir = open_at(
            &owner,
            std::ffi::OsStr::new(EXPORT_DIR_NAME),
            libc::O_RDONLY | libc::O_DIRECTORY,
        )?;
        hand_to_app(&dir, &owner, 0o700)?;
        Ok(Self {
            dir,
            owner,
            path: app.join(EXPORT_DIR_NAME),
        })
    }

    pub(crate) fn subdir(&self, name: &str) -> io::Result<Self> {
        let child = CString::new(name).map_err(io::Error::other)?;
        if unsafe { libc::mkdirat(self.dir.as_raw_fd(), child.as_ptr(), 0o700) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let dir = open_at(
            &self.dir,
            std::ffi::OsStr::new(name),
            libc::O_RDONLY | libc::O_DIRECTORY,
        )?;
        hand_to_app(&dir, &self.owner, 0o700)?;
        Ok(Self {
            dir,
            owner: self.owner.try_clone()?,
            path: self.path.join(name),
        })
    }

    pub(crate) fn write(
        &self,
        filename: &str,
        write: impl FnOnce(&mut File) -> io::Result<()>,
    ) -> io::Result<PathBuf> {
        if Path::new(filename).components().count() != 1
            || filename == "."
            || filename == ".."
            || filename.contains('/')
        {
            return Err(io::Error::other("invalid export name"));
        }
        let mut file = open_at(
            &self.dir,
            std::ffi::OsStr::new(filename),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        )?;
        let result = write(&mut file)
            .and_then(|()| file.sync_all())
            .and_then(|()| hand_to_app(&file, &self.owner, 0o600));
        if let Err(error) = result {
            self.remove(filename);
            return Err(error);
        }
        Ok(self.path.join(filename))
    }

    pub(crate) fn remove(&self, filename: &str) {
        if filename.contains('/') {
            return;
        }
        if let Ok(child) = CString::new(filename) {
            unsafe {
                libc::unlinkat(self.dir.as_raw_fd(), child.as_ptr(), 0);
            }
        }
    }

    pub(crate) fn names(&self) -> io::Result<Vec<String>> {
        // /proc resolves our own pinned descriptor, not an app-controlled parent.
        Ok(
            fs::read_dir(format!("/proc/self/fd/{}", self.dir.as_raw_fd()))?
                .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
                .collect(),
        )
    }
}

/// Random 0700 root-owned staging outside the app tree. TempDir cleans up on
/// normal completion, probe timeout, errors and unwinding. Production startup
/// removes interrupted staging under its exclusive daemon ownership lock.
pub(crate) fn staging() -> io::Result<tempfile::TempDir> {
    #[cfg(target_os = "android")]
    let root = crate::ownership::owner_dir().join("export-staging");
    #[cfg(not(target_os = "android"))]
    let root = std::env::temp_dir();
    fs::create_dir_all(&root)?;
    tempfile::Builder::new()
        .prefix("vpnhide-probe-")
        .tempdir_in(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::symlink;

    #[test]
    fn symlink_destination_and_parent_never_modify_outside_file() {
        let temp = tempfile::tempdir().unwrap();
        let env = ProbeEnv {
            core_exe: "unused".into(),
            app_dir: Some(temp.path().join("app")),
        };
        fs::create_dir(env.app_dir.as_ref().unwrap()).unwrap();
        let target = temp.path().join("outside");
        fs::write(&target, "untouched").unwrap();
        let export = ExportDir::open(&env).unwrap();
        symlink(&target, export.path.join("evil")).unwrap();
        assert!(export.write("evil", |f| f.write_all(b"bad")).is_err());
        let old = temp.path().join("old");
        fs::rename(&export.path, &old).unwrap();
        symlink(temp.path(), &export.path).unwrap();
        assert!(ExportDir::open(&env).is_err());
        export
            .write("outside", |f| f.write_all(b"safe new inode"))
            .unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "untouched");
        assert_eq!(
            fs::read_to_string(old.join("outside")).unwrap(),
            "safe new inode"
        );
    }
}
