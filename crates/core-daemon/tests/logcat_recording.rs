//! Core-owned logcat sessions: idempotency, Busy, the byte and duration caps, a child that
//! exits by itself, delivery into the app's export dir, and stale-file cleanup. A real
//! child runs (`sleep` / `yes` / `true` stand in for `logcat`).

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use vpnhide_core_daemon::diagnostics::{EXPORT_DIR_NAME, ProbeEnv};
use vpnhide_core_daemon::logcat_session::{EXPORT_PREFIX, RecorderConfig};
use vpnhide_core_daemon::{CoreState, Publisher};
use vpnhide_core_rpc::{
    LogcatAutoStopReason, LogcatRecordingError, LogcatSessionId, LogcatSessionState, StateSnapshot,
};

struct Fixture {
    state: Arc<CoreState>,
    core_dir: PathBuf,
    app_dir: PathBuf,
    _tmp: tempfile::TempDir,
}

fn fixture(program: &str, args: &[&str], max_bytes: u64, max_duration: Duration) -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let core_dir = tmp.path().join("core/logcat");
    let app_dir = tmp.path().join("app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let state = CoreState::new(Publisher::new(1, StateSnapshot::default()));
    state.set_probe_env(ProbeEnv {
        core_exe: PathBuf::from("/nonexistent"),
        app_dir: Some(app_dir.clone()),
    });
    state.logcat().set_config(Some(RecorderConfig {
        program: PathBuf::from(program),
        args: args.iter().map(|a| a.to_string()).collect(),
        dir: core_dir.clone(),
        max_bytes,
        max_duration,
        since_start: false,
    }));
    Fixture {
        state,
        core_dir,
        app_dir,
        _tmp: tmp,
    }
}

fn idle() -> Fixture {
    fixture("sleep", &["30"], 1 << 20, Duration::from_secs(60))
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn session_files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|it| {
            it.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[test]
fn start_is_idempotent_and_a_second_id_is_busy() {
    let f = idle();
    let a = LogcatSessionId(11);
    let first = f.state.start_logcat_recording(a).unwrap();
    assert_eq!(first.state, LogcatSessionState::Recording);
    let again = f.state.start_logcat_recording(a).unwrap();
    assert_eq!(again.state, LogcatSessionState::Recording);
    assert_eq!(again.started_at_ms, first.started_at_ms);
    assert_eq!(
        f.state.start_logcat_recording(LogcatSessionId(12)),
        Err(LogcatRecordingError::Busy { running: a })
    );
    assert_eq!(
        f.state.start_logcat_recording(LogcatSessionId(0)),
        Err(LogcatRecordingError::Invalid("session id 0".into()))
    );
    assert_eq!(session_files(&f.core_dir), vec!["session-11.log"]);
    let mode = std::fs::metadata(f.core_dir.join("session-11.log"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    f.state.logcat().shutdown();
}

#[test]
fn stop_delivers_once_and_repeats_the_same_result() {
    let f = idle();
    let id = LogcatSessionId(21);
    f.state.start_logcat_recording(id).unwrap();
    let stopped = f.state.stop_logcat_recording(id).unwrap();
    assert_eq!(stopped.state, LogcatSessionState::Stopped);
    assert!(!stopped.truncated);
    let file = PathBuf::from(stopped.file.clone().expect("delivered"));
    assert_eq!(
        file,
        f.app_dir
            .join(EXPORT_DIR_NAME)
            .join(format!("{EXPORT_PREFIX}21.txt"))
    );
    let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);

    let again = f.state.stop_logcat_recording(id).unwrap();
    assert_eq!(again, stopped);
    // The app consumed its copy: a repeated Stop hands it over again.
    std::fs::remove_file(&file).unwrap();
    let redelivered = f.state.stop_logcat_recording(id).unwrap();
    assert_eq!(redelivered, stopped);
    assert!(file.exists());

    // Start with the finished id answers with its status; it does not record again.
    let same = f.state.start_logcat_recording(id).unwrap();
    assert_eq!(same.state, LogcatSessionState::Stopped);
    assert_eq!(
        f.state.stop_logcat_recording(LogcatSessionId(99)),
        Err(LogcatRecordingError::UnknownSession)
    );
    assert_eq!(f.state.logcat().status().unwrap().file, None);
}

#[test]
fn the_byte_cap_auto_stops_and_marks_the_session_truncated() {
    let f = fixture(
        "yes",
        &["vpnhide-logcat-line"],
        64 * 1024,
        Duration::from_secs(60),
    );
    let id = LogcatSessionId(31);
    f.state.start_logcat_recording(id).unwrap();
    wait_until("the size cap", || {
        f.state.logcat().status().unwrap().state != LogcatSessionState::Recording
    });
    let status = f.state.logcat().status().unwrap();
    assert_eq!(
        status.state,
        LogcatSessionState::AutoStopped {
            reason: LogcatAutoStopReason::SizeCap
        }
    );
    assert!(status.truncated);
    assert_eq!(status.bytes, 64 * 1024);
    let stopped = f.state.stop_logcat_recording(id).unwrap();
    assert_eq!(stopped.state, status.state);
    let text = std::fs::read_to_string(stopped.file.unwrap()).unwrap();
    assert_eq!(text.len(), 64 * 1024);
    assert!(text.starts_with("vpnhide-logcat-line\n"));
    // A new session may start once the capped one ended.
    f.state.start_logcat_recording(LogcatSessionId(32)).unwrap();
    f.state.logcat().shutdown();
}

#[test]
fn the_duration_cap_kills_the_child() {
    let f = fixture("sleep", &["30"], 1 << 20, Duration::from_millis(300));
    let id = LogcatSessionId(41);
    let started = Instant::now();
    f.state.start_logcat_recording(id).unwrap();
    wait_until("the duration cap", || {
        f.state.logcat().status().unwrap().state != LogcatSessionState::Recording
    });
    assert!(started.elapsed() < Duration::from_secs(5));
    let status = f.state.logcat().status().unwrap();
    assert_eq!(
        status.state,
        LogcatSessionState::AutoStopped {
            reason: LogcatAutoStopReason::DurationCap
        }
    );
    assert!(!status.truncated);
    assert!(status.duration_ms >= 300);
}

#[test]
fn a_child_that_exits_by_itself_is_reported_with_its_exit() {
    let f = fixture("false", &[], 1 << 20, Duration::from_secs(60));
    let id = LogcatSessionId(51);
    f.state.start_logcat_recording(id).unwrap();
    wait_until("the exit", || {
        f.state.logcat().status().unwrap().state != LogcatSessionState::Recording
    });
    let status = f.state.logcat().status().unwrap();
    assert_eq!(
        status.state,
        LogcatSessionState::AutoStopped {
            reason: LogcatAutoStopReason::LogcatExited
        }
    );
    assert_eq!(status.logcat_exit, Some(1));
}

#[test]
fn a_missing_logcat_fails_start_and_keeps_no_session() {
    let f = fixture("/nonexistent/logcat", &[], 1 << 20, Duration::from_secs(60));
    let err = f
        .state
        .start_logcat_recording(LogcatSessionId(61))
        .unwrap_err();
    assert!(matches!(err, LogcatRecordingError::Failed(_)), "{err:?}");
    assert_eq!(f.state.logcat().status(), None);
    assert!(session_files(&f.core_dir).is_empty());
}

#[test]
fn shutdown_stops_the_running_child() {
    let f = idle();
    f.state.start_logcat_recording(LogcatSessionId(71)).unwrap();
    f.state.logcat().shutdown();
    assert_eq!(
        f.state.logcat().status().unwrap().state,
        LogcatSessionState::Stopped
    );
}

#[test]
fn stale_files_are_removed_at_start_and_by_the_next_session() {
    let f = idle();
    std::fs::create_dir_all(&f.core_dir).unwrap();
    std::fs::write(f.core_dir.join("session-1.log"), b"old run").unwrap();
    f.state.logcat().clean_stale();
    assert!(!f.core_dir.exists());

    let export = f.app_dir.join(EXPORT_DIR_NAME);
    std::fs::create_dir_all(&export).unwrap();
    std::fs::write(export.join(format!("{EXPORT_PREFIX}5.txt")), b"left over").unwrap();
    let first = LogcatSessionId(81);
    f.state.start_logcat_recording(first).unwrap();
    assert!(session_files(&export).is_empty());
    f.state.stop_logcat_recording(first).unwrap();
    assert_eq!(
        session_files(&export),
        vec![format!("{EXPORT_PREFIX}81.txt")]
    );

    f.state.start_logcat_recording(LogcatSessionId(82)).unwrap();
    assert_eq!(session_files(&f.core_dir), vec!["session-82.log"]);
    assert!(session_files(&export).is_empty());
    f.state.logcat().shutdown();
}

#[test]
fn recording_is_disabled_without_a_config() {
    let state = CoreState::new(Publisher::new(1, StateSnapshot::default()));
    state.logcat().set_config(None);
    assert_eq!(
        state.start_logcat_recording(LogcatSessionId(1)),
        Err(LogcatRecordingError::Disabled)
    );
}
