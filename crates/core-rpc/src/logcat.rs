//! Core-owned full-system logcat recording (protocol v8).
//!
//! The app asks core to record `logcat -b all`, and core alone owns the root child, its
//! output file and its limits: one session at a time, a byte cap and a wall-clock cap
//! after which core stops the child itself. A killed app therefore can never leave a
//! root logcat running. Both calls are idempotent and keyed by the app's session id.

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
