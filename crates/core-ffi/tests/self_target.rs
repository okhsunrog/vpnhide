//! Generated command over a real socket, with Core-derived intent and recovery.
mod common;
use vpnhide_core_daemon::{CoreState, Publisher, effects::EffectPaths, serve};
use vpnhide_core_ffi::*;
use vpnhide_core_rpc::StateSnapshot;

#[test]
fn preparation_crosses_ffi_and_reconnect_queries_the_same_operation() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("core.sock");
    let (state, _server) = rt.block_on(async {
        let state = CoreState::load_with_journal(
            Publisher::new(19, StateSnapshot::default()),
            dir.path().join("config.json"),
            Some(dir.path().join("journal.json")),
            None,
        )
        .unwrap();
        let legacy = dir.path().join("legacy");
        std::fs::create_dir(&legacy).unwrap();
        state.set_effect_paths(Some(EffectPaths { root: legacy }));
        let server = serve(&socket, state.clone()).unwrap();
        (state, server)
    });
    rt.block_on(async {
        let session = CoreSession::new(socket.display().to_string());
        let op = session.allocate_operation_id().await.unwrap();
        assert!(matches!(
            session
                .submit_command(op, CoreCommand::PrepareSelfTargets { base_revision: 0 })
                .await
                .unwrap(),
            SubmitReply::Accepted { .. }
        ));
        let (record, current) = common::settled(&session, op).await;
        assert_eq!(record.settled, Some(CorePhaseOutcome::Confirmed));
        assert_eq!(record.phases.len(), 1);
        assert_eq!(record.phases[0].phase, CoreConfigPhase::Persist);
        let config = current.state.config.unwrap();
        let app = &config.apps[vpnhide_migration_policy::APP_PACKAGE];
        assert!(app.java && app.native.enabled && app.hidden && !app.ports);
        let revision = current.state.config_revision;
        drop(session);
        let reconnected = CoreSession::new(socket.display().to_string());
        assert_eq!(
            reconnected.operation(op).await.unwrap().record,
            Some(record)
        );
        assert_eq!(
            reconnected
                .read_state()
                .await
                .unwrap()
                .state
                .config_revision,
            revision
        );
    });
    state.shutdown();
}
