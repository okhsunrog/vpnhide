//! Core-owned logcat sessions: idempotency, Busy, the byte and duration caps, a child that
//! exits by itself, delivery into the app's export dir, the debug-logging lease (taken on
//! Start, restored on every end and after a core restart, never over a user change), and
//! the undelivered-recording lifecycle (ack, discard, kept across restarts). A real child
//! runs (`sleep` / `yes` / `false` stand in for `logcat`).

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use vpnhide_core_daemon::diagnostics::{EXPORT_DIR_NAME, ProbeEnv};
use vpnhide_core_daemon::logcat_session::{EXPORT_PREFIX, RecorderConfig};
use vpnhide_core_daemon::{CoreState, Publisher};
use vpnhide_core_rpc::{
    BaseRef, CanonicalConfig, ConfigEdit, ConfigToggle, LogcatAutoStopReason, LogcatDebugRestore,
    LogcatRecordingError, LogcatSessionId, LogcatSessionState, LogcatSessionStatus, StateSnapshot,
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
    let state = core(&core_dir, &app_dir, program, args, max_bytes, max_duration);
    Fixture {
        state,
        core_dir,
        app_dir,
        _tmp: tmp,
    }
}

/// A fresh core run over the same dirs (what a restart sees).
fn core(
    core_dir: &Path,
    app_dir: &Path,
    program: &str,
    args: &[&str],
    max_bytes: u64,
    max_duration: Duration,
) -> Arc<CoreState> {
    let state_dir = core_dir.parent().unwrap();
    let state = CoreState::load_with_journal(
        Publisher::new(1, StateSnapshot::default()),
        state_dir.join("canonical.json"),
        Some(state_dir.join("operations.json")),
        None,
    )
    .unwrap();
    attach(
        &state,
        core_dir,
        app_dir,
        program,
        args,
        max_bytes,
        max_duration,
    );
    state
}

fn attach(
    state: &CoreState,
    core_dir: &Path,
    app_dir: &Path,
    program: &str,
    args: &[&str],
    max_bytes: u64,
    max_duration: Duration,
) {
    state.set_probe_env(ProbeEnv {
        core_exe: PathBuf::from("/nonexistent"),
        app_dir: Some(app_dir.to_path_buf()),
    });
    state.logcat().set_config(Some(RecorderConfig {
        program: PathBuf::from(program),
        args: args.iter().map(|a| a.to_string()).collect(),
        dir: core_dir.to_path_buf(),
        max_bytes,
        max_duration,
        since_start: false,
    }));
}

/// An edit the app would send, against the current revision.
fn edit(state: &CoreState, edit: ConfigEdit) {
    let base = BaseRef {
        revision: state.config_revision(),
    };
    state
        .submit_edit_public(state.issue_operation_id().unwrap(), base, &edit)
        .unwrap();
}

fn bootstrap(state: &CoreState, debug: bool, debug_switch: bool) {
    edit(
        state,
        ConfigEdit::Replace {
            config: CanonicalConfig {
                debug,
                debug_switch,
                ..CanonicalConfig::default()
            },
        },
    );
}

fn toggle(state: &CoreState, field: ConfigToggle, enabled: bool) {
    edit(state, ConfigEdit::Toggle { field, enabled });
}

/// `(debug, debugSwitch)` of the confirmed config.
fn debug_of(state: &CoreState) -> (bool, bool) {
    let config = state.snapshot().config.expect("config");
    (config.debug, config.debug_switch)
}

fn restore_of(status: &LogcatSessionStatus) -> Option<LogcatDebugRestore> {
    status.debug.map(|l| l.restore)
}

/// Wait until the session's lease left `Held` (the supervisor restores after the end).
fn released(state: &CoreState) -> LogcatSessionStatus {
    wait_until("the debug restore", || {
        let status = state.logcat().status().unwrap();
        status.state != LogcatSessionState::Recording
            && !matches!(restore_of(&status), Some(LogcatDebugRestore::Held))
            && !state.debug_lease_held()
    });
    state.logcat().status().unwrap()
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
fn recording_durability_failure_precedes_logging_lease_and_child_spawn() {
    let f = fixture("sleep", &["30"], 1024, Duration::from_secs(60));
    bootstrap(&f.state, false, false);
    std::fs::create_dir_all(f.core_dir.join("session.json")).unwrap();
    assert!(f.state.start_logcat_recording(LogcatSessionId(9)).is_err());
    let snapshot = f.state.publisher.current().snapshot;
    assert!(!snapshot.config.unwrap().debug);
    assert!(!snapshot.debug_lease);
    assert!(snapshot.debug_leases.is_empty());
    assert!(!f.state.logcat().is_active());
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
    assert_eq!(
        session_files(&f.core_dir),
        vec!["session-11.log", "session.json"]
    );
    let mode = std::fs::metadata(f.core_dir.join("session-11.log"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    f.state.logcat().shutdown();
}

#[test]
fn stop_hands_over_a_copy_and_repeats_the_same_result() {
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

    assert!(!stopped.delivered);
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
fn recover_removes_files_without_a_session_record_and_start_clears_old_exports() {
    let f = idle();
    std::fs::create_dir_all(&f.core_dir).unwrap();
    std::fs::write(f.core_dir.join("session-1.log"), b"old run").unwrap();
    f.state.logcat().recover();
    assert!(!f.core_dir.exists());
    assert_eq!(f.state.logcat().status(), None);

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

    // A new Start replaces the finished, undelivered session and its copies.
    f.state.start_logcat_recording(LogcatSessionId(82)).unwrap();
    assert_eq!(
        session_files(&f.core_dir),
        vec!["session-82.log", "session.json"]
    );
    assert!(session_files(&export).is_empty());
    assert_eq!(
        f.state.stop_logcat_recording(first),
        Err(LogcatRecordingError::UnknownSession)
    );
    f.state.logcat().shutdown();
}

#[test]
fn start_turns_debug_on_and_stop_restores_it() {
    let f = idle();
    bootstrap(&f.state, false, false);
    let id = LogcatSessionId(101);
    let started = f.state.start_logcat_recording(id).unwrap();
    let lease = started.debug.expect("lease");
    assert!(!lease.previous_debug && !lease.previous_debug_switch);
    assert_eq!(lease.restore, LogcatDebugRestore::Held);
    assert_eq!(debug_of(&f.state), (true, false));
    assert!(f.state.snapshot().debug_lease);

    // The app's own writes (here: debug = switch, as without a capture) can't turn it off.
    edit(
        &f.state,
        ConfigEdit::Replace {
            config: CanonicalConfig::default(),
        },
    );
    toggle(&f.state, ConfigToggle::Debug, false);
    assert_eq!(debug_of(&f.state), (true, false));

    f.state.stop_logcat_recording(id).unwrap();
    let status = released(&f.state);
    assert_eq!(restore_of(&status), Some(LogcatDebugRestore::Restored));
    assert_eq!(debug_of(&f.state), (false, false));
    assert!(!f.state.snapshot().debug_lease);
    // Once released, edits are the user's again.
    toggle(&f.state, ConfigToggle::Debug, true);
    assert_eq!(debug_of(&f.state), (true, false));
}

#[test]
fn debug_that_was_already_on_stays_on() {
    let f = idle();
    bootstrap(&f.state, true, true);
    let id = LogcatSessionId(102);
    let revision = f.state.config_revision();
    f.state.start_logcat_recording(id).unwrap();
    // Nothing to write: debug was on already.
    assert_eq!(f.state.config_revision(), revision);
    f.state.stop_logcat_recording(id).unwrap();
    let status = released(&f.state);
    assert_eq!(restore_of(&status), Some(LogcatDebugRestore::Restored));
    assert_eq!(debug_of(&f.state), (true, true));
    assert_eq!(f.state.config_revision(), revision);
}

#[test]
fn a_user_switch_change_during_the_session_wins_over_the_restore() {
    // Switched on during the session: debug stays on.
    let f = idle();
    bootstrap(&f.state, false, false);
    let on = LogcatSessionId(111);
    f.state.start_logcat_recording(on).unwrap();
    toggle(&f.state, ConfigToggle::DebugSwitch, true);
    f.state.stop_logcat_recording(on).unwrap();
    assert_eq!(
        restore_of(&released(&f.state)),
        Some(LogcatDebugRestore::UserChoice)
    );
    assert_eq!(debug_of(&f.state), (true, true));

    // Switched off during the session: debug goes off although it was on before.
    let off = LogcatSessionId(112);
    f.state.start_logcat_recording(off).unwrap();
    toggle(&f.state, ConfigToggle::DebugSwitch, false);
    assert_eq!(debug_of(&f.state), (true, false));
    f.state.stop_logcat_recording(off).unwrap();
    assert_eq!(
        restore_of(&released(&f.state)),
        Some(LogcatDebugRestore::UserChoice)
    );
    assert_eq!(debug_of(&f.state), (false, false));
}

#[test]
fn every_auto_stop_restores_debug() {
    for (program, args, max_bytes, reason) in [
        (
            "yes",
            &["vpnhide-logcat-line"][..],
            64 * 1024,
            LogcatAutoStopReason::SizeCap,
        ),
        (
            "false",
            &[][..],
            1 << 20,
            LogcatAutoStopReason::LogcatExited,
        ),
    ] {
        let f = fixture(program, args, max_bytes, Duration::from_secs(60));
        bootstrap(&f.state, false, false);
        f.state
            .start_logcat_recording(LogcatSessionId(121))
            .unwrap();
        let status = released(&f.state);
        assert_eq!(status.state, LogcatSessionState::AutoStopped { reason });
        assert_eq!(restore_of(&status), Some(LogcatDebugRestore::Restored));
        assert_eq!(debug_of(&f.state), (false, false));
    }
    let f = fixture("sleep", &["30"], 1 << 20, Duration::from_millis(300));
    bootstrap(&f.state, false, false);
    f.state
        .start_logcat_recording(LogcatSessionId(122))
        .unwrap();
    let status = released(&f.state);
    assert_eq!(
        status.state,
        LogcatSessionState::AutoStopped {
            reason: LogcatAutoStopReason::DurationCap
        }
    );
    assert_eq!(restore_of(&status), Some(LogcatDebugRestore::Restored));
    assert_eq!(debug_of(&f.state), (false, false));
}

#[test]
fn without_a_config_the_session_records_without_a_lease() {
    let f = idle();
    let status = f
        .state
        .start_logcat_recording(LogcatSessionId(131))
        .unwrap();
    assert_eq!(status.debug, None);
    assert!(!f.state.debug_lease_held());
    f.state.logcat().shutdown();
}

/// What a crash leaves: the record and file as they were while recording.
fn crash_copy(f: &Fixture) -> Vec<(PathBuf, Vec<u8>)> {
    let mut paths: Vec<PathBuf> = session_files(&f.core_dir)
        .into_iter()
        .map(|name| f.core_dir.join(name))
        .collect();
    // The debug baseline belongs to the journal, independently of recorder files.
    for name in ["canonical.json", "operations.json"] {
        let path = f.core_dir.parent().unwrap().join(name);
        if path.exists() {
            paths.push(path);
        }
    }
    paths
        .into_iter()
        .map(|path| {
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect()
}

#[test]
fn a_session_unfinished_at_core_start_ends_and_restores_debug() {
    let f = idle();
    bootstrap(&f.state, false, false);
    let id = LogcatSessionId(141);
    f.state.start_logcat_recording(id).unwrap();
    let left = crash_copy(&f);
    f.state.shutdown();
    for (path, bytes) in &left {
        std::fs::write(path, bytes).unwrap();
    }
    // What logcat had written by the crash.
    std::fs::write(f.core_dir.join("session-141.log"), b"line\n").unwrap();

    // The next run: the config file still says debug = true (the lease's write).
    let next = core(
        &f.core_dir,
        &f.app_dir,
        "sleep",
        &["30"],
        1 << 20,
        Duration::from_secs(60),
    );
    next.logcat().recover();
    let status = next.logcat().status().expect("recovered");
    assert_eq!(status.session, id);
    assert_eq!(
        status.state,
        LogcatSessionState::AutoStopped {
            reason: LogcatAutoStopReason::CoreRestarted
        }
    );
    assert_eq!(status.bytes, 5);
    assert!(!status.delivered);
    assert_eq!(restore_of(&status), Some(LogcatDebugRestore::Restored));
    assert_eq!(debug_of(&next), (false, false));
    assert!(!next.debug_lease_held());
    // Still offered: Stop hands it over, ack ends it.
    let stopped = next.stop_logcat_recording(id).unwrap();
    let file = stopped.file.expect("copy");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "line\n");
    next.ack_logcat_recording(id).unwrap();
}

#[test]
fn a_restart_does_not_clobber_a_switch_the_user_changed() {
    let f = idle();
    bootstrap(&f.state, false, false);
    let id = LogcatSessionId(142);
    f.state.start_logcat_recording(id).unwrap();
    // The choice belongs to the durable pre-crash state, not a racing import
    // submitted while the restarted core is restoring its capture baseline.
    edit(
        &f.state,
        ConfigEdit::Toggle {
            field: ConfigToggle::DebugSwitch,
            enabled: true,
        },
    );
    let left = crash_copy(&f);
    f.state.shutdown();
    for (path, bytes) in &left {
        std::fs::write(path, bytes).unwrap();
    }
    let next = core(
        &f.core_dir,
        &f.app_dir,
        "sleep",
        &["30"],
        1 << 20,
        Duration::from_secs(60),
    );
    next.logcat().recover();
    let status = next.logcat().status().expect("recovered");
    assert_eq!(restore_of(&status), Some(LogcatDebugRestore::UserChoice));
    assert_eq!(debug_of(&next), (true, true));
}

#[test]
fn ack_marks_delivered_deletes_core_copies_and_is_idempotent() {
    let f = idle();
    let id = LogcatSessionId(151);
    f.state.start_logcat_recording(id).unwrap();
    assert_eq!(
        f.state.ack_logcat_recording(id),
        Err(LogcatRecordingError::Invalid(
            "the session is still recording".into()
        ))
    );
    let stopped = f.state.stop_logcat_recording(id).unwrap();
    let copy = PathBuf::from(stopped.file.unwrap());
    let acked = f.state.ack_logcat_recording(id).unwrap();
    assert!(acked.delivered);
    assert_eq!(acked.file, None);
    assert!(!copy.exists());
    assert_eq!(session_files(&f.core_dir), vec!["session.json"]);
    assert_eq!(f.state.ack_logcat_recording(id).unwrap(), acked);
    // A late Stop (lost ack reply) answers without a file instead of failing.
    let late = f.state.stop_logcat_recording(id).unwrap();
    assert!(late.delivered);
    assert_eq!(late.file, None);
    assert_eq!(
        f.state.ack_logcat_recording(LogcatSessionId(9)),
        Err(LogcatRecordingError::UnknownSession)
    );
    // Delivered sessions are not kept across a restart.
    let next = core(
        &f.core_dir,
        &f.app_dir,
        "sleep",
        &["30"],
        1 << 20,
        Duration::from_secs(60),
    );
    next.logcat().recover();
    assert_eq!(next.logcat().status(), None);
    assert!(!f.core_dir.exists());
}

#[test]
fn an_undelivered_recording_is_offered_after_a_restart() {
    let f = idle();
    let id = LogcatSessionId(161);
    f.state.start_logcat_recording(id).unwrap();
    let first = f.state.stop_logcat_recording(id).unwrap();
    // The app died before saving (its copy is gone with its cache).
    std::fs::remove_file(first.file.as_ref().unwrap()).unwrap();
    f.state.logcat().shutdown();

    let next = core(
        &f.core_dir,
        &f.app_dir,
        "sleep",
        &["30"],
        1 << 20,
        Duration::from_secs(60),
    );
    next.logcat().recover();
    let offered = next.logcat().status().expect("offered");
    assert_eq!(offered.session, id);
    assert_eq!(offered.state, LogcatSessionState::Stopped);
    assert!(!offered.delivered);
    assert_eq!(offered.bytes, first.bytes);
    assert_eq!(offered.started_at_ms, first.started_at_ms);
    // Restarting again keeps it too.
    let again = core(
        &f.core_dir,
        &f.app_dir,
        "sleep",
        &["30"],
        1 << 20,
        Duration::from_secs(60),
    );
    again.logcat().recover();
    assert_eq!(again.logcat().status(), Some(offered.clone()));
    let saved = again.stop_logcat_recording(id).unwrap();
    assert!(PathBuf::from(saved.file.unwrap()).exists());
    assert!(again.ack_logcat_recording(id).unwrap().delivered);
}

#[test]
fn discard_stops_restores_and_deletes_everything() {
    let f = idle();
    bootstrap(&f.state, false, false);
    let id = LogcatSessionId(171);
    f.state.start_logcat_recording(id).unwrap();
    assert_eq!(debug_of(&f.state), (true, false));
    f.state.discard_logcat_recording(id).unwrap();
    assert_eq!(f.state.logcat().status(), None);
    assert_eq!(debug_of(&f.state), (false, false));
    assert!(!f.state.debug_lease_held());
    assert!(session_files(&f.core_dir).is_empty());
    // Idempotent, and an unknown id is already gone.
    f.state.discard_logcat_recording(id).unwrap();
    f.state
        .discard_logcat_recording(LogcatSessionId(5))
        .unwrap();

    // A finished, delivered-to-export session: the copy goes too.
    let second = LogcatSessionId(172);
    f.state.start_logcat_recording(second).unwrap();
    let copy = PathBuf::from(f.state.stop_logcat_recording(second).unwrap().file.unwrap());
    f.state.discard_logcat_recording(second).unwrap();
    assert!(!copy.exists());
    assert!(session_files(&f.core_dir).is_empty());
    let next = core(
        &f.core_dir,
        &f.app_dir,
        "sleep",
        &["30"],
        1 << 20,
        Duration::from_secs(60),
    );
    next.logcat().recover();
    assert_eq!(next.logcat().status(), None);
}

#[test]
fn a_known_failed_restore_can_be_explicitly_retried_after_storage_is_fixed() {
    let tmp = tempfile::tempdir().unwrap();
    let core_dir = tmp.path().join("core/logcat");
    let app_dir = tmp.path().join("app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let config_path = tmp.path().join("config.json");
    let state = CoreState::load(
        Publisher::new(1, StateSnapshot::default()),
        config_path.clone(),
        None,
    )
    .unwrap();
    attach(
        &state,
        &core_dir,
        &app_dir,
        "sleep",
        &["30"],
        1 << 20,
        Duration::from_secs(60),
    );
    bootstrap(&state, false, false);
    let id = LogcatSessionId(181);
    state.start_logcat_recording(id).unwrap();
    assert_eq!(debug_of(&state), (true, false));
    // The config file can't be replaced now (a dir stands in its place).
    std::fs::remove_file(&config_path).unwrap();
    std::fs::create_dir(&config_path).unwrap();
    state.stop_logcat_recording(id).unwrap();
    let status = released(&state);
    assert_eq!(restore_of(&status), Some(LogcatDebugRestore::Pending));
    assert_eq!(debug_of(&state), (true, false));
    // An explicit release retries a known failure with a fresh receipt. It must
    // not rewrite the original receipt or replay an Unknown dispatch.
    std::fs::remove_dir(&config_path).unwrap();
    state
        .release_debug_lease(vpnhide_core_rpc::DebugLeaseId(u64::MAX))
        .unwrap();
    let status = state.logcat().status().unwrap();
    assert_eq!(restore_of(&status), Some(LogcatDebugRestore::Restored));
    assert_eq!(debug_of(&state), (false, false));
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
