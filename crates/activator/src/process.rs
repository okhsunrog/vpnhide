//! Bounded direct-exec children for backend and diagnostics work. One thread
//! drains all pipes with nonblocking IO; inherited pipe owners cannot strand
//! output-reader threads. The group leader is not reaped until its group is
//! terminated, so a group signal never targets a recycled PID.

use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

pub struct CapturedCommand {
    pub output: Output,
    pub timed_out: bool,
    pub truncated: bool,
}

struct ChildGroup {
    child: Child,
    reaped: bool,
}

impl ChildGroup {
    fn kill(&mut self) {
        // SAFETY: the unreaped leader still reserves this PID and process group.
        unsafe { libc::kill(-(self.child.id() as libc::pid_t), libc::SIGKILL) };
    }

    fn wait(&mut self) -> io::Result<std::process::ExitStatus> {
        let status = self.child.wait()?;
        // Once reaped, even querying this numeric PID could match a recycled child.
        self.reaped = true;
        Ok(status)
    }
}

impl Drop for ChildGroup {
    fn drop(&mut self) {
        // Only needed on an IO error; normal completion reaps after group cleanup.
        if !self.reaped && exited_unreaped(&self.child).is_ok() {
            self.kill();
            let _ = self.wait();
        }
    }
}

fn nonblocking(fd: &impl AsRawFd) -> io::Result<()> {
    let raw = fd.as_raw_fd();
    let flags = unsafe { libc::fcntl(raw, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(raw, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn exited_unreaped(child: &Child) -> io::Result<bool> {
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    // SAFETY: valid output pointer, only our own child, WNOWAIT retains its identity.
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            child.id(),
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result != 0 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            return Ok(false);
        }
        return Err(error);
    }
    Ok(unsafe { info.si_pid() } != 0)
}

fn drain(
    reader: &mut impl Read,
    output: &mut Vec<u8>,
    limit: usize,
    truncated: &mut bool,
) -> io::Result<()> {
    let mut buffer = [0u8; 16 * 1024];
    // A chatty process cannot starve deadline checks or the other stream.
    for _ in 0..64 {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                let keep = count.min(limit.saturating_sub(output.len()));
                output.extend_from_slice(&buffer[..keep]);
                *truncated |= keep < count;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub fn capture_command(
    command: &mut Command,
    input: Option<&[u8]>,
    timeout: Duration,
    output_limit: usize,
) -> io::Result<CapturedCommand> {
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let parent = unsafe { libc::getpid() };
    // SAFETY: only async-signal-safe syscalls before exec; no allocation/locks.
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) != 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::getppid() != parent {
                libc::_exit(1);
            }
            Ok(())
        });
    }
    let mut group = ChildGroup {
        child: command.spawn()?,
        reaped: false,
    };
    let mut stdin = group.child.stdin.take();
    let mut stdout = group
        .child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("stdout missing"))?;
    let mut stderr = group
        .child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("stderr missing"))?;
    if let Some(pipe) = &stdin {
        nonblocking(pipe)?;
    }
    nonblocking(&stdout)?;
    nonblocking(&stderr)?;
    let mut written = 0;
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut truncated = false;
    let deadline = Instant::now() + timeout;
    let timed_out = loop {
        if let (Some(pipe), Some(input)) = (&mut stdin, input) {
            match pipe.write(&input[written..]) {
                Ok(count) => written += count,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(error) if error.kind() == io::ErrorKind::BrokenPipe => {
                    stdin = None;
                }
                Err(error) => return Err(error),
            }
            if written == input.len() {
                stdin = None;
            }
        }
        drain(
            &mut stdout,
            &mut out,
            output_limit.saturating_sub(err.len()),
            &mut truncated,
        )?;
        drain(
            &mut stderr,
            &mut err,
            output_limit.saturating_sub(out.len()),
            &mut truncated,
        )?;
        if exited_unreaped(&group.child)? {
            break false;
        }
        if Instant::now() >= deadline {
            break true;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    group.kill();
    let status = group.wait()?;
    drain(
        &mut stdout,
        &mut out,
        output_limit.saturating_sub(err.len()),
        &mut truncated,
    )?;
    drain(
        &mut stderr,
        &mut err,
        output_limit.saturating_sub(out.len()),
        &mut truncated,
    )?;
    Ok(CapturedCommand {
        output: Output {
            status,
            stdout: out,
            stderr: err,
        },
        timed_out,
        truncated,
    })
}

pub fn output_with_input_timeout(
    command: &mut Command,
    input: Option<&[u8]>,
    timeout: Duration,
) -> io::Result<Output> {
    let captured = capture_command(command, input, timeout, 16 * 1024 * 1024)?;
    if captured.timed_out {
        return Err(io::Error::new(io::ErrorKind::TimedOut, "command timed out"));
    }
    if captured.truncated {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "command output exceeded limit",
        ));
    }
    Ok(captured.output)
}

pub fn output_with_timeout(command: &mut Command, timeout: Duration) -> io::Result<Output> {
    output_with_input_timeout(command, None, timeout)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(mode: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "process::tests::child_fixture", "--nocapture"])
            .env("VPNHIDE_PROCESS_FIXTURE", mode);
        command
    }

    #[test]
    fn child_fixture() {
        let Ok(mode) = std::env::var("VPNHIDE_PROCESS_FIXTURE") else {
            return;
        };
        match mode.as_str() {
            "echo" => {
                std::io::stderr()
                    .write_all(&vec![b'E'; 128 * 1024])
                    .unwrap();
                let mut input = Vec::new();
                std::io::stdin().read_to_end(&mut input).unwrap();
                std::io::stdout().write_all(&input).unwrap();
            }
            "both-streams" => {
                std::io::stdout().write_all(&vec![b'O'; 8192]).unwrap();
                std::io::stderr().write_all(&vec![b'E'; 8192]).unwrap();
            }
            "chatty" => std::io::stdout()
                .write_all(&vec![b'X'; 1024 * 1024])
                .unwrap(),
            "sleep" => std::thread::sleep(Duration::from_secs(30)),
            "orphan" => {
                // SAFETY: the fork child performs only async-signal-safe syscalls.
                let pid = unsafe { libc::fork() };
                assert!(pid >= 0);
                if pid == 0 {
                    unsafe {
                        libc::sleep(30);
                        libc::_exit(0);
                    }
                }
            }
            _ => panic!("invalid fixture"),
        }
        std::process::exit(0);
    }

    #[test]
    fn drains_output_while_writing_large_stdin_without_pipe_deadlock() {
        let input = vec![b'Z'; 512 * 1024];
        let out = capture_command(
            &mut fixture("echo"),
            Some(&input),
            Duration::from_secs(3),
            1024 * 1024,
        )
        .unwrap();
        assert!(!out.timed_out && !out.truncated);
        assert_eq!(
            out.output
                .stdout
                .iter()
                .filter(|&&byte| byte == b'Z')
                .count(),
            input.len()
        );
        assert_eq!(out.output.stderr.len(), 128 * 1024);
    }

    #[test]
    fn output_and_wall_time_are_bounded() {
        let out =
            capture_command(&mut fixture("chatty"), None, Duration::from_secs(3), 1024).unwrap();
        assert!(out.truncated);
        assert_eq!(out.output.stdout.len(), 1024);
        let start = Instant::now();
        let out =
            capture_command(&mut fixture("sleep"), None, Duration::from_millis(50), 1024).unwrap();
        assert!(out.timed_out);
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn stdout_and_stderr_share_one_retained_output_budget() {
        let out = capture_command(
            &mut fixture("both-streams"),
            None,
            Duration::from_secs(3),
            1024,
        )
        .unwrap();
        assert!(out.output.status.success());
        assert!(out.truncated);
        assert_eq!(out.output.stdout.len() + out.output.stderr.len(), 1024);
    }

    #[test]
    fn descendant_pipe_owners_cannot_keep_a_completed_command_alive() {
        let start = Instant::now();
        let out =
            capture_command(&mut fixture("orphan"), None, Duration::from_secs(3), 1024).unwrap();
        assert!(!out.timed_out);
        assert!(out.output.status.success());
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
