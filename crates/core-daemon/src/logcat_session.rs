//! Core-owned full-system logcat recording (protocol v8; debug lease and delivery v9).
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
//!   shutdown stops it explicitly. At start, [`Recorder::recover`] ends a session an
//!   earlier core run left recording (`CoreRestarted`) and removes everything else.
//! - **Delivery.** Stop copies the file into the app's export dir (app-owned, 0600), the
//!   same route large probe output takes, never inline over the 1 MiB frame. The session
//!   stays undelivered until the app acknowledges it saved the recording (`ack`), or
//!   discards it; until then core keeps the file, also across a core restart.
//! - **Debug logging (v9).** Start takes a lease on the config's `debug` field: core writes
//!   `debug = true` through its own journaled edit path and keeps it true on every edit it
//!   admits while the session records. When the session ends (Stop, any auto-stop, or at
//!   the next core start after a crash or reboot) core puts it back:
//!   - if the user's debug switch (`debugSwitch`) still has its value from the start of the
//!     session, `debug` returns to the value it had then (`Restored`);
//!   - if the user changed the switch during the session, `debug` follows the switch
//!     (`UserChoice`): an explicit change always wins over the restore.
//!
//!   A restore that cannot be written yet (an uncertain operation blocks writes) stays
//!   `Pending` in the session record and is retried on the next session call and at start.
//! - **Persistence.** `<dir>/session.json` holds the one session's record (limits, end
//!   state, lease, delivery); only the latest session is kept.

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

use serde::{Deserialize, Serialize};
use vpnhide_core_rpc::{
    LogcatAutoStopReason, LogcatDebugLease, LogcatDebugRestore, LogcatRecordingError,
    LogcatSessionId, LogcatSessionState, LogcatSessionStatus,
};

use crate::diagnostics::ProbeEnv;

/// Byte cap per session. A busy device writes roughly 1–3 MiB/min across all buffers,
/// so a full-length session normally fits; the app zips the result.
pub const MAX_BYTES: u64 = 64 * 1024 * 1024;
/// Wall-clock cap per session.
pub const MAX_DURATION: Duration = Duration::from_secs(30 * 60);
/// How long Stop (and shutdown) waits for the child to exit and the file to flush.
const STOP_WAIT: Duration = Duration::from_secs(10);
/// How long shutdown and discard wait for the debug restore after the child ended.
const SETTLE_WAIT: Duration = Duration::from_secs(30);
/// The session record in the session dir.
const RECORD_NAME: &str = "session.json";
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

/// Core's side of the debug-logging lease (implemented by `CoreState`).
pub trait DebugControl: Send + Sync {
    /// Turn `debug` on and hold it while recording. `None` when core cannot write now.
    /// `persist` gets the lease (`restore: Pending`) after the previous values are read
    /// and before anything is written, so a crash in between still knows what to restore.
    fn acquire(
        &self,
        persist: &dyn Fn(&LogcatDebugLease) -> std::io::Result<()>,
    ) -> Option<LogcatDebugLease>;
    /// Stop holding `debug` and put it back by the restore rule (see the module docs).
    fn release(&self, lease: &LogcatDebugLease) -> LogcatDebugRestore;
}

enum Event {
    Stop,
    Cap,
    Eof,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Ended {
    state: LogcatSessionState,
    duration_ms: u64,
    bytes: u64,
    truncated: bool,
    #[serde(default)]
    logcat_exit: Option<i32>,
}

/// What core persists for the one session it keeps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SessionRecord {
    session: u64,
    started_at_ms: u64,
    max_bytes: u64,
    max_duration_ms: u64,
    #[serde(default)]
    ended: Option<Ended>,
    #[serde(default)]
    debug: Option<LogcatDebugLease>,
    #[serde(default)]
    delivered: bool,
}

struct Shared {
    bytes: AtomicU64,
    truncated: AtomicBool,
    record: Mutex<SessionRecord>,
    record_path: PathBuf,
    /// Discarded: never write the record again (a late supervisor must not recreate it).
    discarded: AtomicBool,
    /// The supervisor finished (child reaped, debug released).
    settled: AtomicBool,
    cv: Condvar,
}

impl Shared {
    fn new(record: SessionRecord, record_path: PathBuf, settled: bool) -> Self {
        let bytes = record.ended.as_ref().map_or(0, |e| e.bytes);
        let truncated = record.ended.as_ref().is_some_and(|e| e.truncated);
        Shared {
            bytes: AtomicU64::new(bytes),
            truncated: AtomicBool::new(truncated),
            record: Mutex::new(record),
            record_path,
            discarded: AtomicBool::new(false),
            settled: AtomicBool::new(settled),
            cv: Condvar::new(),
        }
    }

    fn record(&self) -> std::sync::MutexGuard<'_, SessionRecord> {
        self.record.lock().expect("logcat record")
    }

    fn running(&self) -> bool {
        self.record().ended.is_none()
    }

    /// Change the record and write it through (unless discarded).
    fn update(&self, change: impl FnOnce(&mut SessionRecord)) {
        let mut record = self.record();
        change(&mut record);
        if !self.discarded.load(Ordering::SeqCst) {
            if let Err(error) = write_record(&self.record_path, &record) {
                crate::alog::info(&format!("logcat: session record write failed: {error}"));
            }
        }
        drop(record);
        self.cv.notify_all();
    }

    fn try_update(&self, change: impl FnOnce(&mut SessionRecord)) -> std::io::Result<()> {
        let mut record = self.record();
        let mut candidate = record.clone();
        change(&mut candidate);
        if !self.discarded.load(Ordering::SeqCst) {
            write_record(&self.record_path, &candidate)?;
        }
        *record = candidate;
        drop(record);
        self.cv.notify_all();
        Ok(())
    }

    fn wait(&self, wait: Duration, done: impl Fn(&SessionRecord, &Shared) -> bool) -> bool {
        let guard = self.record();
        let (guard, _) = self
            .cv
            .wait_timeout_while(guard, wait, |record| !done(record, self))
            .expect("logcat record");
        done(&guard, self)
    }

    fn wait_ended(&self, wait: Duration) -> bool {
        self.wait(wait, |record, _| record.ended.is_some())
    }

    fn wait_settled(&self, wait: Duration) -> bool {
        self.wait(wait, |_, shared| shared.settled.load(Ordering::SeqCst))
    }

    fn mark_settled(&self) {
        let _guard = self.record();
        self.settled.store(true, Ordering::SeqCst);
        self.cv.notify_all();
    }
}

fn write_record(path: &Path, record: &SessionRecord) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(record).map_err(std::io::Error::other)? + "\n";
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    File::open(path.parent().unwrap_or_else(|| Path::new(".")))?.sync_all()
}

fn read_record(path: &Path) -> Option<SessionRecord> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

struct Session {
    id: LogcatSessionId,
    /// `None` for a session recovered from disk (no child of this core run).
    started: Option<Instant>,
    file: PathBuf,
    shared: Arc<Shared>,
    events: Option<mpsc::Sender<Event>>,
    /// The export copy Stop handed to the app.
    delivered_copy: Option<PathBuf>,
}

impl Session {
    fn status(&self) -> LogcatSessionStatus {
        let record = self.shared.record();
        let (state, duration_ms, bytes, truncated, logcat_exit) = match record.ended.as_ref() {
            Some(e) => (
                e.state.clone(),
                e.duration_ms,
                e.bytes,
                e.truncated,
                e.logcat_exit,
            ),
            None => (
                LogcatSessionState::Recording,
                self.started.map_or(0, |s| millis(s.elapsed())),
                self.shared.bytes.load(Ordering::SeqCst),
                self.shared.truncated.load(Ordering::SeqCst),
                None,
            ),
        };
        LogcatSessionStatus {
            session: self.id,
            state,
            started_at_ms: record.started_at_ms,
            duration_ms,
            bytes,
            truncated,
            max_bytes: record.max_bytes,
            max_duration_ms: record.max_duration_ms,
            logcat_exit,
            file: None,
            delivered: record.delivered,
            debug: record.debug,
        }
    }

    /// Ask the supervisor to stop and wait for the flushed end.
    fn stop(&self) -> bool {
        if self.shared.running() {
            if let Some(events) = &self.events {
                let _ = events.send(Event::Stop);
            }
        }
        self.shared.wait_ended(STOP_WAIT)
    }

    /// Delete everything this session left (core file, export copies, record).
    fn remove_files(&self, env: &ProbeEnv) {
        self.shared.discarded.store(true, Ordering::SeqCst);
        let _ = fs::remove_file(&self.file);
        let _ = fs::remove_file(&self.shared.record_path);
        if let Some(copy) = &self.delivered_copy {
            let _ = fs::remove_file(copy);
        }
        remove_stale_exports(env);
    }
}

/// The single recording slot.
#[derive(Default)]
pub struct Recorder {
    config: Mutex<Option<RecorderConfig>>,
    debug: Mutex<Option<Arc<dyn DebugControl>>>,
    session: Mutex<Option<Session>>,
}

impl Recorder {
    /// Admission checks use the physical session, independently of app-local UI tokens.
    pub fn is_active(&self) -> bool {
        // Recorder callbacks may await the writer queue while holding this slot.
        // Reset admission on that writer must refuse a concurrent transition,
        // rather than wait on the slot and deadlock the callback behind itself.
        match self.session.try_lock() {
            Ok(slot) => slot
                .as_ref()
                .is_some_and(|session| session.shared.running()),
            Err(_) => true,
        }
    }
    pub fn new(config: Option<RecorderConfig>) -> Self {
        Recorder {
            config: Mutex::new(config),
            debug: Mutex::new(None),
            session: Mutex::new(None),
        }
    }

    /// Replace the configuration (tests). A running session keeps its own limits.
    pub fn set_config(&self, config: Option<RecorderConfig>) {
        *self.config.lock().expect("logcat config") = config;
    }

    /// Who takes and releases the debug-logging lease (core installs itself).
    pub fn set_debug_control(&self, control: Option<Arc<dyn DebugControl>>) {
        *self.debug.lock().expect("logcat debug") = control;
    }

    fn debug_control(&self) -> Option<Arc<dyn DebugControl>> {
        self.debug.lock().expect("logcat debug").clone()
    }

    /// Core start: pick up what an earlier core run left. An unfinished session (core
    /// crashed, or the device rebooted) ends as `CoreRestarted` and has its debug lease
    /// released; a finished, undelivered recording is kept so the app can still save it;
    /// anything else in the dir is removed.
    pub fn recover(&self) {
        let Some(config) = self.config.lock().expect("logcat config").clone() else {
            return;
        };
        let mut slot = self.session.lock().expect("logcat session");
        if slot.is_some() {
            return;
        }
        let record_path = config.dir.join(RECORD_NAME);
        let Some(mut record) = read_record(&record_path) else {
            let _ = fs::remove_dir_all(&config.dir);
            return;
        };
        let file = config.dir.join(log_name(record.session));
        if record.ended.is_none() {
            let meta = fs::metadata(&file).ok();
            let bytes = meta.as_ref().map_or(0, |m| m.len());
            let last_write_ms = meta
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(record.started_at_ms, millis);
            record.ended = Some(Ended {
                state: LogcatSessionState::AutoStopped {
                    reason: LogcatAutoStopReason::CoreRestarted,
                },
                duration_ms: last_write_ms
                    .saturating_sub(record.started_at_ms)
                    .min(record.max_duration_ms),
                bytes,
                truncated: bytes >= record.max_bytes,
                logcat_exit: None,
            });
            crate::alog::info(&format!(
                "logcat: session {} was still recording when core stopped; ended it",
                record.session
            ));
        }
        if let Some(lease) = record.debug.as_mut() {
            if matches!(
                lease.restore,
                LogcatDebugRestore::Held | LogcatDebugRestore::Pending
            ) {
                lease.restore = match self.debug_control() {
                    Some(control) => control.release(lease),
                    None => LogcatDebugRestore::Pending,
                };
            }
        }
        // Keep only this session's two files.
        if let Ok(entries) = fs::read_dir(&config.dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path != record_path && path != file {
                    let _ = fs::remove_file(path);
                }
            }
        }
        let pending = record
            .debug
            .is_some_and(|l| l.restore == LogcatDebugRestore::Pending);
        if record.delivered || (!file.exists() && !pending) {
            let _ = fs::remove_dir_all(&config.dir);
            return;
        }
        let shared = Arc::new(Shared::new(record.clone(), record_path, true));
        if let Err(e) = write_record(&shared.record_path, &record) {
            crate::alog::info(&format!("logcat: session record write failed: {e}"));
        }
        *slot = Some(Session {
            id: LogcatSessionId(record.session),
            started: None,
            file,
            shared,
            events: None,
            delivered_copy: None,
        });
    }

    /// A restore that could not be written earlier: try again (every session call).
    fn retry_pending(&self, session: &Session) {
        if !session.shared.settled.load(Ordering::SeqCst) {
            return;
        }
        let lease = session.shared.record().debug;
        let Some(lease) = lease.filter(|l| l.restore == LogcatDebugRestore::Pending) else {
            return;
        };
        let Some(control) = self.debug_control() else {
            return;
        };
        let restore = control.release(&lease);
        session.shared.update(|record| {
            if let Some(l) = record.debug.as_mut() {
                l.restore = restore;
            }
        });
    }

    pub fn status(&self) -> Option<LogcatSessionStatus> {
        let slot = self.session.lock().expect("logcat session");
        let session = slot.as_ref()?;
        self.retry_pending(session);
        Some(session.status())
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
            self.retry_pending(current);
            if current.id == id {
                return Ok(current.status());
            }
            if current.shared.running() {
                return Err(LogcatRecordingError::Busy {
                    running: current.id,
                });
            }
        }
        // A new Start replaces the previous session, delivered or not.
        if let Some(old) = slot.take() {
            old.shared.wait_settled(SETTLE_WAIT);
            old.remove_files(env);
        }
        remove_stale_exports(env);
        let debug = self.debug_control();
        let session = spawn(&config, id, debug).map_err(LogcatRecordingError::Failed)?;
        crate::alog::info(&format!(
            "logcat: session {} started (cap {} bytes, {} s, debug lease {:?})",
            id.0,
            config.max_bytes,
            config.max_duration.as_secs(),
            session.shared.record().debug
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
        self.retry_pending(session);
        let mut status = session.status();
        if status.delivered {
            // The app already saved it; core's copies are gone.
            return Ok(status);
        }
        let copy = match &session.delivered_copy {
            Some(copy) if copy.exists() => copy.clone(),
            _ => deliver(env, &session.file, id).map_err(LogcatRecordingError::Failed)?,
        };
        session.delivered_copy = Some(copy.clone());
        status.file = Some(copy.display().to_string());
        Ok(status)
    }

    /// The app saved the recording: mark it delivered and delete core's copies.
    pub fn ack(
        &self,
        id: LogcatSessionId,
        env: &ProbeEnv,
    ) -> Result<LogcatSessionStatus, LogcatRecordingError> {
        let slot = self.session.lock().expect("logcat session");
        let Some(session) = slot.as_ref().filter(|s| s.id == id) else {
            return Err(LogcatRecordingError::UnknownSession);
        };
        if session.shared.running() {
            return Err(LogcatRecordingError::Invalid(
                "the session is still recording".into(),
            ));
        }
        self.retry_pending(session);
        if !session.shared.record().delivered {
            session.shared.update(|record| record.delivered = true);
            let _ = fs::remove_file(&session.file);
            if let Some(copy) = &session.delivered_copy {
                let _ = fs::remove_file(copy);
            }
            remove_stale_exports(env);
            crate::alog::info(&format!("logcat: session {} delivered", id.0));
        }
        Ok(session.status())
    }

    /// Drop the session: stop it if it records, restore debug logging, delete its files.
    pub fn discard(&self, id: LogcatSessionId, env: &ProbeEnv) -> Result<(), LogcatRecordingError> {
        let mut slot = self.session.lock().expect("logcat session");
        if !slot.as_ref().is_some_and(|s| s.id == id) {
            return Ok(());
        }
        let session = slot.take().expect("checked");
        if !session.stop() {
            *slot = Some(session);
            return Err(LogcatRecordingError::Failed("logcat did not stop".into()));
        }
        session.shared.wait_settled(SETTLE_WAIT);
        self.retry_pending(&session);
        session.remove_files(env);
        crate::alog::info(&format!("logcat: session {} discarded", id.0));
        Ok(())
    }

    /// Core is exiting: stop a running child and wait until its debug lease is released.
    pub fn shutdown(&self) {
        if let Some(session) = self.session.lock().expect("logcat session").as_ref() {
            session.stop();
            session.shared.wait_settled(SETTLE_WAIT);
        }
    }
}

fn log_name(id: u64) -> String {
    format!("session-{id}.log")
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

/// Create the file and record, take the debug lease, start the supervisor, and return once
/// the child is running.
fn spawn(
    config: &RecorderConfig,
    id: LogcatSessionId,
    debug: Option<Arc<dyn DebugControl>>,
) -> Result<Session, String> {
    fs::create_dir_all(&config.dir).map_err(|e| format!("logcat dir: {e}"))?;
    let _ = fs::set_permissions(&config.dir, fs::Permissions::from_mode(0o700));
    let path = config.dir.join(log_name(id.0));
    let _ = fs::remove_file(&path);
    let out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .map_err(|e| format!("logcat file: {e}"))?;
    let record = SessionRecord {
        session: id.0,
        started_at_ms: now_ms(),
        max_bytes: config.max_bytes,
        max_duration_ms: millis(config.max_duration),
        ended: None,
        debug: None,
        delivered: false,
    };
    let shared = Arc::new(Shared::new(record, config.dir.join(RECORD_NAME), false));
    // Persist first: a crash after the lease is taken must still find a record to restore from.
    shared
        .try_update(|_| {})
        .map_err(|error| format!("logcat record: {error}"))?;
    let failed = |e: String| {
        shared.discarded.store(true, Ordering::SeqCst);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&shared.record_path);
        Err(e)
    };
    // Lease before the child starts, so the first lines are already verbose.
    let lease = debug.as_ref().and_then(|control| {
        let persist =
            |lease: &LogcatDebugLease| shared.try_update(|record| record.debug = Some(*lease));
        let lease = control.acquire(&persist);
        shared.update(|record| record.debug = lease);
        lease
    });
    let (events, inbox) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::channel();
    let started = Instant::now();
    {
        let config = config.clone();
        let shared = shared.clone();
        let events = events.clone();
        let control = debug.clone();
        let spawned = std::thread::Builder::new()
            .name("logcat-session".into())
            .spawn(move || {
                supervise(config, out, &shared, events, inbox, ready_tx, started);
                release_lease(&shared, control.as_deref());
                shared.mark_settled();
            });
        if let Err(e) = spawned {
            if let (Some(lease), Some(control)) = (lease, debug.as_ref()) {
                control.release(&lease);
            }
            return failed(format!("logcat thread: {e}"));
        }
    }
    match ready_rx.recv() {
        Ok(Ok(())) => Ok(Session {
            id,
            started: Some(started),
            file: path,
            shared,
            events: Some(events),
            delivered_copy: None,
        }),
        // The supervisor already released the lease on its way out.
        Ok(Err(e)) => {
            shared.wait_settled(SETTLE_WAIT);
            failed(e)
        }
        Err(_) => {
            shared.wait_settled(SETTLE_WAIT);
            failed("logcat supervisor ended".into())
        }
    }
}

/// The session ended: put debug logging back (the record says how).
fn release_lease(shared: &Shared, control: Option<&dyn DebugControl>) {
    let lease = shared.record().debug;
    let Some(lease) = lease.filter(|l| {
        matches!(
            l.restore,
            LogcatDebugRestore::Held | LogcatDebugRestore::Pending
        )
    }) else {
        return;
    };
    // `Held` on disk already means "still to restore" to the next core run.
    let restore = control.map_or(LogcatDebugRestore::Pending, |c| c.release(&lease));
    crate::alog::info(&format!("logcat: debug lease released: {restore:?}"));
    shared.update(|record| {
        if let Some(l) = record.debug.as_mut() {
            l.restore = restore;
        }
    });
}

/// Owns the child from spawn to reap: the parent-death signal is tied to this thread.
fn supervise(
    config: RecorderConfig,
    out: File,
    shared: &Arc<Shared>,
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
        .stderr(Stdio::null())
        .process_group(0);
    let parent = unsafe { libc::getpid() };
    // SAFETY: only async-signal-safe syscalls before exec. The parent check
    // closes the fork/prctl window where core could already have died.
    unsafe {
        cmd.pre_exec(move || {
            if libc::prctl(
                libc::PR_SET_PDEATHSIG,
                libc::SIGKILL as libc::c_ulong,
                0,
                0,
                0,
            ) != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            if libc::getppid() != parent {
                libc::_exit(1);
            }
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
    // The leader is still unreaped, reserving its PID/group identity. Kill any
    // descendants too so inherited pipe owners cannot strand the reader join.
    unsafe { libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL) };
    let exit = child.wait().ok().and_then(|s| s.code());
    // The pipe hits EOF once the child is gone; the reader then flushes and syncs.
    let _ = reader.join();
    let duration_ms = millis(started.elapsed());
    crate::alog::info(&format!(
        "logcat: session ended {state:?} after {duration_ms} ms, {} bytes",
        shared.bytes.load(Ordering::SeqCst)
    ));
    let bytes = shared.bytes.load(Ordering::SeqCst);
    let truncated = shared.truncated.load(Ordering::SeqCst);
    shared.update(|record| {
        record.ended = Some(Ended {
            state,
            duration_ms,
            bytes,
            truncated,
            logcat_exit: if exited_alone { exit } else { None },
        });
    });
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
    let export = crate::export::ExportDir::open(env).map_err(|e| format!("export dir: {e}"))?;
    let filename = format!("{EXPORT_PREFIX}{}.txt", id.0);
    export.remove(&filename);
    let mut from = File::open(file).map_err(|e| format!("session file: {e}"))?;
    export
        .write(&filename, |to| std::io::copy(&mut from, to).map(|_| ()))
        .map_err(|e| format!("export copy: {e}"))
}

/// Remove names only relative to the pinned export directory.
fn remove_stale_exports(env: &ProbeEnv) {
    let Ok(export) = crate::export::ExportDir::open(env) else {
        return;
    };
    for filename in export.names().unwrap_or_default() {
        if filename.starts_with(EXPORT_PREFIX) {
            export.remove(&filename);
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
