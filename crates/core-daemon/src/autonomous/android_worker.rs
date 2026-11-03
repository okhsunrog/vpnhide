//! Fixed Android service adapter. Only the dedicated worker is stopped; the UI
//! package is never force-stopped and an arbitrary PID is never signalled.
use super::boot_time::{BootDeadline, BootInstant};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::Duration;
use vpnhide_loader_ipc::{WORKER_PROCESS, image};

static OWNER: std::sync::Mutex<()> = std::sync::Mutex::new(());
pub(crate) fn lease() -> Result<std::sync::MutexGuard<'static, ()>, String> {
    OWNER
        .try_lock()
        .map_err(|_| "dedicated Android worker is owned by another run".into())
}

const COMPONENT: &str = "dev.okhsunrog.vpnhide/.diagnostics.LoaderProbeService";

fn run(args: &[&str]) -> Result<(), String> {
    let mut command = std::process::Command::new("/system/bin/cmd");
    command.args(args);
    let output = vpnhide_activator::output_with_timeout(&mut command, Duration::from_secs(2))
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "Android worker command refused: {}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(256)
                .collect::<String>()
        ));
    }
    Ok(())
}
fn existing(app_dir: &Path) -> Result<bool, String> {
    let uid = fs::metadata(app_dir).map_err(|e| e.to_string())?.uid();
    if uid < 10_000 {
        return Err("invalid Android worker owner".into());
    }
    for (count, entry) in fs::read_dir("/proc")
        .map_err(|e| e.to_string())?
        .enumerate()
    {
        if count >= 65_536 {
            return Err("process inventory exceeds bound".into());
        }
        let path = entry.map_err(|e| e.to_string())?.path();
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
            Err(e) => return Err(e.to_string()),
        };
        if metadata.uid() != uid {
            continue;
        }
        let name = match image::bounded_text(&path.join("cmdline"), 512) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.to_string()),
        };
        if name.split('\0').next() == Some(WORKER_PROCESS) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn prepare_fresh(app_dir: &Path) -> Result<(), String> {
    if existing(app_dir)? {
        // AOSP returns nonzero even for its "Service stopped" branch. Command
        // status is not exit evidence: observe actual dedicated-process absence.
        let stopped = run(&["activity", "stop-service", "--user", "0", "-n", COMPONENT]);
        let deadline = BootDeadline::from_now(Duration::from_secs(2)).map_err(|e| e.to_string())?;
        while existing(app_dir)? {
            if deadline.expired(BootInstant::now().map_err(|e| e.to_string())?) {
                return Err(format!(
                    "previous dedicated diagnostic process did not exit: {stopped:?}"
                ));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    Ok(())
}

pub fn launch() -> Result<(), String> {
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
