//! The production typed UniFFI API over a real multiplexed socket. No JSON command
//! facade is involved; canonical JSON is only the daemon's durable storage format.
use std::{path::Path, sync::Arc};
use vpnhide_core_daemon::{CoreState, Publisher, ServerHandle, serve};
use vpnhide_core_ffi::*;
use vpnhide_core_rpc::StateSnapshot;

struct Core {
    _runtime: tokio::runtime::Runtime,
    handle: ServerHandle,
    _state: Arc<CoreState>,
}
fn spawn_core(socket: &Path, config: &Path, epoch: u64) -> Core {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (state, handle) = runtime.block_on(async {
        let state = CoreState::load(
            Publisher::new(epoch, StateSnapshot::default()),
            config.to_path_buf(),
            None,
        )
        .unwrap();
        let handle = serve(socket, state.clone()).unwrap();
        (state, handle)
    });
    Core {
        _runtime: runtime,
        handle,
        _state: state,
    }
}
fn accepted(reply: SubmitReply) -> (u64, Option<CoreOperationRecord>, Option<StateReply>) {
    match reply {
        SubmitReply::Accepted { op, record, state } => (op, record, state.map(|state| *state)),
        other => panic!("{other:?}"),
    }
}
fn default_config() -> CoreCanonicalConfig {
    vpnhide_core_rpc::CanonicalConfig::default().into()
}
fn toggle(field: CoreConfigToggle, enabled: bool) -> CoreEdit {
    CoreEdit::Batch {
        edits: vec![CoreFieldEdit::Toggle { field, enabled }],
    }
}

#[test]
fn typed_replace_batch_conflict_deduplication_and_receipts() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("core.sock");
    let _core = spawn_core(&socket, &dir.path().join("config.json"), 77);
    let session = CoreSession::new(socket.display().to_string());
    rt.block_on(async {
        let initial = session.read_state().await.unwrap();
        assert_eq!(initial.state.epoch, 77);
        assert!(initial.state.config.is_none());
        assert_eq!(initial.connection_generation, 1);
        let op = session.allocate_operation_id().await.unwrap();
        let (_, record, state) = accepted(
            session
                .submit_edit(
                    op,
                    0,
                    CoreEdit::Replace {
                        config: default_config(),
                    },
                )
                .await
                .unwrap(),
        );
        assert_eq!(record.unwrap().settled, Some(CorePhaseOutcome::Confirmed));
        assert_eq!(state.unwrap().state.config_revision, 1);
        let batch = CoreEdit::Batch {
            edits: vec![
                CoreFieldEdit::Toggle {
                    field: CoreConfigToggle::DebugSwitch,
                    enabled: true,
                },
                CoreFieldEdit::Toggle {
                    field: CoreConfigToggle::RememberSuperkey,
                    enabled: true,
                },
            ],
        };
        let id = session.allocate_operation_id().await.unwrap();
        accepted(session.submit_edit(id, 1, batch.clone()).await.unwrap());
        // An unrelated stale edit still merges.
        let other = session.allocate_operation_id().await.unwrap();
        accepted(
            session
                .submit_edit(other, 1, toggle(CoreConfigToggle::AutoHideName, true))
                .await
                .unwrap(),
        );
        let conflicting = session.allocate_operation_id().await.unwrap();
        let refused = session
            .submit_edit(
                conflicting,
                1,
                CoreEdit::Batch {
                    edits: vec![
                        CoreFieldEdit::Toggle {
                            field: CoreConfigToggle::DebugSwitch,
                            enabled: false,
                        },
                        CoreFieldEdit::Toggle {
                            field: CoreConfigToggle::AutoHideServices,
                            enabled: false,
                        },
                    ],
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            refused,
            SubmitReply::Refused {
                reason: CoreSubmitRefusal::Conflict { .. }
            }
        ));
        let current = session.read_state().await.unwrap();
        let config = current.state.config.unwrap();
        assert!(config.debug_switch);
        assert!(config.settings.auto_hide_vpn_services);
        assert!(config.settings.auto_hide_vpn_name);
        assert_eq!(current.state.config_revision, 3);
        accepted(session.submit_edit(id, 1, batch).await.unwrap());
        assert!(matches!(
            session
                .submit_edit(id, 1, toggle(CoreConfigToggle::AutoHideName, false))
                .await
                .unwrap(),
            SubmitReply::Refused {
                reason: CoreSubmitRefusal::IdReused
            }
        ));
        let receipt = session.operation(id).await.unwrap();
        assert_eq!(receipt.record.unwrap().op, id);
        assert!(session.operation(u64::MAX).await.unwrap().record.is_none());
        assert!(matches!(
            session.submit_edit(0, 3, CoreEdit::RepairActivation).await,
            Err(CoreError::InvalidInput { .. })
        ));
    });
}

#[test]
fn long_poll_and_command_share_the_production_connection() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("core.sock");
    let _core = spawn_core(&socket, &dir.path().join("config.json"), 77);
    let session = CoreSession::new(socket.display().to_string());
    rt.block_on(async {
        let initial = session.read_state().await.unwrap();
        let id = session.allocate_operation_id().await.unwrap();
        let reader = session.clone();
        let poll = tokio::spawn(async move {
            reader
                .await_state(initial.state.epoch, initial.state.revision)
                .await
                .unwrap()
        });
        tokio::task::yield_now().await;
        accepted(
            session
                .submit_edit(
                    id,
                    0,
                    CoreEdit::Replace {
                        config: default_config(),
                    },
                )
                .await
                .unwrap(),
        );
        let update = tokio::time::timeout(std::time::Duration::from_secs(2), poll)
            .await
            .unwrap()
            .unwrap();
        assert!(update.state.revision > initial.state.revision);
        assert_eq!(update.connection_generation, initial.connection_generation);
    });
}

#[test]
fn failure_and_restarted_epoch_are_fenced_by_connection_generation() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("core.sock");
    let config = dir.path().join("config.json");
    let core = spawn_core(&socket, &config, 77);
    let session = CoreSession::new(socket.display().to_string());
    let first = rt.block_on(session.read_state()).unwrap();
    core.handle.shutdown();
    drop(core);
    let failed = rt.block_on(session.await_state(77, u64::MAX)).unwrap_err();
    let generation = match failed {
        CoreError::Transport { generation, .. } | CoreError::Unavailable { generation, .. } => {
            generation
        }
        other => panic!("{other:?}"),
    };
    assert_eq!(generation, first.connection_generation);
    let _restarted = spawn_core(&socket, &config, 88);
    let next = rt.block_on(session.await_state(77, u64::MAX)).unwrap();
    assert_eq!(next.state.epoch, 88);
    assert!(next.connection_generation > generation);
    assert!(next.state.revision < 100);
}

#[test]
fn absent_core_has_a_typed_unavailable_error() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let session = CoreSession::new(dir.path().join("none.sock").display().to_string());
    assert!(matches!(
        rt.block_on(session.read_state()),
        Err(CoreError::Unavailable { generation: 1, .. })
    ));
}
