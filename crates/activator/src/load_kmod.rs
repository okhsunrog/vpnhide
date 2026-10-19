//! Kernel module loading and the timed child-process helper for `kpatch`.

use std::io::{ErrorKind, Read};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub(crate) const CHILD_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);

/// Load the kmod artifact. On device this is `finit_module(2)` (no `insmod`, no shell);
/// the error is `(errno, message)`. Host builds have no module loader to call, so the
/// integration tests' orchestration runs against an `insmod` test double on `PATH`.
#[cfg(target_os = "android")]
pub(crate) fn load_kmod_artifact(
    artifact: &Path,
    filesystem_hiding: bool,
) -> Result<(), (i32, String)> {
    let params = format!("filesystem_hiding={}", u8::from(filesystem_hiding));
    crate::sys::finit_module(artifact, &params).map_err(|e| {
        (
            e.raw_os_error().unwrap_or(-1),
            format!("finit_module {}: {e}", artifact.display()),
        )
    })
}

#[cfg(not(target_os = "android"))]
pub(crate) fn load_kmod_artifact(
    artifact: &Path,
    filesystem_hiding: bool,
) -> Result<(), (i32, String)> {
    let mut command = Command::new("insmod");
    command
        .arg(artifact)
        .arg(format!("filesystem_hiding={}", u8::from(filesystem_hiding)));
    match output_with_timeout(&mut command, CHILD_COMMAND_TIMEOUT) {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err((
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )),
        Err(e) if e.kind() == ErrorKind::TimedOut => Err((
            124,
            format!(
                "insmod timed out after {}s",
                CHILD_COMMAND_TIMEOUT.as_secs()
            ),
        )),
        Err(e) => Err((126, e.to_string())),
    }
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
