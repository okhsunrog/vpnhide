//! Fixed root probes and core-owned recording through the generated API's typed surface.
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use vpnhide_core_daemon::{CoreState, Publisher, ServerHandle, diagnostics::ProbeEnv, serve};
use vpnhide_core_ffi::*;
use vpnhide_core_rpc::StateSnapshot;
struct Core {
    _runtime: tokio::runtime::Runtime,
    _handle: ServerHandle,
    state: Arc<CoreState>,
}
fn spawn_core(socket: &Path, config: &Path) -> Core {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (state, handle) = runtime.block_on(async {
        let state = CoreState::load(
            Publisher::new(5, StateSnapshot::default()),
            config.to_path_buf(),
            None,
        )
        .unwrap();
        state.set_probe_env(ProbeEnv {
            core_exe: PathBuf::from("/nonexistent"),
            app_dir: None,
        });
        let handle = serve(socket, state.clone()).unwrap();
        (state, handle)
    });
    Core {
        _runtime: runtime,
        _handle: handle,
        state,
    }
}
#[test]
fn diagnostics_are_a_closed_typed_probe_vocabulary() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("core.sock");
    let _core = spawn_core(&socket, &dir.path().join("config.json"));
    let session = CoreSession::new(socket.display().to_string());
    let reply = rt
        .block_on(session.collect_diagnostics(CoreDiagnosticsRequest {
            probes: vec![
                CoreDiagnosticProbe::HookCounters,
                CoreDiagnosticProbe::GroundTruthChecks,
            ],
            clear_dmesg: false,
        }))
        .unwrap();
    assert_eq!(reply.outputs.len(), 2);
    assert_eq!(reply.outputs[0].probe, CoreDiagnosticProbe::HookCounters);
    assert!(!reply.dmesg_cleared);
}
fn session(reply: LogcatReply) -> Option<CoreLogcatSessionStatus> {
    match reply {
        LogcatReply::Ok { session } => session,
        other => panic!("{other:?}"),
    }
}
#[test]
fn recording_lifecycle_and_refusals_are_typed() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("core.sock");
    let core = spawn_core(&socket, &dir.path().join("config.json"));
    let app_dir = dir.path().join("app");
    std::fs::create_dir_all(&app_dir).unwrap();
    core.state.set_probe_env(ProbeEnv {
        core_exe: PathBuf::from("/nonexistent"),
        app_dir: Some(app_dir),
    });
    let client = CoreSession::new(socket.display().to_string());
    let call = |request| rt.block_on(client.logcat_recording(request)).unwrap();
    core.state.logcat().set_config(None);
    assert_eq!(
        call(CoreLogcatRequest::Start { session: 7 }),
        LogcatReply::Refused {
            reason: CoreLogcatError::Disabled
        }
    );
    core.state
        .logcat()
        .set_config(Some(vpnhide_core_daemon::logcat_session::RecorderConfig {
            program: PathBuf::from("sleep"),
            args: vec!["30".into()],
            dir: dir.path().join("core/logcat"),
            max_bytes: 1 << 20,
            max_duration: std::time::Duration::from_secs(60),
            since_start: false,
        }));
    assert!(session(call(CoreLogcatRequest::Status)).is_none());
    let recording = session(call(CoreLogcatRequest::Start { session: 7 })).unwrap();
    assert_eq!(recording.state, CoreLogcatSessionState::Recording);
    assert_eq!(
        call(CoreLogcatRequest::Start { session: 8 }),
        LogcatReply::Refused {
            reason: CoreLogcatError::Busy { running: 7 }
        }
    );
    assert_eq!(
        call(CoreLogcatRequest::Stop { session: 8 }),
        LogcatReply::Refused {
            reason: CoreLogcatError::UnknownSession
        }
    );
    let stopped = session(call(CoreLogcatRequest::Stop { session: 7 })).unwrap();
    assert_eq!(stopped.state, CoreLogcatSessionState::Stopped);
    let file = stopped.file.as_ref().unwrap();
    assert!(Path::new(file).exists());
    let stopped_again = session(call(CoreLogcatRequest::Stop { session: 7 })).unwrap();
    assert_eq!(stopped_again, stopped);
    let acked = session(call(CoreLogcatRequest::Ack { session: 7 })).unwrap();
    assert!(acked.delivered);
    assert!(!Path::new(file).exists());
    assert_eq!(
        session(call(CoreLogcatRequest::Ack { session: 7 })).unwrap(),
        acked
    );
    assert!(session(call(CoreLogcatRequest::Discard { session: 7 })).is_none());
    assert!(session(call(CoreLogcatRequest::Status)).is_none());
}

#[test]
fn compound_command_crosses_the_typed_boundary_as_one_receipt() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("core.sock");
    let core = spawn_core(&socket, &dir.path().join("config.json"));
    let root = dir.path().join("root");
    core.state
        .set_effect_paths(Some(vpnhide_core_daemon::effects::EffectPaths {
            root: root.clone(),
        }));
    let session = CoreSession::new(socket.display().to_string());
    rt.block_on(async {
        let bootstrap = session.allocate_operation_id().await.unwrap();
        assert!(matches!(
            session
                .submit_command(
                    bootstrap,
                    CoreCommand::Import {
                        base_revision: 0,
                        config: vpnhide_core_rpc::CanonicalConfig::default().into(),
                        cleanup_legacy: false
                    }
                )
                .await
                .unwrap(),
            SubmitReply::Accepted { .. }
        ));
        let op = session.allocate_operation_id().await.unwrap();
        let command = CoreCommand::SetSuperkey {
            base_revision: 1,
            remember: true,
            superkey: "test-superkey".into(),
        };
        let reply = session.submit_command(op, command.clone()).await.unwrap();
        let SubmitReply::Accepted {
            record: Some(record),
            state: Some(state),
            ..
        } = reply
        else {
            panic!("{reply:?}")
        };
        assert_eq!(record.settled, Some(CorePhaseOutcome::Confirmed));
        assert!(
            record
                .phases
                .iter()
                .any(|phase| phase.phase == CoreConfigPhase::Secret
                    && phase.outcome == CorePhaseOutcome::Confirmed)
        );
        assert!(state.state.config.unwrap().settings.remember_superkey);
        assert_eq!(
            std::fs::read_to_string(root.join("data/adb/vpnhide/superkey")).unwrap(),
            "test-superkey\n"
        );
        // Secret values are never journaled. Recovery queries the durable receipt;
        // resubmission cannot prove original secret equality and must refuse.
        assert!(matches!(
            session.submit_command(op, command).await.unwrap(),
            SubmitReply::Refused {
                reason: CoreSubmitRefusal::IdReused
            }
        ));
        assert_eq!(
            session.operation(op).await.unwrap().record.unwrap().settled,
            Some(CorePhaseOutcome::Confirmed)
        );
    });
}
