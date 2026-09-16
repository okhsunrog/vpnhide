use std::fs::File;
use std::io::{self, Seek, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::output::Output;
use super::state::{Status, Store};

/// A separate executable, never the JNI/app process: waitpid(-1) belongs exclusively to this supervisor.
pub fn execute(
    store: &mut Store,
    sequence: u64,
    script: &[u8],
    timeout: Duration,
) -> io::Result<()> {
    // Root managers can pass ignored SIGCHLD; that would discard exit status and defeat waitpid evidence.
    if unsafe { libc::signal(libc::SIGCHLD, libc::SIG_DFL) } == libc::SIG_ERR
        || unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1 as libc::c_ulong, 0, 0, 0) } != 0
    {
        return Err(io::Error::last_os_error());
    }
    let input = script_input(script)?;
    let helper = std::env::current_exe()?;
    let (reader, writer) = UnixStream::pair()?;
    let mut output = Output::new(reader)?;
    let stdout = Stdio::from(OwnedFd::from(writer.try_clone()?));
    let stderr = Stdio::from(OwnedFd::from(writer));
    let mut running = store.state.clone();
    running.sequence = sequence;
    running.status = Status::Running;
    running.exit_code = None;
    running.descendant_failed = false;
    running.native_capacity = None;
    // Write-ahead marker must be durable BEFORE any possible fork/exec side effect.
    store.replace(running)?;
    #[cfg(target_os = "android")]
    let shell = "/system/bin/sh";
    #[cfg(not(target_os = "android"))]
    let shell = "/bin/sh";
    let child = Command::new(shell)
        .env("VPNHIDE_MUTATION_HELPER", helper)
        .stdin(input)
        .stdout(stdout)
        .stderr(stderr)
        .spawn();
    let child = match child {
        Ok(child) => child,
        Err(_) => {
            let mut state = store.state.clone();
            state.status = Status::NotStarted;
            return store.replace(state);
        }
    };
    // Children read the script via an anonymous file, not argv or a durable secret-bearing staging file.
    let result = drain(child.id() as libc::pid_t, timeout, &mut output)?;
    if let Some((exit_code, descendant_failed)) = result {
        let mut state = store.state.clone();
        state.status = Status::Finished;
        state.exit_code = Some(exit_code);
        state.descendant_failed = descendant_failed;
        state.native_capacity = output.warning;
        store.replace(state)?;
    }
    // Deadline leaves Running durable. A released flock or absent PID cannot clear it in this boot.
    Ok(())
}

fn script_input(script: &[u8]) -> io::Result<Stdio> {
    // bionic exposes the wrapper only from API 30; the syscall exists on every supported kernel.
    let fd = unsafe {
        libc::syscall(
            libc::SYS_memfd_create,
            c"vpnhide-command".as_ptr(),
            libc::MFD_CLOEXEC as libc::c_long,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut file = unsafe { File::from_raw_fd(fd as libc::c_int) };
    file.write_all(script)?;
    file.rewind()?;
    Ok(Stdio::from(file))
}

fn drain(
    leader: libc::pid_t,
    timeout: Duration,
    output: &mut Output,
) -> io::Result<Option<(i32, bool)>> {
    let deadline = Instant::now() + timeout;
    let mut leader_exit = None;
    let mut descendant_failed = false;
    loop {
        let output_closed = output.drain()?;
        let mut status = 0;
        let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
        if pid > 0 {
            let code = if libc::WIFEXITED(status) {
                libc::WEXITSTATUS(status)
            } else {
                128 + libc::WTERMSIG(status)
            };
            if pid == leader {
                leader_exit = Some(code);
            } else {
                descendant_failed |= code != 0;
            }
        } else if pid < 0 {
            let error = io::Error::last_os_error();
            match error.raw_os_error() {
                Some(libc::ECHILD) if output_closed => {
                    return Ok(leader_exit.map(|code| (code, descendant_failed)));
                }
                Some(libc::ECHILD) => (),
                Some(libc::EINTR) => continue,
                _ => return Err(error),
            }
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        if pid <= 0 {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
