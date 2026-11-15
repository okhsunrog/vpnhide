//! Fixed Android service adapter. Only the dedicated worker is stopped; the UI
//! package is never force-stopped and an arbitrary PID is never signalled.
use super::boot_time::{BootDeadline, BootInstant};
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::Duration;
use vpnhide_loader_ipc::{WORKER_PROCESS, image};

static OWNER: std::sync::Mutex<()> = std::sync::Mutex::new(());
pub(crate) fn lease() -> io::Result<std::sync::MutexGuard<'static, ()>> {
    OWNER.try_lock().map_err(|error| match error {
        std::sync::TryLockError::WouldBlock => io::Error::new(
            io::ErrorKind::WouldBlock,
            "dedicated Android worker is owned by another run",
        ),
        std::sync::TryLockError::Poisoned(_) => {
            io::Error::other("dedicated Android worker owner poisoned")
        }
    })
}

/// Boot probing waits for an existing autonomous run rather than converting
/// ordinary worker contention into unavailable backend evidence. Off-writer only.
pub(crate) fn lease_before(
    deadline: BootDeadline,
) -> Result<std::sync::MutexGuard<'static, ()>, String> {
    lease_from_before(&OWNER, deadline)
}
fn lease_from_before(
    owner: &std::sync::Mutex<()>,
    deadline: BootDeadline,
) -> Result<std::sync::MutexGuard<'_, ()>, String> {
    loop {
        if deadline.expired(BootInstant::now().map_err(|e| e.to_string())?) {
            return Err("dedicated Android worker lease deadline expired".into());
        }
        match owner.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(std::sync::TryLockError::Poisoned(_)) => {
                return Err("dedicated Android worker owner poisoned".into());
            }
            Err(std::sync::TryLockError::WouldBlock) => {
                std::thread::sleep(Duration::from_millis(25))
            }
        }
    }
}

const COMPONENT: &str = "dev.okhsunrog.vpnhide/.diagnostics.LoaderProbeService";

fn run(args: &[&str]) -> io::Result<()> {
    let mut command = std::process::Command::new("/system/bin/cmd");
    command.args(args);
    let name = args[..2].join(" ");
    let output = vpnhide_activator::output_with_timeout(&mut command, Duration::from_secs(2))
        .map_err(|error| io::Error::new(error.kind(), format!("Android worker {name}: {error}")))?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "Android worker {name} refused: {}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(256)
                .collect::<String>()
        )));
    }
    Ok(())
}
fn existing(app_dir: &Path) -> io::Result<bool> {
    let uid = fs::metadata(app_dir)?.uid();
    if uid < 10_000 {
        return Err(io::Error::other("invalid Android worker owner"));
    }
    for (count, entry) in fs::read_dir("/proc")?.enumerate() {
        if count >= 65_536 {
            return Err(io::Error::other("process inventory exceeds bound"));
        }
        let path = entry?.path();
        if path
            .file_name()
            .and_then(|s| s.to_str())
            .is_none_or(|s| !s.bytes().all(|b| b.is_ascii_digit()))
        {
            continue;
        }
        let metadata = match fs::metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        if metadata.uid() != uid {
            continue;
        }
        let name = match image::bounded_text(&path.join("cmdline"), 512) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        if name.split('\0').next() == Some(WORKER_PROCESS) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn prepare_fresh(app_dir: &Path) -> io::Result<()> {
    if existing(app_dir)? {
        // AOSP returns nonzero even for its "Service stopped" branch. Command
        // status is not exit evidence: observe actual dedicated-process absence.
        let stopped = run(&["activity", "stop-service", "--user", "0", "-n", COMPONENT]);
        let deadline = BootDeadline::from_now(Duration::from_secs(2))?;
        while existing(app_dir)? {
            if deadline.expired(BootInstant::now()?) {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("previous dedicated diagnostic process did not exit: {stopped:?}"),
                ));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    Ok(())
}

pub fn launch() -> io::Result<()> {
    run(&[
        "deviceidle",
        "tempwhitelist",
        "-u",
        "0",
        "-d",
        "5000",
        "dev.okhsunrog.vpnhide",
    ])?;
    run(&["activity", "start-service", "--user", "0", "-n", COMPONENT])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    #[test]
    fn waiting_worker_lease_observes_release_without_busy_and_never_grants_after_deadline() {
        let owner = Mutex::new(());
        let held = owner.lock().unwrap();
        assert!(
            lease_from_before(&owner, BootDeadline::from_now(Duration::ZERO).unwrap()).is_err()
        );
        std::thread::scope(|scope| {
            let (ready, received) = std::sync::mpsc::channel();
            let owner = &owner;
            let waiter = scope.spawn(move || {
                ready.send(()).unwrap();
                lease_from_before(
                    owner,
                    BootDeadline::from_now(Duration::from_secs(2)).unwrap(),
                )
                .is_ok()
            });
            received.recv().unwrap();
            drop(held);
            assert!(waiter.join().unwrap());
        });
        assert!(
            lease_from_before(&owner, BootDeadline::from_now(Duration::ZERO).unwrap()).is_err()
        );
    }
    #[test]
    fn poisoned_worker_owner_is_an_error_instead_of_waiting_or_stealing() {
        let owner = Mutex::new(());
        std::thread::scope(|scope| {
            let _ = scope
                .spawn(|| {
                    let _held = owner.lock().unwrap();
                    panic!("poison fixture");
                })
                .join();
        });
        assert!(
            lease_from_before(
                &owner,
                BootDeadline::from_now(Duration::from_secs(1)).unwrap()
            )
            .unwrap_err()
            .contains("poisoned")
        );
    }
}
