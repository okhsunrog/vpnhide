//! Kernel loading with explicit dispatch provenance. Numeric host status and
//! diagnostic text can never stand in for an actual Android syscall errno.
#[cfg(not(target_os = "android"))]
use crate::output_with_timeout;
use crate::{KmodLoadFailure, KmodLoadOrigin};
use std::path::Path;
use std::time::Duration;
#[cfg(not(target_os = "android"))]
use std::{io::ErrorKind, process::Command};
pub(crate) const CHILD_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(target_os = "android")]
pub(crate) fn load_kmod_artifact(
    artifact: &Path,
    filesystem_hiding: bool,
) -> Result<(), KmodLoadFailure> {
    use crate::sys::FinitModuleError;
    let params = format!("filesystem_hiding={}", u8::from(filesystem_hiding));
    crate::sys::finit_module(artifact, &params).map_err(|error| {
        let (origin, error) = match error {
            FinitModuleError::BeforeDispatch(error) => (KmodLoadOrigin::NotDispatched, error),
            FinitModuleError::Syscall(error) => (KmodLoadOrigin::AndroidFinitModuleErrno, error),
        };
        KmodLoadFailure {
            origin,
            exit: error.raw_os_error().unwrap_or(-1),
            stderr: format!("finit_module {}: {error}", artifact.display()),
        }
    })
}
#[cfg(not(target_os = "android"))]
pub(crate) fn load_kmod_artifact(
    artifact: &Path,
    filesystem_hiding: bool,
) -> Result<(), KmodLoadFailure> {
    let mut command = Command::new("insmod");
    command
        .arg(artifact)
        .arg(format!("filesystem_hiding={}", u8::from(filesystem_hiding)));
    let failure = |exit, stderr| KmodLoadFailure {
        origin: KmodLoadOrigin::HostProcessStatus,
        exit,
        stderr,
    };
    match output_with_timeout(&mut command, CHILD_COMMAND_TIMEOUT) {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(failure(
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )),
        Err(e) if e.kind() == ErrorKind::TimedOut => Err(failure(
            124,
            format!(
                "insmod timed out after {}s",
                CHILD_COMMAND_TIMEOUT.as_secs()
            ),
        )),
        Err(e) => Err(failure(126, e.to_string())),
    }
}
