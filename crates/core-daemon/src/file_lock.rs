//! Nonblocking exclusion owned by a Rust scope, not by inherited pre-exec FDs.
use std::{fs::File, io, os::fd::AsRawFd};

pub(crate) struct FileLock {
    file: File,
    owner: libc::pid_t,
}
impl FileLock {
    /// Callers open an independent description and validate its inode/owner
    /// before admission. The guard never deletes or replaces the lock inode.
    pub fn exclusive(file: File) -> io::Result<Self> {
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            file,
            owner: unsafe { libc::getpid() },
        })
    }
}
impl Drop for FileLock {
    fn drop(&mut self) {
        // CLOEXEC has not run in a fork child yet. Explicit unlock ends the
        // logical owner's scope even while a child retains that description.
        // A child's copy must never revoke its still-live parent's exclusion.
        if unsafe { libc::getpid() } == self.owner {
            unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dropping_an_inherited_child_guard_does_not_unlock_its_parent() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("owner.lock");
        let open = || {
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)
                .unwrap()
        };
        let guard = FileLock::exclusive(open()).unwrap();
        // SAFETY: child Drop only calls getpid/close; _exit bypasses unrelated
        // Rust destructors in this multithreaded test process.
        let child = unsafe { libc::fork() };
        assert!(child >= 0);
        if child == 0 {
            drop(guard);
            unsafe { libc::_exit(0) };
        }
        let mut status = 0;
        while unsafe { libc::waitpid(child, &mut status, 0) } < 0 {
            assert_eq!(
                io::Error::last_os_error().kind(),
                io::ErrorKind::Interrupted
            );
        }
        assert!(libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0);
        assert!(
            matches!(FileLock::exclusive(open()), Err(e) if e.kind() == io::ErrorKind::WouldBlock)
        );
        drop(guard);
        assert!(FileLock::exclusive(open()).is_ok());
    }
}
