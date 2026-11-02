//! Host-only ADB adapter. Ordinary cleanup is a Core FullReset operation. No
//! duplicated path wipe, offline writer, journal deletion or implicit retry.
use clap::Args as ClapArgs;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
use vpnhide_artifact_model::archive::read_regular;
use vpnhide_bounded_process::capture_command;
use vpnhide_maintenance_model::{DeviceIdentity, ResetPhase, ResetReply, ResetSession};
const CORE: &str = "/data/adb/modules/vpnhide_core/vpnhide-core";
const APP: &str = "dev.okhsunrog.vpnhide";
#[derive(ClapArgs)]
pub struct Args {
    /// Without this flag, print an offline plan and contact no device.
    #[arg(long,requires_all=["serial","state_file"])]
    execute: bool,
    #[arg(long)]
    serial: Option<String>,
    /// Private host receipt file. Resume it after timeout; never invent a new ID.
    #[arg(long)]
    state_file: Option<PathBuf>,
    #[arg(long, default_value = "adb")]
    adb: PathBuf,
    /// Remove the APK only after Core has confirmed its durable reset receipt.
    #[arg(long)]
    uninstall_app: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Uninstall {
    NotRequested,
    Requested,
    Intent,
    Confirmed,
    Unknown,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    session: ResetSession,
    uninstall: Uninstall,
    #[serde(skip)]
    current_confirmation: bool,
}
impl State {
    fn validate(&self) -> io::Result<()> {
        self.session.validate().map_err(io::Error::other)?;
        if matches!(
            self.uninstall,
            Uninstall::Intent | Uninstall::Confirmed | Uninstall::Unknown
        ) && self.session.phase != ResetPhase::Confirmed
        {
            return Err(io::Error::other(
                "APK removal requires a confirmed Core reset",
            ));
        }
        Ok(())
    }
}
struct Lease {
    path: PathBuf,
    _file: File,
}
impl Lease {
    fn open(path: &Path) -> io::Result<Self> {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent)?;
        let parent_meta = fs::symlink_metadata(parent)?;
        if !parent_meta.is_dir()
            || parent_meta.uid() != unsafe { libc::geteuid() }
            || parent_meta.mode() & 0o022 != 0
        {
            return Err(io::Error::other(
                "reset receipt directory must be owned and not writable by others",
            ));
        }
        let mut lock = path.as_os_str().to_os_string();
        lock.push(".lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(lock)?;
        let meta = file.metadata()?;
        if !meta.is_file()
            || meta.nlink() != 1
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.mode() & 0o077 != 0
        {
            return Err(io::Error::other("reset lease is not private and owned"));
        }
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            path: path.into(),
            _file: file,
        })
    }
    fn load(&self) -> io::Result<Option<State>> {
        let meta = match fs::symlink_metadata(&self.path) {
            Ok(m) => m,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        if !meta.is_file()
            || meta.nlink() != 1
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.mode() & 0o077 != 0
        {
            return Err(io::Error::other("reset receipt is not private and owned"));
        }
        let state: State = serde_json::from_slice(&read_regular(&self.path, 16 * 1024)?)
            .map_err(io::Error::other)?;
        state.validate()?;
        Ok(Some(state))
    }
    fn save(&self, state: &State) -> io::Result<()> {
        state.validate()?;
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let parent_dir = File::open(parent)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(&serde_json::to_vec(state).map_err(io::Error::other)?)?;
        temporary.as_file().sync_all()?;
        temporary.persist(&self.path).map_err(|e| e.error)?;
        parent_dir.sync_all()?;
        Ok(())
    }
}

trait Transport {
    fn identity(&mut self) -> io::Result<DeviceIdentity>;
    fn reset(
        &mut self,
        action: &str,
        operation: Option<u64>,
        expected_boot: &str,
    ) -> io::Result<ResetReply>;
    fn uninstall(&mut self) -> io::Result<()>;
}
struct Adb {
    program: PathBuf,
    serial: String,
}
fn remote_command(
    core: &str,
    action: &str,
    operation: Option<u64>,
    boot: &str,
) -> io::Result<String> {
    DeviceIdentity {
        serial: "native".into(),
        boot_id: boot.into(),
    }
    .validate()
    .map_err(io::Error::other)?;
    if !core.starts_with('/')
        || !core
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b))
        || !["allocate", "submit", "query"].contains(&action)
    {
        return Err(io::Error::other("invalid fixed native reset command"));
    }
    // ADB joins remote argv: pass one fully quoted su command. FD3 pins both
    // capability check and client execution to the same opened Core inode.
    // Old Core rejects the probe and never receives an unrecognized new verb.
    Ok(format!(
        "su -c 'exec 3<{core}; /proc/self/fd/3 probe maintenance-reset-ready {} && /proc/self/fd/3 maintenance-reset {action}{} {boot}'",
        vpnhide_maintenance_model::RESET_SCHEMA,
        operation.map(|op| format!(" {op}")).unwrap_or_default()
    ))
}
impl Adb {
    fn reset_at(
        &self,
        core: &str,
        action: &str,
        operation: Option<u64>,
        expected_boot: &str,
    ) -> io::Result<ResetReply> {
        let shell_command = remote_command(core, action, operation, expected_boot)?;
        let out = self.output(&["shell", &shell_command], Duration::from_secs(20))?;
        serde_json::from_slice(&out).map_err(io::Error::other)
    }
    fn output(&self, args: &[&str], timeout: Duration) -> io::Result<Vec<u8>> {
        let result = capture_command(
            Command::new(&self.program)
                .arg("-s")
                .arg(&self.serial)
                .args(args),
            None,
            timeout,
            64 * 1024,
        )?;
        if result.timed_out || result.truncated || !result.output.status.success() {
            return Err(io::Error::other(
                "bounded ADB command did not confirm completion",
            ));
        }
        Ok(result.output.stdout)
    }
}
impl Transport for Adb {
    fn identity(&mut self) -> io::Result<DeviceIdentity> {
        let out = self.output(
            &["shell", "cat", "/proc/sys/kernel/random/boot_id"],
            Duration::from_secs(10),
        )?;
        let device = DeviceIdentity {
            serial: self.serial.clone(),
            boot_id: std::str::from_utf8(&out)
                .map_err(io::Error::other)?
                .trim()
                .into(),
        };
        device.validate().map_err(io::Error::other)?;
        Ok(device)
    }
    fn reset(
        &mut self,
        action: &str,
        operation: Option<u64>,
        expected_boot: &str,
    ) -> io::Result<ResetReply> {
        // All remote command tokens are fixed literals or validated decimal IDs.
        // ADB/su is exclusively a host developer adapter, never APK/Core runtime.
        if !["allocate", "submit", "query"].contains(&action) {
            return Err(io::Error::other("unknown reset action"));
        }
        self.reset_at(CORE, action, operation, expected_boot)
    }
    fn uninstall(&mut self) -> io::Result<()> {
        let out = self.output(&["uninstall", APP], Duration::from_secs(30))?;
        if std::str::from_utf8(&out).map_err(io::Error::other)?.trim() != "Success" {
            return Err(io::Error::other("APK uninstall did not confirm success"));
        }
        Ok(())
    }
}
fn advance(lease: &Lease, transport: &mut impl Transport, uninstall: bool) -> io::Result<State> {
    let device = transport.identity()?;
    let mut state = match lease.load()? {
        Some(s) => {
            if s.session.device.serial != device.serial {
                return Err(io::Error::other("receipt belongs to another device"));
            }
            if (s.uninstall != Uninstall::NotRequested) != uninstall {
                return Err(io::Error::other(
                    "resume with the original uninstall choice",
                ));
            }
            s
        }
        None => {
            let state = State {
                current_confirmation: false,
                session: ResetSession::new(device.clone()).map_err(io::Error::other)?,
                uninstall: if uninstall {
                    Uninstall::Requested
                } else {
                    Uninstall::NotRequested
                },
            };
            lease.save(&state)?;
            state
        }
    };
    if state.session.phase == ResetPhase::Confirmed
        && matches!(
            state.uninstall,
            Uninstall::NotRequested | Uninstall::Requested
        )
    {
        state
            .session
            .recheck_confirmation()
            .map_err(io::Error::other)?;
        lease.save(&state)?;
    }
    if state.session.phase == ResetPhase::Prepared {
        if state.session.device != device {
            return Err(io::Error::other(
                "device rebooted before reset allocation; preserve receipt and prepare a new explicit session",
            ));
        }
        let reply = transport.reset("allocate", None, &state.session.device.boot_id)?;
        state.session.allocated(&reply).map_err(io::Error::other)?;
        lease.save(&state)?;
    }
    if state.session.phase == ResetPhase::Allocated {
        // Refresh the boot immediately before durable intent/dispatch.
        let current = transport.identity()?;
        let op = state
            .session
            .begin_submit(&current)
            .map_err(io::Error::other)?;
        lease.save(&state)?;
        match transport.reset("submit", Some(op), &state.session.device.boot_id) {
            Ok(reply) if state.session.observe(&reply).is_ok() => {}
            _ => state.session.lost_reply().map_err(io::Error::other)?,
        }
        lease.save(&state)?;
    }
    if matches!(
        state.session.phase,
        ResetPhase::SubmitIntent | ResetPhase::Waiting | ResetPhase::Unknown
    ) {
        match transport.reset(
            "query",
            state.session.operation,
            &state.session.device.boot_id,
        ) {
            Ok(reply) if state.session.observe(&reply).is_ok() => {}
            _ => state.session.lost_reply().map_err(io::Error::other)?,
        }
        lease.save(&state)?;
    }
    state.current_confirmation = state.session.phase == ResetPhase::Confirmed
        && matches!(
            state.uninstall,
            Uninstall::Requested | Uninstall::NotRequested
        );
    if state.session.phase == ResetPhase::Confirmed && state.uninstall == Uninstall::Requested {
        // A reboot after confirmation cannot make a different boot's running
        // hooks part of this reset's postcondition. Require an explicit review.
        if transport.identity()? != state.session.device {
            return Err(io::Error::other(
                "device rebooted; confirmed old reset is retained, APK removal not dispatched",
            ));
        }
        state.uninstall = Uninstall::Intent;
        lease.save(&state)?;
        state.uninstall = if transport.uninstall().is_ok() {
            Uninstall::Confirmed
        } else {
            Uninstall::Unknown
        };
        lease.save(&state)?;
    }
    if state.uninstall == Uninstall::Intent {
        state.uninstall = Uninstall::Unknown;
        lease.save(&state)?;
    }
    Ok(state)
}
pub fn run(args: Args) -> io::Result<()> {
    if !args.execute {
        println!(
            "{}",
            serde_json::json!({"action":"core_full_reset","execution":"explicit --execute --serial DEVICE --state-file PRIVATE_RECEIPT required","preserve":["Core journal and attempt budgets","module installations"],"uninstall_app":args.uninstall_app,"rescue":"If Core is unavailable or incompatible, retain the journal and install a matching Core/APK; no offline wipe is attempted."})
        );
        return Ok(());
    }
    let serial = args
        .serial
        .ok_or_else(|| io::Error::other("explicit serial required"))?;
    // Validate serial before starting any process; the real boot is read later.
    DeviceIdentity {
        serial: serial.clone(),
        boot_id: "00000000-0000-0000-0000-000000000000".into(),
    }
    .validate()
    .map_err(io::Error::other)?;
    let lease = Lease::open(
        &args
            .state_file
            .ok_or_else(|| io::Error::other("private receipt required"))?,
    )?;
    let mut adb = Adb {
        program: args.adb,
        serial,
    };
    let state = advance(&lease, &mut adb, args.uninstall_app)?;
    println!(
        "{}",
        serde_json::json!({"receipt":state,"proof":if state.current_confirmation{"current_rpc_readback"}else{"historical_or_unconfirmed"}})
    );
    if state.session.phase != ResetPhase::Confirmed
        || matches!(
            state.uninstall,
            Uninstall::Unknown | Uninstall::Intent | Uninstall::Requested
        )
    {
        return Err(io::Error::other(
            "reset/uninstall is not confirmed; retain this receipt and resume queries, never wipe the journal",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
