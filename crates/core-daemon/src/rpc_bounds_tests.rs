use super::*;
use std::time::{Duration, Instant};

#[path = "root_probe_tests.rs"]
mod root_probe_tests;

fn test_state() -> Arc<CoreState> {
    CoreState::new(Publisher::new(42, Default::default()))
}

fn hold_writer(state: &CoreState) -> std::sync::mpsc::Sender<()> {
    let (entered, ready) = std::sync::mpsc::channel();
    let (release, gate) = std::sync::mpsc::channel();
    state
        .engine
        .enqueue_test(Box::new(move |_| {
            entered.send(()).unwrap();
            let _ = gate.recv();
        }))
        .unwrap();
    ready.recv_timeout(Duration::from_secs(2)).unwrap();
    release
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_and_expired_queued_commands_never_execute() {
    let state = test_state();
    let effect_root = tempfile::tempdir().unwrap();
    state.set_effect_paths(Some(crate::effects::EffectPaths {
        root: effect_root.path().into(),
    }));
    let id = state.issue_operation_id().unwrap();
    let gate = hold_writer(&state);
    let ticket =
        crate::command_engine::AdmissionTicket::new(Instant::now() + Duration::from_millis(50));
    let worker_ticket = ticket.clone();
    let worker_state = state.clone();
    let task = tokio::task::spawn_blocking(move || {
        worker_state
            .engine
            .submit(id, CoreCommand::CleanupLegacy, worker_ticket, None, None)
    });
    assert_eq!(task.await.unwrap(), Err(SubmitError::Busy));
    // Exercise actual handler drop, not just the ticket's cancellation method.
    let server = CoreServer::new(state.clone());
    let second =
        tokio::spawn(server.submit_command(Context::current(), id, CoreCommand::CleanupLegacy));
    tokio::time::timeout(Duration::from_secs(1), async {
        while state.recovery_rpc.available_permits() != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    second.abort();
    assert!(second.await.unwrap_err().is_cancelled());
    drop(gate);
    tokio::time::timeout(Duration::from_secs(2), async {
        while state.recovery_rpc.available_permits() != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(state.get_operation(id).is_none());
    assert!(state.snapshot().config.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_blocking_waiters_keep_their_budget_and_reads_stay_live() {
    let state = test_state();
    let gate = hold_writer(&state);
    let server = CoreServer::new(state.clone());
    let worker_server = server.clone();
    let task = tokio::spawn(worker_server.allocate_operation_id(Context::current()));
    tokio::time::timeout(Duration::from_secs(2), async {
        while state.work_rpc.available_permits() != 5 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let occupied = state.work_rpc.clone().acquire_many_owned(5).await.unwrap();
    task.abort();
    assert_eq!(
        server
            .clone()
            .allocate_operation_id(Context::current())
            .await,
        Err(SubmitError::Busy)
    );
    assert_eq!(state.work_rpc.available_permits(), 0);
    assert_eq!(state.recovery_rpc.available_permits(), 2);
    for _ in 0..100 {
        let current = tokio::time::timeout(
            Duration::from_millis(200),
            server.clone().state_since(Context::current(), 0, 0),
        )
        .await
        .unwrap();
        assert_eq!(current.epoch, 42);
        assert!(
            server
                .clone()
                .get_operation(Context::current(), OperationId(999))
                .await
                .is_none()
        );
    }
    // Workers time out even while the writer is still occupied. Queued closures
    // retain no permits and cannot allocate IDs after the writer is released.
    tokio::time::timeout(Duration::from_secs(3), async {
        while state.work_rpc.available_permits() != 1 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    drop(occupied);
    assert_eq!(state.work_rpc.available_permits(), 6);
    drop(gate);
    let next = tokio::task::spawn_blocking(move || state.issue_operation_id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(next, OperationId(1));
}

#[tokio::test]
async fn long_polls_have_separate_slots_and_overflow_returns_current_state() {
    let state = test_state();
    let current = state.publisher.current();
    let server = CoreServer::new(state);
    let mut pending = Vec::new();
    for _ in 0..2 {
        pending.push(tokio::spawn(server.clone().state_since(
            Context::current(),
            current.epoch,
            current.revision,
        )));
    }
    while server.polls.available_permits() != 0 {
        tokio::task::yield_now().await;
    }
    for _ in 0..100 {
        let reply = tokio::time::timeout(
            Duration::from_millis(100),
            server
                .clone()
                .state_since(Context::current(), current.epoch, current.revision),
        )
        .await
        .unwrap();
        assert_eq!(reply.revision, current.revision);
    }
    for task in pending {
        task.abort();
    }
}

#[test]
fn cancellation_after_writer_start_cannot_revoke_admission() {
    // Exercised through the real writer: cancellation after its first reply must
    // not suppress the durable command. Real socket disconnect coverage also lives
    // in journal_crash::durable_admission_precedes_slow_effect_and_survives_disconnect.
    let state = test_state();
    let id = state.issue_operation_id().unwrap();
    let ticket =
        crate::command_engine::AdmissionTicket::new(Instant::now() + Duration::from_secs(1));
    let result = state
        .engine
        .submit(
            id,
            CoreCommand::Edit {
                base: BaseRef { revision: 0 },
                edit: ConfigEdit::Replace {
                    config: CanonicalConfig::default(),
                },
            },
            ticket.clone(),
            None,
            None,
        )
        .unwrap();
    ticket.cancel();
    assert_eq!(result.op, id);
    // A writer barrier observes completion without re-submitting the operation.
    state.issue_operation_id().unwrap();
    assert_eq!(
        state.get_operation(id).unwrap().settled,
        Some(PhaseOutcome::Confirmed)
    );
}

#[tokio::test]
async fn saturated_forensics_does_not_consume_root_inventory_slot() {
    use vpnhide_core_rpc::DiagnosticProbe;
    let state = test_state();
    state.set_probe_env(crate::diagnostics::ProbeEnv {
        core_exe: "true".into(),
        app_dir: None,
    });
    let _forensic = state.heavy_rpc.clone().acquire_many_owned(2).await.unwrap();
    let server = CoreServer::new(state.clone());
    let refused = server
        .clone()
        .collect_diagnostics(
            Context::current(),
            DiagnosticsRequest {
                probes: vec![DiagnosticProbe::DebugSnapshot],
                ..Default::default()
            },
        )
        .await;
    assert_eq!(refused.outputs[0].output, "core diagnostics busy");
    let root = server
        .collect_diagnostics(
            Context::current(),
            DiagnosticsRequest {
                probes: vec![DiagnosticProbe::RootSnapshot { packages: true }],
                ..Default::default()
            },
        )
        .await;
    assert_eq!(root.outputs[0].exit, 0);
    assert_eq!(state.root_rpc.available_permits(), 1);
    assert_eq!(state.heavy_rpc.available_permits(), 0);
}

#[test]
fn preparation_time_does_not_consume_queue_budget_and_cancellation_still_fences_admission() {
    let state = test_state();
    let deadline = Instant::now() + Duration::from_secs(10);
    // Equivalent to a slow source/PM preparation before the actual queue ticket.
    std::thread::sleep(Duration::from_millis(2100));
    let cancelled = CancellationToken::new();
    let ticket = crate::command_engine::AdmissionTicket::after_preparation(deadline, cancelled);
    let id = state.issue_operation_id().unwrap();
    assert_eq!(
        state.engine.submit(
            id,
            CoreCommand::Edit {
                base: BaseRef { revision: 0 },
                edit: ConfigEdit::Replace {
                    config: CanonicalConfig::default()
                }
            },
            ticket,
            None,
            None
        ),
        Ok(Accepted { op: id })
    );
    for expired in [false, true] {
        let id = state.issue_operation_id().unwrap();
        let cancelled = CancellationToken::new();
        if !expired {
            cancelled.cancel();
        }
        let ticket = crate::command_engine::AdmissionTicket::after_preparation(
            if expired {
                Instant::now() - Duration::from_millis(1)
            } else {
                Instant::now() + Duration::from_secs(2)
            },
            cancelled,
        );
        assert_eq!(
            state.engine.submit(
                id,
                CoreCommand::Edit {
                    base: BaseRef {
                        revision: state.config_revision()
                    },
                    edit: ConfigEdit::Toggle {
                        field: vpnhide_core_rpc::ConfigToggle::DebugSwitch,
                        enabled: true
                    }
                },
                ticket,
                None,
                None
            ),
            Err(SubmitError::Busy)
        );
        assert!(state.get_operation_record(id).is_none());
    }
    assert!(!state.snapshot().config.unwrap().debug_switch);
}
