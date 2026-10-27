//! Lifetime ownership of the durable state. Locks are permanent inodes, separate
//! from atomically replaced JSON, and are acquired before reads, boot or socket bind.

use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

pub struct InstanceGuard {
    _locks: Vec<File>,
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        // fork() briefly shares the open-file description until exec closes it.
        // Explicit unlock makes ownership end with this guard, even if a child
        // still has an inherited descriptor during that window.
        for file in &self._locks {
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
        }
    }
}

fn lock_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".lock");
    PathBuf::from(name)
}

impl InstanceGuard {
    /// Both resources are protected: alternative journal paths cannot bypass the
    /// canonical writer lock, nor can alternative canonical paths share a journal.
    pub fn state(config: &Path, journal: &Path) -> io::Result<Self> {
        Self::acquire([lock_path(config), lock_path(journal)])
    }

    pub fn acquire(paths: impl IntoIterator<Item = PathBuf>) -> io::Result<Self> {
        let paths: BTreeSet<_> = paths.into_iter().collect();
        let mut locks = Vec::with_capacity(paths.len());
        for path in paths {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
                .open(&path)?;
            if !file.metadata()?.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "instance lock is not a regular file",
                ));
            }
            // SAFETY: this process owns the open descriptor. Nonblocking admission
            // never waits behind a second daemon and never deletes an owner's inode.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                let error = io::Error::last_os_error();
                return Err(io::Error::new(
                    error.kind(),
                    format!("core state already owned ({}): {error}", path.display()),
                ));
            }
            locks.push(file);
        }
        Ok(Self { _locks: locks })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inherited_pre_exec_descriptors_do_not_extend_guard_ownership() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config");
        let journal = dir.path().join("journal");
        let first = InstanceGuard::state(&config, &journal).unwrap();
        // SAFETY: the child only pauses in an async-signal-safe syscall. The
        // parent always kills and reaps it before inspecting the result.
        let child = unsafe { libc::fork() };
        assert!(child >= 0);
        if child == 0 {
            loop {
                unsafe { libc::pause() };
            }
        }
        drop(first);
        let replacement = InstanceGuard::state(&config, &journal);
        unsafe {
            libc::kill(child, libc::SIGKILL);
            while libc::waitpid(child, std::ptr::null_mut(), 0) < 0 {
                if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                    break;
                }
            }
        }
        assert!(
            replacement.is_ok(),
            "a child descriptor retained a released owner"
        );
    }

    #[test]
    fn shared_config_or_journal_refuses_a_second_writer_and_drop_releases() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.json");
        let journal = dir.path().join("operations.json");
        let first = InstanceGuard::state(&config, &journal).unwrap();
        assert!(InstanceGuard::state(&config, &dir.path().join("other-journal")).is_err());
        assert!(InstanceGuard::state(&dir.path().join("other-config"), &journal).is_err());
        drop(first);
        assert!(lock_path(&config).exists());
        assert!(lock_path(&journal).exists());
        InstanceGuard::state(&config, &journal).unwrap();
    }

    #[test]
    fn lock_symlinks_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        std::fs::write(&target, "unchanged").unwrap();
        let link = dir.path().join("lock");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(InstanceGuard::acquire([link]).is_err());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "unchanged");
    }
}
