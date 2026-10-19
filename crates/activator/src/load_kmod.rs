//! Timed child-process helpers for core-driven loads (insmod / kpatch).

use std::io::{ErrorKind, Read};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub(crate) const CHILD_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) fn load_kmod_artifact(
    artifact: &Path,
    filesystem_hiding: bool,
) -> std::io::Result<Output> {
    let mut command = Command::new("insmod");
    command
        .arg(artifact)
        .arg(format!("filesystem_hiding={}", u8::from(filesystem_hiding)));
    output_with_timeout(&mut command, CHILD_COMMAND_TIMEOUT)
}

pub(crate) fn output_with_timeout(
    command: &mut Command,
    timeout: Duration,
) -> std::io::Result<Output> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("child stdout missing"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| std::io::Error::other("child stderr missing"))?;
    let stdout_reader = thread::spawn(move || read_all(stdout));
    let stderr_reader = thread::spawn(move || read_all(stderr));
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Output {
                status,
                stdout: join_reader(stdout_reader)?,
                stderr: join_reader(stderr_reader)?,
            });
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = join_reader(stdout_reader);
            let _ = join_reader(stderr_reader);
            return Err(std::io::Error::new(
                ErrorKind::TimedOut,
                "command timed out",
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn read_all(mut reader: impl Read) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn join_reader(reader: thread::JoinHandle<std::io::Result<Vec<u8>>>) -> std::io::Result<Vec<u8>> {
    reader
        .join()
        .map_err(|_| std::io::Error::other("child output reader panicked"))?
}
