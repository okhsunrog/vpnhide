//! Core-owned full-system logcat recording (protocol v8).
//!
//! The app starts and stops a session; core runs the one `logcat -b all -v threadtime`
//! child (direct exec, never a shell) and owns everything about it:
//!
//! - **One session at a time.** Start with the running id is a no-op; another id is `Busy`.
//! - **Core-owned file.** stdout is piped through core into `<core dir>/logcat/`, so core
//!   counts every byte and the app can't touch the file while it grows.
//! - **Hard limits.** At [`MAX_BYTES`] core stops the child and marks the session truncated
//!   (stop, not ring: the file stays one contiguous stretch from the start marker, and the
//!   app tells the user it ended early). At [`MAX_DURATION`] core stops it too. Either way
//!   the session is `AutoStopped`, so a killed app can't leave root logcat running.
//! - **Dies with core.** The child gets `PR_SET_PDEATHSIG(SIGKILL)` against the supervisor
//!   thread that spawned it, and that thread lives until the child is reaped; a clean
//!   shutdown stops it explicitly. Files left by an earlier core run are removed at start.
//! - **Delivery.** Stop copies the file into the app's export dir (app-owned, 0600), the
//!   same route large probe output takes, never inline over the 1 MiB frame.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use vpnhide_core_rpc::{
    LogcatAutoStopReason, LogcatRecordingError, LogcatSessionId, LogcatSessionState,
    LogcatSessionStatus,
};

use crate::diagnostics::{ProbeEnv, hand_to_app, prepare_dir};

/// Byte cap per session. A busy device writes roughly 1–3 MiB/min across all buffers,
/// so a full-length session normally fits; the app zips the result.
pub const MAX_BYTES: u64 = 64 * 1024 * 1024;
/// Wall-clock cap per session.
pub const MAX_DURATION: Duration = Duration::from_secs(30 * 60);
/// How long Stop (and shutdown) waits for the child to exit and the file to flush.
const STOP_WAIT: Duration = Duration::from_secs(10);
/// Name prefix of the copy core hands to the app.
pub const EXPORT_PREFIX: &str = "logcat-session-";
/// Core's subdir (in its own state dir) for session files.
pub const DIR_NAME: &str = "logcat";

/// Where and how core records. Production: [`RecorderConfig::from_env`].
#[derive(Debug, Clone)]
pub struct RecorderConfig {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Core-owned dir for the growing file (0700).
    pub dir: PathBuf,
    pub max_bytes: u64,
    pub max_duration: Duration,
    /// Append `-T <start − 1 s>` (logcat's `sssss.mmm` form) so the file starts at the
    /// session instead of replaying the whole ring buffer into the cap.
    pub since_start: bool,
}

impl RecorderConfig {
    /// `logcat -b all -v threadtime` into `dir`, with the production caps.
    pub fn production(dir: PathBuf) -> Self {
        RecorderConfig {
            program: PathBuf::from("logcat"),
            args: ["-b", "all", "-v", "threadtime"]
                .map(str::to_owned)
                .to_vec(),
            dir,
            max_bytes: MAX_BYTES,
            max_duration: MAX_DURATION,
            since_start: true,
        }
    }

    /// Enabled with core's root work (`VPNHIDE_ROOT_EFFECTS=1`), in `<core dir>/logcat`.
    pub fn from_env() -> Option<Self> {
        crate::effects::effect_paths_from_env()?;
        Some(Self::production(
            crate::ownership::owner_dir().join(DIR_NAME),
        ))
    }
}

enum Event {
    Stop,
    Cap,
    Eof,
}

struct Ended {
    state: LogcatSessionState,
    duration_ms: u64,
    logcat_exit: Option<i32>,
}

#[derive(Default)]
struct Shared {
    bytes: AtomicU64,
    truncated: AtomicBool,
    ended: Mutex<Option<Ended>>,
    cv: Condvar,
}

impl Shared {
    fn running(&self) -> bool {
        self.ended.lock().expect("logcat ended").is_none()
    }

    /// Wait until the supervisor recorded the end, up to `wait`.
    fn wait_ended(&self, wait: Duration) -> bool {
        let guard = self.ended.lock().expect("logcat ended");
        let (guard, _) = self
            .cv
            .wait_timeout_while(guard, wait, |ended| ended.is_none())
            .expect("logcat ended");
        guard.is_some()
    }
}

struct Session {
    id: LogcatSessionId,
    started: Instant,
    started_at_ms: u64,
    file: PathBuf,
    max_bytes: u64,
    max_duration: Duration,
    shared: Arc<Shared>,
    events: mpsc::Sender<Event>,
    /// The export copy Stop handed to the app.
    delivered: Option<PathBuf>,
}

impl Session {
    fn status(&self) -> LogcatSessionStatus {
        let ended = self.shared.ended.lock().expect("logcat ended");
        let (state, duration_ms, logcat_exit) = match ended.as_ref() {
            Some(e) => (e.state.clone(), e.duration_ms, e.logcat_exit),
            None => (
                LogcatSessionState::Recording,
                millis(self.started.elapsed()),
                None,
            ),
        };
        LogcatSessionStatus {
            session: self.id,
            state,
            started_at_ms: self.started_at_ms,
            duration_ms,
            bytes: self.shared.bytes.load(Ordering::SeqCst),
            truncated: self.shared.truncated.load(Ordering::SeqCst),
            max_bytes: self.max_bytes,
            max_duration_ms: millis(self.max_duration),
            logcat_exit,
            file: None,
        }
    }

    /// Ask the supervisor to stop and wait for the flushed end.
    fn stop(&self) -> bool {
        if self.shared.running() {
            let _ = self.events.send(Event::Stop);
        }
        self.shared.wait_ended(STOP_WAIT)
    }

    /// Remove core's file and the app's copy (a newer session replaces this one).
    fn discard(self) {
        self.stop();
        let _ = fs::remove_file(&self.file);
        if let Some(copy) = &self.delivered {
            let _ = fs::remove_file(copy);
        }
    }
}

/// The single recording slot.
#[derive(Default)]
pub struct Recorder {
    config: Mutex<Option<RecorderConfig>>,
    session: Mutex<Option<Session>>,
}

impl Recorder {
    pub fn new(config: Option<RecorderConfig>) -> Self {
        Recorder {
            config: Mutex::new(config),
            session: Mutex::new(None),
        }
    }

    /// Replace the configuration (tests). A running session keeps its own limits.
    pub fn set_config(&self, config: Option<RecorderConfig>) {
        *self.config.lock().expect("logcat config") = config;
    }

    /// Remove every session file a previous core run left behind (core start only:
    /// no session of this run exists yet).
    pub fn clean_stale(&self) {
        let Some(config) = self.config.lock().expect("logcat config").clone() else {
            return;
        };
        if self.session.lock().expect("logcat session").is_some() {
            return;
        }
        let _ = fs::remove_dir_all(&config.dir);
    }

    pub fn status(&self) -> Option<LogcatSessionStatus> {
        self.session
            .lock()
            .expect("logcat session")
            .as_ref()
            .map(Session::status)
    }

    pub fn start(
        &self,
        id: LogcatSessionId,
        env: &ProbeEnv,
    ) -> Result<LogcatSessionStatus, LogcatRecordingError> {
        if id.0 == 0 {
            return Err(LogcatRecordingError::Invalid("session id 0".into()));
        }
        let Some(config) = self.config.lock().expect("logcat config").clone() else {
            return Err(LogcatRecordingError::Disabled);
        };
        let mut slot = self.session.lock().expect("logcat session");
        if let Some(current) = slot.as_ref() {
            if current.id == id {
                return Ok(current.status());
            }
            if current.shared.running() {
                return Err(LogcatRecordingError::Busy {
                    running: current.id,
                });
            }
        }
        if let Some(old) = slot.take() {
            old.discard();
        }
        remove_stale_exports(env);
        let session = spawn(&config, id).map_err(LogcatRecordingError::Failed)?;
        crate::alog::info(&format!(
            "logcat: session {} started (cap {} bytes, {} s)",
            id.0,
            config.max_bytes,
            config.max_duration.as_secs()
        ));
        let status = session.status();
        *slot = Some(session);
        Ok(status)
    }

    pub fn stop(
        &self,
        id: LogcatSessionId,
        env: &ProbeEnv,
    ) -> Result<LogcatSessionStatus, LogcatRecordingError> {
        let mut slot = self.session.lock().expect("logcat session");
        let Some(session) = slot.as_mut().filter(|s| s.id == id) else {
            return Err(LogcatRecordingError::UnknownSession);
        };
        if !session.stop() {
            return Err(LogcatRecordingError::Failed("logcat did not stop".into()));
        }
        let copy = match &session.delivered {
            Some(copy) if copy.exists() => copy.clone(),
            _ => deliver(env, &session.file, id).map_err(LogcatRecordingError::Failed)?,
        };
        session.delivered = Some(copy.clone());
        let mut status = session.status();
        status.file = Some(copy.display().to_string());
        Ok(status)
    }

    /// Core is exiting: stop a running child and wait for it.
    pub fn shutdown(&self) {
        if let Some(session) = self.session.lock().expect("logcat session").as_ref() {
            session.stop();
        }
    }
}

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// logcat's epoch time form, one second before `now_ms` (stamp rounding must not drop the
/// app's start marker).
fn since_arg(now_ms: u64) -> String {
    let at = now_ms.saturating_sub(1000);
    format!("{}.{:03}", at / 1000, at % 1000)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(millis)
        .unwrap_or(0)
}

/// Create the file, start the supervisor, and return once the child is running.
fn spawn(config: &RecorderConfig, id: LogcatSessionId) -> Result<Session, String> {
    fs::create_dir_all(&config.dir).map_err(|e| format!("logcat dir: {e}"))?;
    let _ = fs::set_permissions(&config.dir, fs::Permissions::from_mode(0o700));
    let path = config.dir.join(format!("session-{}.log", id.0));
    let _ = fs::remove_file(&path);
    let out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .map_err(|e| format!("logcat file: {e}"))?;
    let shared = Arc::new(Shared::default());
    let (events, inbox) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::channel();
    let started = Instant::now();
    let started_at_ms = now_ms();
    {
        let config = config.clone();
        let shared = shared.clone();
        let events = events.clone();
        std::thread::Builder::new()
            .name("logcat-session".into())
            .spawn(move || supervise(config, out, shared, events, inbox, ready_tx, started))
            .map_err(|e| format!("logcat thread: {e}"))?;
    }
    match ready_rx.recv() {
        Ok(Ok(())) => Ok(Session {
            id,
            started,
            started_at_ms,
            file: path,
            max_bytes: config.max_bytes,
            max_duration: config.max_duration,
            shared,
            events,
            delivered: None,
        }),
        Ok(Err(e)) => {
            let _ = fs::remove_file(&path);
            Err(e)
        }
        Err(_) => {
            let _ = fs::remove_file(&path);
            Err("logcat supervisor ended".into())
        }
    }
}

/// Owns the child from spawn to reap: the parent-death signal is tied to this thread.
fn supervise(
    config: RecorderConfig,
    out: File,
    shared: Arc<Shared>,
    events: mpsc::Sender<Event>,
    inbox: mpsc::Receiver<Event>,
    ready: mpsc::Sender<Result<(), String>>,
    started: Instant,
) {
    let mut cmd = Command::new(&config.program);
    cmd.args(&config.args);
    if config.since_start {
        cmd.arg("-T").arg(since_arg(now_ms()));
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // SAFETY: prctl is async-signal-safe and touches only the child being exec'd.
    unsafe {
        cmd.pre_exec(|| {
            libc::prctl(
                libc::PR_SET_PDEATHSIG,
                libc::SIGKILL as libc::c_ulong,
                0,
                0,
                0,
            );
            Ok(())
        });
    }
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            let _ = ready.send(Err(format!("spawn {}: {e}", config.program.display())));
            return;
        }
    };
    let stdout = child.stdout.take().expect("piped");
    let reader = {
        let shared = shared.clone();
        let max = config.max_bytes;
        std::thread::spawn(move || copy_capped(stdout, out, &shared, max, &events))
    };
    let _ = ready.send(Ok(()));

    let deadline = started + config.max_duration;
    let state = loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break LogcatSessionState::AutoStopped {
                reason: LogcatAutoStopReason::DurationCap,
            };
        }
        match inbox.recv_timeout(left) {
            Ok(Event::Stop) | Err(RecvTimeoutError::Disconnected) => {
                break LogcatSessionState::Stopped;
            }
            Ok(Event::Cap) => {
                break LogcatSessionState::AutoStopped {
                    reason: LogcatAutoStopReason::SizeCap,
                };
            }
            Ok(Event::Eof) => {
                break LogcatSessionState::AutoStopped {
                    reason: LogcatAutoStopReason::LogcatExited,
                };
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
    };
    let exited_alone = matches!(
        state,
        LogcatSessionState::AutoStopped {
            reason: LogcatAutoStopReason::LogcatExited
        }
    );
    if !exited_alone {
        let _ = child.kill();
    }
    let exit = child.wait().ok().and_then(|s| s.code());
    // The pipe hits EOF once the child is gone; the reader then flushes and syncs.
    let _ = reader.join();
    let duration_ms = millis(started.elapsed());
    crate::alog::info(&format!(
        "logcat: session ended {state:?} after {duration_ms} ms, {} bytes",
        shared.bytes.load(Ordering::SeqCst)
    ));
    *shared.ended.lock().expect("logcat ended") = Some(Ended {
        state,
        duration_ms,
        logcat_exit: if exited_alone { exit } else { None },
    });
    shared.cv.notify_all();
}

/// Pipe → file up to `max` bytes. Past the cap the rest is dropped and the supervisor told.
fn copy_capped(
    mut from: impl Read,
    mut to: File,
    shared: &Shared,
    max: u64,
    events: &mpsc::Sender<Event>,
) {
    let mut chunk = vec![0u8; 64 * 1024];
    let mut written: u64 = 0;
    loop {
        let n = match from.read(&mut chunk) {
            Ok(0) | Err(_) => {
                let _ = events.send(Event::Eof);
                break;
            }
            Ok(n) => n as u64,
        };
        let room = max.saturating_sub(written);
        let take = n.min(room);
        if take > 0 && to.write_all(&chunk[..take as usize]).is_err() {
            let _ = events.send(Event::Eof);
            break;
        }
        written += take;
        shared.bytes.store(written, Ordering::SeqCst);
        if n > room {
            shared.truncated.store(true, Ordering::SeqCst);
            let _ = events.send(Event::Cap);
            break;
        }
    }
    let _ = to.sync_all();
}

/// Copy core's file into the app's export dir as an app-owned 0600 file.
fn deliver(env: &ProbeEnv, file: &Path, id: LogcatSessionId) -> Result<PathBuf, String> {
    let export = env
        .export_dir()
        .ok_or_else(|| "no app data dir".to_owned())?;
    prepare_dir(env, &export).map_err(|e| format!("export dir: {e}"))?;
    // The export dir is the app's: never follow a link it planted there.
    let meta = fs::symlink_metadata(&export).map_err(|e| format!("export dir: {e}"))?;
    if !meta.is_dir() {
        return Err("export dir is not a directory".into());
    }
    let path = export.join(format!("{EXPORT_PREFIX}{}.txt", id.0));
    let _ = fs::remove_file(&path);
    let mut to = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .map_err(|e| format!("export file: {e}"))?;
    let mut from = File::open(file).map_err(|e| format!("session file: {e}"))?;
    std::io::copy(&mut from, &mut to).map_err(|e| format!("export copy: {e}"))?;
    hand_to_app(env, &path, 0o600);
    Ok(path)
}

/// Copies an earlier session left in the export dir (the app normally removes its own).
fn remove_stale_exports(env: &ProbeEnv) {
    let Some(export) = env.export_dir() else {
        return;
    };
    let Ok(entries) = fs::read_dir(&export) else {
        return;
    };
    for entry in entries.flatten() {
        if entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.starts_with(EXPORT_PREFIX))
        {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn since_is_one_second_before_start_in_logcats_epoch_form() {
        assert_eq!(since_arg(1_760_000_000_123), "1759999999.123");
        assert_eq!(since_arg(500), "0.000");
    }

    #[test]
    fn production_runs_logcat_directly_with_fixed_arguments() {
        let config = RecorderConfig::production(PathBuf::from("/data/adb/vpnhide/core/logcat"));
        assert_eq!(config.program, PathBuf::from("logcat"));
        assert_eq!(config.args, ["-b", "all", "-v", "threadtime"]);
        assert!(config.since_start);
        assert_eq!(config.max_bytes, 64 << 20);
        assert_eq!(config.max_duration, Duration::from_secs(1800));
    }
}
