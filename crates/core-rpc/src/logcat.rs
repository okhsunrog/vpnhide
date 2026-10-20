//! Core-owned full-system logcat recording (protocol v8; debug lease and delivery v9).
//!
//! The app asks core to record `logcat -b all`, and core alone owns the root child, its
//! output file and its limits: one session at a time, a byte cap and a wall-clock cap
//! after which core stops the child itself. A killed app therefore can never leave a
//! root logcat running. Every call is idempotent and keyed by the app's session id.
//!
//! The session also owns the temporary debug logging a capture needs (v9): core turns
//! `debug` on through its own journaled config write, holds it on while recording, and puts
//! it back when the session ends, also after a crash or reboot. The finished recording
//! stays with core until the app acknowledges it has saved it (or discards it), so an
//! app killed between Stop and saving can still offer it later.

use serde::{Deserialize, Serialize};

/// The app's id for one recording (any non-zero value it chooses; the retry key).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LogcatSessionId(pub u64);

/// Why core ended a session that nobody stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogcatAutoStopReason {
    /// The file reached the session's byte cap; later output was dropped.
    SizeCap,
    /// The session ran for its maximum duration.
    DurationCap,
    /// `logcat` exited by itself (see [`LogcatSessionStatus::logcat_exit`]).
    LogcatExited,
    /// Core found the session unfinished when it started (it crashed, or the device
    /// rebooted). The file ends where the old core stopped.
    CoreRestarted,
}

/// What the session's debug-logging lease did with the config's `debug` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogcatDebugRestore {
    /// Recording: core holds `debug` on.
    Held,
    /// Ended, but the restore write could not run yet (retried by core).
    Pending,
    /// Ended: `debug` is back to its value before the session.
    Restored,
    /// Ended: the user changed the debug switch during the session; that choice was kept.
    UserChoice,
}

/// The session's hold on debug logging (`debug` in the canonical config).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogcatDebugLease {
    /// `debug` and `debugSwitch` when the session started.
    pub previous_debug: bool,
    pub previous_debug_switch: bool,
    pub restore: LogcatDebugRestore,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LogcatSessionState {
    /// The child is running.
    Recording,
    /// Stopped by the app's Stop.
    Stopped,
    /// Ended by core on its own limits before any Stop.
    AutoStopped { reason: LogcatAutoStopReason },
}

/// One session as core sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogcatSessionStatus {
    pub session: LogcatSessionId,
    pub state: LogcatSessionState,
    /// Wall clock (ms since the epoch) when the child started.
    pub started_at_ms: u64,
    /// Elapsed while recording; fixed once the session ended.
    pub duration_ms: u64,
    /// Bytes in core's file so far.
    pub bytes: u64,
    /// Output was dropped at the byte cap.
    #[serde(default)]
    pub truncated: bool,
    /// The session's limits, so the app can show them.
    pub max_bytes: u64,
    pub max_duration_ms: u64,
    /// `logcat`'s exit status when it exited by itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logcat_exit: Option<i32>,
    /// Set by Stop: the recording, copied into the app's export dir (app-owned, 0600).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// The app acknowledged it saved the recording; core has deleted its copies.
    #[serde(default)]
    pub delivered: bool,
    /// Core's debug-logging lease. `None` when core could not take one (no config yet, or
    /// an uncertain operation blocks writes); the recording runs without forced logging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug: Option<LogcatDebugLease>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogcatRecordingError {
    /// Another session is recording; stop it first.
    Busy { running: LogcatSessionId },
    /// Stop for an id core does not hold (never started, or replaced by a newer session).
    UnknownSession,
    /// Recording is not enabled in this core (dev sandbox).
    Disabled,
    /// The id is not usable (zero).
    Invalid(String),
    /// Core could not start the child or deliver the file.
    Failed(String),
}
