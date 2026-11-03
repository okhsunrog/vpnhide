//! Command-owner tests use the actual durable queue and receipt store.
use super::*;
use std::fs;
use vpnhide_core_rpc::{ConfigToggle, StateSnapshot};

fn load(root: &std::path::Path, epoch: u64) -> Arc<CoreState> {
    CoreState::load_with_journal(
        Publisher::new(epoch, StateSnapshot::default()),
        root.join("config.json"),
        Some(root.join("operations.json")),
        None,
    )
    .unwrap()
}

fn command(state: &CoreState, edit: ConfigEdit) -> OperationId {
    let op = state.issue_operation_id().unwrap();
    state
        .submit_command(
            op,
            CoreCommand::Edit {
                base: BaseRef {
                    revision: state.config_revision(),
                },
                edit,
            },
        )
        .unwrap();
    op
}

fn bootstrap(state: &CoreState) {
    command(
        state,
        ConfigEdit::Replace {
            config: CanonicalConfig::default(),
        },
    );
}

fn uncertain(
    op: u64,
    phase: ConfigPhase,
    candidate: CanonicalConfig,
    revision: u64,
) -> JournalEntry {
    JournalEntry {
        maintenance: None,
        maintenance_report: None,
        admission: Some(journal::Admission {
            from_revision: revision,
            commit_revision: revision,
            writes: Vec::new(),
        }),
        command: Some(journal::CommandIntent::Edit),
        superseded_by: None,
        superseded_phases: Vec::new(),
        finalized_unknown: true,
        base: BaseRef { revision },
        edit: ConfigEdit::RepairActivation,
        candidate: Some(candidate),
        native_intent: None,
        record: OperationRecord {
            op: OperationId(op),
            phases: vec![PhaseRecord {
                phase,
                outcome: PhaseOutcome::Unknown,
            }],
            settled: Some(PhaseOutcome::Unknown),
            ports: None,
        },
    }
}

fn native_none(state: &CoreState, root: &std::path::Path) {
    use vpnhide_core_rpc::{NativeBootOutcome, NativeBootStatus, ZygiskTargets};
    let env = crate::native_boot::NativeEnv {
        ctl: root.join("missing-ctl"),
        modules: None,
        kpm_lock: None,
        zygisk_module: None,
        boot_id_path: root.join("boot-id"),
        state_file: root.join("native.json"),
        legacy_root: None,
        apatch_kpms: root.join("apatch"),
        payload_override: None,
        bundle_identity: None,
        loader: None,
    };
    state.install_native_boot(crate::native_boot::NativeBoot {
        env,
        selection: None,
        status: NativeBootStatus {
            boot_id: "test".into(),
            decided_epoch: 2,
            decided_at_ms: 0,
            updated_at_ms: 0,
            outcome: NativeBootOutcome::NoBackend,
            installed: Vec::new(),
            live: Vec::new(),
            staged: Vec::new(),
            kmod: None,
            zygisk_targets: ZygiskTargets::NotInstalled,
            ports: None,
            obsolete_modules: Vec::new(),
        },
    });
}

fn landed_unknown(
    state: &CoreState,
    candidate: CanonicalConfig,
    writes: Vec<String>,
) -> OperationId {
    let op = state.issue_operation_id().unwrap();
    config_file::write_atomic(state.config_path.as_deref().unwrap(), &candidate).unwrap();
    state
        .call_writer(move |state| {
            let mut store = state.engine.store.lock().unwrap().clone();
            let mut entry = uncertain(op.0, ConfigPhase::Persist, candidate, store.config_revision);
            entry.admission = Some(journal::Admission {
                from_revision: store.config_revision,
                commit_revision: store.config_revision + 1,
                writes,
            });
            store.journal.upsert(entry);
            state.persist_store(&store).unwrap();
        })
        .unwrap();
    op
}

#[test]
fn new_leases_and_credentials_refuse_uncertain_persist_before_capturing_stale_state() {
    let _env = crate::ports::test_env_lock();
    let root = tempfile::tempdir().unwrap();
    let state = load(root.path(), 1);
    std::fs::create_dir(root.path().join("root")).unwrap();
    state.set_effect_paths(Some(crate::effects::EffectPaths {
        root: root.path().join("root"),
    }));
    bootstrap(&state);
    let candidate = CanonicalConfig {
        debug: true,
        settings: vpnhide_core_rpc::Settings {
            auto_hide_vpn_name: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let old = landed_unknown(
        &state,
        candidate.clone(),
        vec!["debug".into(), "settings/autoHideVpnName".into()],
    );
    assert_eq!(
        state.acquire_debug_lease(DebugLeaseId(10), DebugLeaseOwner::Forensic),
        Err(SubmitError::Unavailable)
    );
    assert!(
        journal::read(&root.path().join("operations.json"))
            .unwrap()
            .debug_restore
            .is_none()
    );
    let op = state.issue_operation_id().unwrap();
    assert_eq!(
        state.submit_command(
            op,
            CoreCommand::SetSuperkey {
                base: BaseRef { revision: 1 },
                remember: true,
                superkey: "test-secret".into()
            }
        ),
        Err(SubmitError::Unavailable)
    );
    assert_eq!(
        config_file::read(&root.path().join("config.json")).unwrap(),
        Some(candidate)
    );
    assert!(!root.path().join("root/data/adb/vpnhide/superkey").exists());
    command(&state, ConfigEdit::RepairActivation);
    state
        .acquire_debug_lease(DebugLeaseId(10), DebugLeaseOwner::Forensic)
        .unwrap();
    state.release_debug_lease(DebugLeaseId(10)).unwrap();
    let current = state.snapshot().config.unwrap();
    assert!(
        current.debug && current.settings.auto_hide_vpn_name,
        "the new lease captured the repaired durable baseline"
    );
    assert_eq!(
        state.get_operation_record(old).unwrap().settled,
        Some(PhaseOutcome::Unknown)
    );
}

#[test]
fn existing_lease_release_uses_fresh_user_switch_and_preserves_disjoint_landed_fields() {
    let _env = crate::ports::test_env_lock();
    let root = tempfile::tempdir().unwrap();
    let state = load(root.path(), 1);
    bootstrap(&state);
    state
        .acquire_debug_lease(DebugLeaseId(10), DebugLeaseOwner::Forensic)
        .unwrap();
    let mut candidate = state.snapshot().config.unwrap();
    candidate.debug_switch = true;
    candidate.settings.auto_hide_vpn_name = true;
    let old = landed_unknown(
        &state,
        candidate.clone(),
        vec!["debugSwitch".into(), "settings/autoHideVpnName".into()],
    );
    state.release_debug_lease(DebugLeaseId(10)).unwrap();
    assert_eq!(state.snapshot().config, Some(candidate.clone()));
    assert_eq!(
        config_file::read(&root.path().join("config.json")).unwrap(),
        Some(candidate)
    );
    assert_eq!(
        state.get_operation_record(old).unwrap().settled,
        Some(PhaseOutcome::Unknown)
    );
    assert!(!state.has_unknown_hold());
}

#[test]
fn expired_unknown_receipts_keep_bounded_holds_and_never_prevent_explicit_repair() {
    let _env = crate::ports::test_env_lock();
    let root = tempfile::tempdir().unwrap();
    config_file::write_atomic(
        &root.path().join("config.json"),
        &CanonicalConfig::default(),
    )
    .unwrap();
    let mut journal = Journal {
        config_revision: 1,
        ..Default::default()
    };
    for op in 1..=100 {
        journal.upsert(uncertain(
            op,
            ConfigPhase::Native,
            CanonicalConfig::default(),
            1,
        ));
    }
    assert!(journal.operations.len() <= 64);
    assert_eq!(journal.expired_holds, vec![ConfigPhase::Native]);
    journal::write_atomic(&root.path().join("operations.json"), &journal).unwrap();
    let state = CoreState::load_with_journal(
        Publisher::new(1, StateSnapshot::default()),
        root.path().join("config.json"),
        Some(root.path().join("operations.json")),
        Some(root.path().join("missing-ctl")),
    )
    .unwrap();
    for _ in 0..70 {
        command(&state, ConfigEdit::RepairActivation);
    }
    assert!(state.has_unknown_hold());
    assert!(state.snapshot().operations.len() <= 64);
    native_none(&state, root.path());
    command(&state, ConfigEdit::RepairActivation);
    assert!(!state.has_unknown_hold());
    assert!(state.snapshot().held_resources.is_empty());
    let disk = journal::read(&root.path().join("operations.json")).unwrap();
    assert!(disk.operations.len() <= 64);
    assert!(disk.expired_holds.is_empty());
    assert_eq!(
        state.submit_command(
            OperationId(1),
            CoreCommand::Edit {
                base: BaseRef { revision: 1 },
                edit: ConfigEdit::RepairActivation
            }
        ),
        Err(SubmitError::Expired)
    );
}

#[test]
fn expired_persist_proof_keeps_actual_revision_and_conflict_evidence() {
    let _env = crate::ports::test_env_lock();
    let root = tempfile::tempdir().unwrap();
    let candidate = CanonicalConfig {
        debug: true,
        ..Default::default()
    };
    config_file::write_atomic(&root.path().join("config.json"), &candidate).unwrap();
    let mut journal = Journal {
        config_revision: 8,
        ..Default::default()
    };
    let mut persist = uncertain(1, ConfigPhase::Persist, candidate.clone(), 8);
    persist.admission = Some(journal::Admission {
        from_revision: 8,
        commit_revision: 9,
        writes: vec!["debug".into()],
    });
    journal.upsert(persist);
    for op in 2..=100 {
        journal.upsert(uncertain(op, ConfigPhase::Native, candidate.clone(), 8));
    }
    assert!(journal.get(OperationId(1)).is_none());
    assert!(journal.pending_persist.is_some());
    journal::write_atomic(&root.path().join("operations.json"), &journal).unwrap();
    let state = CoreState::load_with_journal(
        Publisher::new(1, StateSnapshot::default()),
        root.path().join("config.json"),
        Some(root.path().join("operations.json")),
        Some(root.path().join("missing-ctl")),
    )
    .unwrap();
    assert_eq!(state.config_revision(), 9);
    native_none(&state, root.path());
    command(&state, ConfigEdit::RepairActivation);
    assert_eq!(state.config_revision(), 9);
    assert!(!state.has_unknown_hold());
    let op = state.issue_operation_id().unwrap();
    assert_eq!(
        state.submit_command(
            op,
            CoreCommand::Edit {
                base: BaseRef { revision: 8 },
                edit: ConfigEdit::Toggle {
                    field: ConfigToggle::Debug,
                    enabled: false
                }
            }
        ),
        Err(SubmitError::Conflict {
            field: "debug".into()
        })
    );
}

#[test]
fn pruning_known_failed_persist_never_expires_the_current_revision() {
    let _env = crate::ports::test_env_lock();
    let root = tempfile::tempdir().unwrap();
    config_file::write_atomic(
        &root.path().join("config.json"),
        &CanonicalConfig::default(),
    )
    .unwrap();
    let mut journal = Journal {
        config_revision: 1,
        ..Default::default()
    };
    for op in 1..=100 {
        let mut entry = uncertain(op, ConfigPhase::Persist, CanonicalConfig::default(), 1);
        entry.admission.as_mut().unwrap().commit_revision = 2;
        entry.record.phases[0].outcome = PhaseOutcome::FailedKnown;
        entry.record.settled = Some(PhaseOutcome::FailedKnown);
        journal.upsert(entry);
    }
    assert!(journal.conflict_floor <= journal.config_revision);
    journal::write_atomic(&root.path().join("operations.json"), &journal).unwrap();
    let state = load(root.path(), 1);
    command(
        &state,
        ConfigEdit::Toggle {
            field: ConfigToggle::Debug,
            enabled: true,
        },
    );
    assert_eq!(state.config_revision(), 2);
}

#[test]
fn cleanup_legacy_is_an_explicit_resource_repair() {
    let _env = crate::ports::test_env_lock();
    let root = tempfile::tempdir().unwrap();
    config_file::write_atomic(
        &root.path().join("config.json"),
        &CanonicalConfig::default(),
    )
    .unwrap();
    let original = uncertain(1, ConfigPhase::Cleanup, CanonicalConfig::default(), 1);
    let receipt = original.record.clone();
    let mut journal = Journal {
        config_revision: 1,
        ..Default::default()
    };
    journal.upsert(original);
    journal::write_atomic(&root.path().join("operations.json"), &journal).unwrap();
    let state = load(root.path(), 1);
    std::fs::create_dir(root.path().join("root")).unwrap();
    state.set_effect_paths(Some(crate::effects::EffectPaths {
        root: root.path().join("root"),
    }));
    let op = state.issue_operation_id().unwrap();
    state
        .submit_command(op, CoreCommand::CleanupLegacy)
        .unwrap();
    assert_eq!(state.get_operation_record(OperationId(1)), Some(receipt));
    assert!(!state.has_unknown_hold());
}

#[test]
fn explicit_release_retries_known_failed_restore_with_a_fresh_receipt() {
    let _env = crate::ports::test_env_lock();
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("canonical");
    std::fs::create_dir(&directory).unwrap();
    let state = CoreState::load_with_journal(
        Publisher::new(1, StateSnapshot::default()),
        directory.join("config.json"),
        Some(root.path().join("operations.json")),
        None,
    )
    .unwrap();
    bootstrap(&state);
    state
        .acquire_debug_lease(DebugLeaseId(10), DebugLeaseOwner::Forensic)
        .unwrap();
    let backup = root.path().join("backup");
    std::fs::rename(&directory, &backup).unwrap();
    std::fs::write(&directory, "pre-rename write refusal").unwrap();
    assert!(state.release_debug_lease(DebugLeaseId(10)).is_err());
    let failed = journal::read(&root.path().join("operations.json"))
        .unwrap()
        .debug_restore_op
        .unwrap();
    let receipt = state.get_operation_record(failed).unwrap();
    assert_eq!(receipt.settled, Some(PhaseOutcome::FailedKnown));
    std::fs::remove_file(&directory).unwrap();
    std::fs::rename(&backup, &directory).unwrap();
    state.release_debug_lease(DebugLeaseId(10)).unwrap();
    assert!(!state.snapshot().config.unwrap().debug);
    assert_eq!(state.get_operation_record(failed), Some(receipt));
    assert!(
        journal::read(&root.path().join("operations.json"))
            .unwrap()
            .debug_restore
            .is_none()
    );
}

#[test]
fn superseded_unknown_restore_recomputes_baseline_instead_of_dropping_it() {
    let _env = crate::ports::test_env_lock();
    let root = tempfile::tempdir().unwrap();
    let current = CanonicalConfig {
        debug: true,
        ..Default::default()
    };
    config_file::write_atomic(&root.path().join("config.json"), &current).unwrap();
    let mut entry = uncertain(1, ConfigPhase::Persist, CanonicalConfig::default(), 1);
    entry.superseded_by = Some(OperationId(2));
    entry.superseded_phases.push(ConfigPhase::Persist);
    let original = entry.record.clone();
    let mut journal = Journal {
        config_revision: 1,
        issued_through: 2,
        debug_restore: Some((false, false)),
        debug_restore_op: Some(OperationId(1)),
        ..Default::default()
    };
    journal.upsert(entry);
    journal::write_atomic(&root.path().join("operations.json"), &journal).unwrap();
    let state = load(root.path(), 1);
    state.release_debug_lease(DebugLeaseId(10)).unwrap();
    assert!(!state.snapshot().config.unwrap().debug);
    assert_eq!(state.get_operation_record(OperationId(1)), Some(original));
    assert!(
        journal::read(&root.path().join("operations.json"))
            .unwrap()
            .debug_restore
            .is_none()
    );
}

#[test]
fn persist_crash_restores_actual_admitted_revision_and_overlapping_writes() {
    let _env = crate::ports::test_env_lock();
    let root = tempfile::tempdir().unwrap();
    let candidate = CanonicalConfig {
        debug: true,
        ..Default::default()
    };
    config_file::write_atomic(&root.path().join("config.json"), &candidate).unwrap();
    let mut journal = Journal {
        config_revision: 8,
        issued_through: 12,
        ..Default::default()
    };
    journal.upsert(JournalEntry {
        maintenance: None,
        maintenance_report: None,
        base: BaseRef { revision: 2 },
        edit: ConfigEdit::Toggle {
            field: ConfigToggle::Debug,
            enabled: true,
        },
        admission: Some(journal::Admission {
            from_revision: 8,
            commit_revision: 9,
            writes: vec!["debug".into()],
        }),
        command: Some(journal::CommandIntent::Edit),
        superseded_by: None,
        superseded_phases: Vec::new(),
        finalized_unknown: false,
        candidate: Some(candidate),
        native_intent: None,
        record: OperationRecord {
            op: OperationId(12),
            phases: vec![PhaseRecord {
                phase: ConfigPhase::Persist,
                outcome: PhaseOutcome::Running,
            }],
            settled: None,
            ports: None,
        },
    });
    journal::write_atomic(&root.path().join("operations.json"), &journal).unwrap();
    let state = load(root.path(), 2);
    assert_eq!(state.config_revision(), 9);
    let durable = journal::read(&root.path().join("operations.json")).unwrap();
    assert_eq!(durable.field_revisions.get("debug"), Some(&9));
    let op = state.issue_operation_id().unwrap();
    assert_eq!(
        state.submit_command(
            op,
            CoreCommand::Edit {
                base: BaseRef { revision: 8 },
                edit: ConfigEdit::Toggle {
                    field: ConfigToggle::Debug,
                    enabled: false
                }
            }
        ),
        Err(SubmitError::Conflict {
            field: "debug".into()
        })
    );
}

#[test]
fn terminal_persist_uncertainty_keeps_history_but_readback_restores_commit_identity() {
    let _env = crate::ports::test_env_lock();
    let root = tempfile::tempdir().unwrap();
    let candidate = CanonicalConfig {
        debug: true,
        ..Default::default()
    };
    config_file::write_atomic(&root.path().join("config.json"), &candidate).unwrap();
    let original = OperationRecord {
        op: OperationId(12),
        phases: vec![PhaseRecord {
            phase: ConfigPhase::Persist,
            outcome: PhaseOutcome::Unknown,
        }],
        settled: Some(PhaseOutcome::Unknown),
        ports: None,
    };
    let mut journal = Journal {
        config_revision: 8,
        issued_through: 12,
        ..Default::default()
    };
    journal.upsert(JournalEntry {
        maintenance: None,
        maintenance_report: None,
        base: BaseRef { revision: 2 },
        edit: ConfigEdit::Toggle {
            field: ConfigToggle::Debug,
            enabled: true,
        },
        admission: Some(journal::Admission {
            from_revision: 8,
            commit_revision: 9,
            writes: vec!["debug".into()],
        }),
        command: Some(journal::CommandIntent::Edit),
        superseded_by: None,
        superseded_phases: Vec::new(),
        finalized_unknown: true,
        candidate: Some(candidate),
        native_intent: None,
        record: original.clone(),
    });
    journal::write_atomic(&root.path().join("operations.json"), &journal).unwrap();
    let state = load(root.path(), 2);
    assert_eq!(state.config_revision(), 9);
    assert_eq!(
        state.get_operation_record(OperationId(12)),
        Some(original.clone())
    );
    assert!(state.has_unknown_hold());
    command(&state, ConfigEdit::RepairActivation);
    assert_eq!(state.config_revision(), 9);
    assert_eq!(state.get_operation_record(OperationId(12)), Some(original));
    assert!(!state.has_unknown_hold());
    assert_eq!(
        journal::read(&root.path().join("operations.json"))
            .unwrap()
            .field_revisions
            .get("debug"),
        Some(&9)
    );
}

#[test]
fn receipts_and_conflict_history_are_bounded_and_old_ids_never_mutate_again() {
    let _env = crate::ports::test_env_lock();
    let root = tempfile::tempdir().unwrap();
    let state = load(root.path(), 1);
    bootstrap(&state);
    for index in 0..100 {
        command(
            &state,
            ConfigEdit::Toggle {
                field: ConfigToggle::Debug,
                enabled: index % 2 == 0,
            },
        );
    }
    let revision = state.config_revision();
    let before = state.snapshot().config;
    let disk = journal::read(&root.path().join("operations.json")).unwrap();
    assert!(disk.operations.len() <= journal::DEFAULT_RETAIN_SETTLED);
    assert!(
        disk.retired_ids.is_empty(),
        "no permanent random-id tombstones"
    );
    assert!(disk.field_revisions.len() <= 4);
    assert!(disk.expired_through > 0);
    assert!(state.get_operation_record(OperationId(1)).is_none());
    assert_eq!(
        state.submit_command(
            OperationId(1),
            CoreCommand::Edit {
                base: BaseRef { revision: 0 },
                edit: ConfigEdit::Replace {
                    config: CanonicalConfig::default()
                }
            }
        ),
        Err(SubmitError::Expired)
    );
    assert_eq!(state.snapshot().config, before);
    assert_eq!(state.config_revision(), revision);
    let restarted = load(root.path(), 2);
    assert!(restarted.issue_operation_id().unwrap().0 > disk.issued_through);
}

#[test]
fn unissued_future_identity_is_rejected_without_effects() {
    let state = CoreState::new(Publisher::new(1, StateSnapshot::default()));
    assert!(matches!(
        state.submit_command(
            OperationId(900),
            CoreCommand::Edit {
                base: BaseRef { revision: 0 },
                edit: ConfigEdit::Replace {
                    config: CanonicalConfig::default()
                }
            }
        ),
        Err(SubmitError::Rejected { .. })
    ));
    assert!(state.snapshot().config.is_none());
}

#[test]
fn reader_and_receipts_remain_available_while_the_bounded_writer_is_busy() {
    let _env = crate::ports::test_env_lock();
    let state = CoreState::new(Publisher::new(1, StateSnapshot::default()));
    bootstrap(&state);
    let (started, entered) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    state
        .engine
        .enqueue_test(Box::new(move |_| {
            started.send(()).unwrap();
            wait.recv().unwrap();
        }))
        .unwrap();
    entered
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    for _ in 0..16 {
        state.engine.enqueue_test(Box::new(|_| {})).unwrap();
    }
    assert_eq!(state.call_writer(|_| ()), Err(SubmitError::Busy));
    assert_eq!(
        state.engine.submit(
            OperationId(1),
            CoreCommand::Edit {
                base: BaseRef { revision: 0 },
                edit: ConfigEdit::RepairActivation,
            },
            crate::command_engine::AdmissionTicket::new(
                std::time::Instant::now() + std::time::Duration::from_secs(1)
            ),
            None,
            None,
        ),
        Err(SubmitError::Busy)
    );
    let start = std::time::Instant::now();
    assert_eq!(
        state.get_operation_record(OperationId(1)).unwrap().settled,
        Some(PhaseOutcome::Confirmed)
    );
    assert!(state.snapshot().config.is_some());
    assert!(start.elapsed() < std::time::Duration::from_millis(100));
    release.send(()).unwrap();
}

#[test]
fn accepted_work_survives_loss_of_its_result_waiter() {
    let _env = crate::ports::test_env_lock();
    let state = CoreState::new(Publisher::new(1, StateSnapshot::default()));
    let op = state.issue_operation_id().unwrap();
    let (result, waiter) = std::sync::mpsc::channel();
    state
        .engine
        .enqueue_test(Box::new(move |state| {
            let receipt = state.execute_command(
                op,
                CoreCommand::Edit {
                    base: BaseRef { revision: 0 },
                    edit: ConfigEdit::Replace {
                        config: CanonicalConfig::default(),
                    },
                },
            );
            let _ = result.send(receipt);
        }))
        .unwrap();
    drop(waiter);
    let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while state
        .get_operation_record(op)
        .and_then(|record| record.settled)
        .is_none()
    {
        assert!(std::time::Instant::now() < until);
        std::thread::yield_now();
    }
    assert_eq!(
        state.get_operation_record(op).unwrap().settled,
        Some(PhaseOutcome::Confirmed)
    );
    assert!(state.has_config());
}

#[test]
fn shutdown_closes_admission_drains_accepted_commands_and_joins_writer() {
    let _env = crate::ports::test_env_lock();
    let state = CoreState::new(Publisher::new(1, StateSnapshot::default()));
    let op = state.issue_operation_id().unwrap();
    let (started, entered) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    state
        .engine
        .enqueue_test(Box::new(move |_| {
            started.send(()).unwrap();
            wait.recv().unwrap();
        }))
        .unwrap();
    entered
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    state
        .engine
        .enqueue_test(Box::new(move |state| {
            state
                .execute_command(
                    op,
                    CoreCommand::Edit {
                        base: BaseRef { revision: 0 },
                        edit: ConfigEdit::Replace {
                            config: CanonicalConfig::default(),
                        },
                    },
                )
                .unwrap();
        }))
        .unwrap();
    let stopping = state.clone();
    let shutdown = std::thread::spawn(move || stopping.shutdown());
    let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !state.shutting_down.load(Ordering::SeqCst) {
        assert!(std::time::Instant::now() < until);
        std::thread::yield_now();
    }
    assert_eq!(state.issue_operation_id(), Err(SubmitError::Unavailable));
    release.send(()).unwrap();
    shutdown.join().unwrap();
    assert_eq!(
        state.get_operation_record(op).unwrap().settled,
        Some(PhaseOutcome::Confirmed)
    );
    assert!(state.engine.is_joined());
    state.shutdown();
}

#[test]
fn singleton_guard_outlives_the_callers_last_reference_while_writer_runs() {
    let _env = crate::ports::test_env_lock();
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config.json");
    let journal = root.path().join("operations.json");
    let guard = crate::instance::InstanceGuard::state(&config, &journal).unwrap();
    let state = load(root.path(), 1);
    state.own_instance_guard(guard).unwrap();
    let (started, entered) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    state
        .engine
        .enqueue_test(Box::new(move |_| {
            started.send(()).unwrap();
            wait.recv().unwrap();
        }))
        .unwrap();
    entered
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    drop(state);
    assert!(crate::instance::InstanceGuard::state(&config, &journal).is_err());
    release.send(()).unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if let Ok(guard) = crate::instance::InstanceGuard::state(&config, &journal) {
            drop(guard);
            break;
        }
        assert!(std::time::Instant::now() < until);
        std::thread::yield_now();
    }
}

#[test]
fn held_resources_remain_independent_after_partial_repair() {
    let mut entry = JournalEntry {
        maintenance: None,
        maintenance_report: None,
        admission: None,
        command: None,
        superseded_by: None,
        superseded_phases: vec![ConfigPhase::Native],
        finalized_unknown: false,
        base: BaseRef { revision: 0 },
        edit: ConfigEdit::RepairActivation,
        candidate: Some(CanonicalConfig::default()),
        native_intent: None,
        record: OperationRecord {
            op: OperationId(1),
            phases: vec![
                PhaseRecord {
                    phase: ConfigPhase::Native,
                    outcome: PhaseOutcome::Unknown,
                },
                PhaseRecord {
                    phase: ConfigPhase::Ports,
                    outcome: PhaseOutcome::Unknown,
                },
            ],
            settled: Some(PhaseOutcome::Unknown),
            ports: None,
        },
    };
    assert_eq!(entry.held_phases(), vec![ConfigPhase::Ports]);
    let original = entry.record.clone();
    entry.superseded_phases.push(ConfigPhase::Ports);
    assert!(entry.held_phases().is_empty());
    assert_eq!(entry.record, original);
}

#[test]
fn composed_secret_receipts_are_durable_deduplicated_and_nonsecret() {
    let _env = crate::ports::test_env_lock();
    let root = tempfile::tempdir().unwrap();
    let state = load(root.path(), 1);
    state.set_effect_paths(Some(crate::effects::EffectPaths {
        root: root.path().join("root"),
    }));
    bootstrap(&state);
    let op = state.issue_operation_id().unwrap();
    let base = BaseRef {
        revision: state.config_revision(),
    };
    let request = CoreCommand::SetSuperkey {
        base,
        remember: true,
        superkey: "only-in-the-secret-file".into(),
    };
    state.submit_command(op, request.clone()).unwrap();
    let path = root.path().join("root/data/adb/vpnhide/superkey");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "only-in-the-secret-file\n"
    );
    assert!(state.snapshot().config.unwrap().settings.remember_superkey);
    let raw = std::fs::read_to_string(root.path().join("operations.json")).unwrap();
    assert!(!raw.contains("only-in-the-secret-file"));
    let receipt = state.get_operation_record(op).unwrap();
    assert!(receipt.phases.iter().any(
        |phase| phase.phase == ConfigPhase::Secret && phase.outcome == PhaseOutcome::Confirmed
    ));
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        state.submit_command(op, request),
        Err(SubmitError::IdReused)
    );
    assert!(
        !path.exists(),
        "same identity queries; it never replays Secret"
    );
    let restarted = load(root.path(), 2);
    assert_eq!(restarted.get_operation_record(op), Some(receipt));
    vpnhide_activator::set_session_superkey(None);
}

#[test]
fn generic_leases_compose_and_block_reset_until_all_owners_release() {
    let _env = crate::ports::test_env_lock();
    let root = tempfile::tempdir().unwrap();
    let state = load(root.path(), 1);
    state.set_effect_paths(Some(crate::effects::EffectPaths {
        root: root.path().join("root"),
    }));
    bootstrap(&state);
    state
        .acquire_debug_lease(DebugLeaseId(10), DebugLeaseOwner::Forensic)
        .unwrap();
    state
        .acquire_debug_lease(DebugLeaseId(20), DebugLeaseOwner::Logcat)
        .unwrap();
    assert!(state.snapshot().config.unwrap().debug);
    let reset = state.issue_operation_id().unwrap();
    assert_eq!(
        state.submit_command(reset, CoreCommand::FullReset),
        Err(SubmitError::Busy)
    );
    state.release_debug_lease(DebugLeaseId(10)).unwrap();
    assert!(state.snapshot().config.unwrap().debug);
    state.release_debug_lease(DebugLeaseId(20)).unwrap();
    assert!(!state.snapshot().config.unwrap().debug);
    assert!(state.snapshot().debug_leases.is_empty());
    assert!(
        journal::read(&root.path().join("operations.json"))
            .unwrap()
            .debug_restore
            .is_none()
    );
}

#[test]
fn lease_expiry_restores_baseline_without_a_recorder_session() {
    let _env = crate::ports::test_env_lock();
    let root = tempfile::tempdir().unwrap();
    let state = load(root.path(), 1);
    bootstrap(&state);
    state
        .acquire_debug_lease(DebugLeaseId(10), DebugLeaseOwner::Forensic)
        .unwrap();
    state
        .call_writer(|state| {
            let mut store = state.engine.store.lock().unwrap().clone();
            store.journal.debug_leases[0].expires_at_ms = 0;
            state.persist_store(&store).unwrap();
            state.expire_debug_leases();
            assert!(
                state.debug_lease_held(),
                "wall-clock expiry must not expire a live lease"
            );
            state
                .live_leases
                .lock()
                .unwrap()
                .get_mut(&DebugLeaseId(10))
                .unwrap()
                .deadline = crate::boot_time::BootInstant::at(0)
                .after(std::time::Duration::ZERO)
                .unwrap();
            state.expire_debug_leases();
        })
        .unwrap();
    assert!(!state.snapshot().config.unwrap().debug);
    assert!(!state.debug_lease_held());
}

#[test]
fn both_logging_lease_owners_count_suspend_and_deny_unknown_or_regressed_clocks() {
    use crate::boot_time::BootInstant;
    for owner in [DebugLeaseOwner::Forensic, DebugLeaseOwner::Logcat] {
        let seconds = lease_duration(owner).as_secs();
        let lease = LiveLease {
            deadline: BootInstant::at(100).after(lease_duration(owner)).unwrap(),
            active: true,
        };
        assert!(lease.live_at(Some(BootInstant::at(100 + seconds - 1))));
        assert!(!lease.live_at(Some(BootInstant::at(100 + seconds))));
        // A long suspend advances BOOTTIME even without application activity.
        assert!(!lease.live_at(Some(BootInstant::at(100 + seconds + 86_400))));
        assert!(!lease.live_at(Some(BootInstant::at(99))));
        assert!(!lease.live_at(None));
    }
}

#[test]
fn logging_authority_expiry_restores_both_owners_through_the_durable_writer() {
    use crate::boot_time::BootInstant;
    let _env = crate::ports::test_env_lock();
    for now in [
        Some(BootInstant::at(100_000)),
        Some(BootInstant::at(99)),
        None,
    ] {
        let root = tempfile::tempdir().unwrap();
        let state = load(root.path(), 1);
        bootstrap(&state);
        for (id, owner) in [
            (DebugLeaseId(10), DebugLeaseOwner::Forensic),
            (DebugLeaseId(20), DebugLeaseOwner::Logcat),
        ] {
            state.acquire_debug_lease(id, owner).unwrap();
        }
        state
            .call_writer(move |state| {
                for lease in state.live_leases.lock().unwrap().values_mut() {
                    lease.deadline = BootInstant::at(100)
                        .after(std::time::Duration::from_secs(1))
                        .unwrap();
                }
                let mut store = state.engine.store.lock().unwrap().clone();
                for lease in &mut store.journal.debug_leases {
                    // Wall-clock metadata cannot extend runtime authority.
                    lease.expires_at_ms = u64::MAX;
                }
                state.persist_store(&store).unwrap();
                state.expire_debug_leases_at(now);
            })
            .unwrap();
        assert!(!state.debug_lease_held());
        assert!(!state.snapshot().config.unwrap().debug);
        let journal = journal::read(&root.path().join("operations.json")).unwrap();
        assert!(journal.debug_leases.is_empty());
        assert!(journal.debug_restore.is_none());
    }
}

#[test]
fn explicit_repair_releases_only_covered_resources_and_preserves_old_unknown_receipt() {
    use vpnhide_core_rpc::{NativeBootOutcome, NativeBootStatus, ZygiskTargets};
    let _env = crate::ports::test_env_lock();
    let root = tempfile::tempdir().unwrap();
    let candidate = CanonicalConfig::default();
    config_file::write_atomic(&root.path().join("config.json"), &candidate).unwrap();
    let unknown = OperationRecord {
        op: OperationId(1),
        phases: vec![PhaseRecord {
            phase: ConfigPhase::Native,
            outcome: PhaseOutcome::Unknown,
        }],
        settled: Some(PhaseOutcome::Unknown),
        ports: None,
    };
    let mut journal = Journal {
        config_revision: 1,
        issued_through: 1,
        ..Default::default()
    };
    journal.upsert(JournalEntry {
        maintenance: None,
        maintenance_report: None,
        base: BaseRef { revision: 1 },
        edit: ConfigEdit::RepairActivation,
        admission: Some(journal::Admission {
            from_revision: 1,
            commit_revision: 1,
            writes: Vec::new(),
        }),
        command: Some(journal::CommandIntent::Edit),
        superseded_by: None,
        superseded_phases: Vec::new(),
        finalized_unknown: false,
        candidate: Some(candidate),
        native_intent: None,
        record: unknown.clone(),
    });
    journal::write_atomic(&root.path().join("operations.json"), &journal).unwrap();
    let state = CoreState::load_with_journal(
        Publisher::new(2, StateSnapshot::default()),
        root.path().join("config.json"),
        Some(root.path().join("operations.json")),
        Some(root.path().join("missing-ctl")),
    )
    .unwrap();
    let env = crate::native_boot::NativeEnv {
        ctl: root.path().join("missing-ctl"),
        modules: None,
        kpm_lock: None,
        zygisk_module: None,
        boot_id_path: root.path().join("boot-id"),
        state_file: root.path().join("native.json"),
        legacy_root: None,
        apatch_kpms: root.path().join("apatch"),
        payload_override: None,
        bundle_identity: None,
        loader: None,
    };
    state.install_native_boot(crate::native_boot::NativeBoot {
        env,
        selection: None,
        status: NativeBootStatus {
            boot_id: "test".into(),
            decided_epoch: 2,
            decided_at_ms: 0,
            updated_at_ms: 0,
            outcome: NativeBootOutcome::NoBackend,
            installed: Vec::new(),
            live: Vec::new(),
            staged: Vec::new(),
            kmod: None,
            zygisk_targets: ZygiskTargets::NotInstalled,
            ports: None,
            obsolete_modules: Vec::new(),
        },
    });
    let blocked = state.issue_operation_id().unwrap();
    assert_eq!(
        state.submit_command(
            blocked,
            CoreCommand::Edit {
                base: BaseRef { revision: 1 },
                edit: ConfigEdit::Toggle {
                    field: ConfigToggle::Debug,
                    enabled: true
                }
            }
        ),
        Err(SubmitError::Unavailable)
    );
    let repair = command(&state, ConfigEdit::RepairActivation);
    assert_eq!(
        state.get_operation_record(OperationId(1)),
        Some(unknown.clone())
    );
    assert!(!state.has_unknown_hold());
    assert!(state.snapshot().held_resources.is_empty());
    let disk = journal::read(&root.path().join("operations.json")).unwrap();
    assert_eq!(
        disk.get(OperationId(1)).unwrap().superseded_by,
        Some(repair)
    );
    let restarted = load(root.path(), 3);
    assert_eq!(
        restarted.get_operation_record(OperationId(1)),
        Some(unknown)
    );
    assert!(!restarted.has_unknown_hold());
}

#[test]
fn import_retains_sources_on_persist_failure_and_duplicate_never_replays_cleanup() {
    let _env = crate::ports::test_env_lock();
    let root = tempfile::tempdir().unwrap();
    let state = load(root.path(), 1);
    let effects = root.path().join("root");
    state.set_effect_paths(Some(crate::effects::EffectPaths {
        root: effects.clone(),
    }));
    bootstrap(&state);
    let source = effects.join("data/adb/vpnhide_kmod/targets.txt");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::write(&source, "com.bank\n").unwrap();
    let mut config = CanonicalConfig::default();
    config.apps.insert(
        "com.bank".into(),
        vpnhide_core_rpc::AppConfig {
            java: true,
            native: vpnhide_core_rpc::NativeRole {
                enabled: true,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let request = CoreCommand::Import {
        base: BaseRef {
            revision: state.config_revision(),
        },
        config: config.clone(),
        cleanup_legacy: true,
    };
    let failed_id = state.issue_operation_id().unwrap();
    std::fs::create_dir(root.path().join("config.json.tmp")).unwrap();
    state.submit_command(failed_id, request).unwrap();
    assert_eq!(std::fs::read_to_string(&source).unwrap(), "com.bank\n");
    assert!(
        !state
            .snapshot()
            .config
            .unwrap()
            .apps
            .contains_key("com.bank")
    );
    std::fs::remove_dir(root.path().join("config.json.tmp")).unwrap();
    let id = state.issue_operation_id().unwrap();
    let request = CoreCommand::Import {
        base: BaseRef {
            revision: state.config_revision(),
        },
        config,
        cleanup_legacy: true,
    };
    state.submit_command(id, request.clone()).unwrap();
    assert!(!source.exists());
    assert!(
        state
            .snapshot()
            .config
            .unwrap()
            .apps
            .contains_key("com.bank")
    );
    drop(state);
    std::fs::write(&source, "new user source\n").unwrap();
    let restarted = load(root.path(), 2);
    restarted.set_effect_paths(Some(crate::effects::EffectPaths { root: effects }));
    restarted.submit_command(id, request).unwrap();
    assert_eq!(
        std::fs::read_to_string(source).unwrap(),
        "new user source\n"
    );
}

#[test]
fn boot_repairs_only_independent_resources_and_preserves_unknown_receipts() {
    let phases = [
        ConfigPhase::Persist,
        ConfigPhase::Secret,
        ConfigPhase::Cleanup,
        ConfigPhase::Native,
        ConfigPhase::Ports,
    ];
    for expired in [false, true] {
        for bits in 0..32 {
            for (kind, resource) in [
                (operation::PlanKind::NativeOnly, ConfigPhase::Native),
                (operation::PlanKind::PortsOnly, ConfigPhase::Ports),
            ] {
                let root = tempfile::tempdir().unwrap();
                let config = CanonicalConfig::default();
                crate::config_file::write_atomic(&root.path().join("config.json"), &config)
                    .unwrap();
                let held: Vec<_> = phases
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| bits & (1 << i) != 0)
                    .map(|(_, phase)| *phase)
                    .collect();
                let mut journal = Journal {
                    config_revision: 1,
                    ..Default::default()
                };
                for (i, phase) in held.iter().enumerate() {
                    journal.upsert(uncertain(i as u64 + 1, *phase, config.clone(), 1));
                }
                if expired {
                    journal.operations.clear();
                    journal.expired_holds = held.clone();
                }
                journal::write_atomic(&root.path().join("operations.json"), &journal).unwrap();
                let state = load(root.path(), 1);
                let before = state.snapshot().operations;
                let report = crate::boot::reapply_with(
                    &state,
                    SubmitOptions {
                        plan: Some(kind),
                        ..Default::default()
                    },
                );
                // Independent oracle, including Native's credential dependency.
                let allowed = bits & 1 == 0
                    && !held.contains(&resource)
                    && !(resource == ConfigPhase::Native && bits & 2 != 0);
                assert_eq!(
                    report.op.is_some(),
                    allowed,
                    "bits={bits}, expired={expired}, {kind:?}: {report:?}"
                );
                if !allowed {
                    assert_eq!(report.status, crate::boot::BootApplyStatus::HeldUnknown);
                }
                // Host runtime writers are disabled: this checks real admission,
                // journal and restart, not an actual firewall/backend delivery.
                for old in before {
                    assert_eq!(state.get_operation_record(old.op), Some(old));
                }
                let saved = journal::read(&root.path().join("operations.json")).unwrap();
                for old in &journal.operations {
                    assert_eq!(saved.get(old.record.op).unwrap().record, old.record);
                    assert!(
                        saved
                            .get(old.record.op)
                            .unwrap()
                            .superseded_phases
                            .is_empty()
                    );
                }
                assert_eq!(saved.expired_holds, journal.expired_holds);
                drop(state);
                let restarted = load(root.path(), 2);
                for old in journal.operations {
                    assert_eq!(
                        restarted.get_operation_record(old.record.op),
                        Some(old.record)
                    );
                }
            }
        }
    }
}

#[test]
fn journal_faults_never_publish_a_nondurable_terminal_receipt() {
    for after_rename in [false, true] {
        for write in 1..=4 {
            let root = tempfile::tempdir().unwrap();
            let state = load(root.path(), 1);
            bootstrap(&state);
            let op = state.issue_operation_id().unwrap();
            let result = state
                .call_writer(move |state| {
                    journal::fail_write(write, after_rename);
                    state.execute_command(
                        op,
                        CoreCommand::Edit {
                            base: BaseRef {
                                revision: state.config_revision(),
                            },
                            edit: ConfigEdit::Toggle {
                                field: ConfigToggle::Debug,
                                enabled: true,
                            },
                        },
                    )
                })
                .unwrap();
            let observed = state.get_operation_record(op).unwrap();
            if write == 3 {
                assert!(result.is_err());
                assert_eq!(
                    observed.settled, None,
                    "settlement sync failure is not terminal Unknown"
                );
            } else {
                assert!(result.is_ok(), "write={write}, after={after_rename}");
                assert!(observed.settled.is_some());
            }
            assert_eq!(state.snapshot().storage_fault.is_some(), write >= 3);
            if write >= 3 {
                assert!(state.snapshot().mutation_held);
                assert!(state.snapshot().operations_in_flight.is_empty());
                assert_eq!(state.issue_operation_id(), Err(SubmitError::Unavailable));
                assert_eq!(state.get_operation_record(op), Some(observed.clone()));
            }
            drop(state);
            let restarted = load(root.path(), 2);
            let reconciled = restarted.get_operation_record(op).unwrap();
            if observed.settled.is_some() {
                assert_eq!(
                    observed, reconciled,
                    "durable terminal receipt changed on restart"
                );
            } else {
                assert_eq!(reconciled.settled, Some(PhaseOutcome::Confirmed));
            }
            assert!(restarted.snapshot().storage_fault.is_none());
        }
    }
}

#[test]
fn failed_lease_acquire_cannot_be_renewed_as_active_without_logging() {
    let root = tempfile::tempdir().unwrap();
    let state = load(root.path(), 1);
    bootstrap(&state);
    let blocked = root.path().join("config.json.tmp");
    std::fs::create_dir(&blocked).unwrap();
    let id = DebugLeaseId(77);
    for _ in 0..2 {
        assert_eq!(
            state.acquire_debug_lease(id, DebugLeaseOwner::Forensic),
            Err(SubmitError::Unavailable)
        );
        assert!(!state.current_debug().unwrap().0);
        assert!(!state.live_leases.lock().unwrap().get(&id).unwrap().active);
        assert_eq!(
            state.engine.store.lock().unwrap().journal.debug_restore,
            Some((false, false))
        );
    }
    std::fs::remove_dir(blocked).unwrap();
    state
        .acquire_debug_lease(id, DebugLeaseOwner::Forensic)
        .unwrap();
    assert!(state.current_debug().unwrap().0);
    assert_eq!(
        state.acquire_debug_lease(id, DebugLeaseOwner::Logcat),
        Err(SubmitError::IdReused)
    );
    state.release_debug_lease(id).unwrap();
    assert!(!state.current_debug().unwrap().0);
}

#[test]
fn restart_retires_both_lease_owners_and_restores_the_original_baseline() {
    let root = tempfile::tempdir().unwrap();
    let state = load(root.path(), 1);
    bootstrap(&state);
    state
        .acquire_debug_lease(DebugLeaseId(1), DebugLeaseOwner::Forensic)
        .unwrap();
    state
        .acquire_debug_lease(DebugLeaseId(2), DebugLeaseOwner::Logcat)
        .unwrap();
    drop(state);
    let restarted = load(root.path(), 2);
    assert!(!restarted.debug_lease_held());
    restarted
        .call_writer(|state| state.expire_debug_leases())
        .unwrap();
    assert!(!restarted.current_debug().unwrap().0);
    assert!(
        restarted
            .engine
            .store
            .lock()
            .unwrap()
            .journal
            .debug_restore
            .is_none()
    );
}

fn startup_env(root: &std::path::Path) -> crate::native_boot::NativeEnv {
    std::fs::write(root.join("boot-id"), "test-boot").unwrap();
    crate::native_boot::NativeEnv {
        ctl: root.join("ctl"),
        modules: None,
        kpm_lock: None,
        zygisk_module: None,
        boot_id_path: root.join("boot-id"),
        state_file: root.join("native.json"),
        legacy_root: None,
        apatch_kpms: root.join("no-apatch"),
        payload_override: None,
        bundle_identity: None,
        loader: None,
    }
}

#[test]
fn native_startup_is_durable_and_not_repeated_on_worker_restart() {
    let root = tempfile::tempdir().unwrap();
    let env = startup_env(root.path());
    let state = load(root.path(), 1);
    state.start_native_boot(env.clone(), 1);
    assert!(matches!(
        state.native_boot_status().unwrap().outcome,
        vpnhide_core_rpc::NativeBootOutcome::NoBackend
    ));
    let saved = journal::read(&root.path().join("operations.json")).unwrap();
    assert!(saved.native_startup.unwrap().completed.is_some());
    state.shutdown();
    // A corrupt best-effort file cannot redirect a durable same-boot decision.
    std::fs::write(&env.state_file, "corrupt").unwrap();
    let state = load(root.path(), 2);
    state.start_native_boot(env.clone(), 2);
    assert!(matches!(
        state.native_boot_status().unwrap().outcome,
        vpnhide_core_rpc::NativeBootOutcome::NoBackend
    ));
    assert_eq!(std::fs::read_to_string(env.state_file).unwrap(), "corrupt");
    state.shutdown();
}

#[test]
fn native_startup_crash_intent_blocks_replay_even_after_reboot() {
    for previous_boot in ["test-boot", "previous-boot"] {
        let root = tempfile::tempdir().unwrap();
        let env = startup_env(root.path());
        let j = Journal {
            native_startup: Some(journal::NativeStartup {
                intent: None,
                repair_op: None,
                boot_id: previous_boot.into(),
                config_revision: 0,
                filesystem_hiding: false,
                completed: None,
            }),
            ..Journal::default()
        };
        journal::write_atomic(&root.path().join("operations.json"), &j).unwrap();
        let state = load(root.path(), 1);
        state.start_native_boot(env.clone(), 1);
        assert!(matches!(
            state.native_boot_status().unwrap().outcome,
            vpnhide_core_rpc::NativeBootOutcome::Unknown { .. }
        ));
        assert!(
            state
                .snapshot()
                .held_resources
                .contains(&ConfigPhase::Native)
        );
        assert!(!env.state_file.exists(), "no scan/effect/persist replay");
        assert!(
            journal::read(&root.path().join("operations.json"))
                .unwrap()
                .native_startup_uncertain()
        );
        state.shutdown();
    }
}

#[test]
fn native_startup_intent_storage_failures_never_dispatch() {
    for after_rename in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let env = startup_env(root.path());
        let state = load(root.path(), 1);
        let worker_env = env.clone();
        state
            .call_writer(move |state| {
                journal::fail_write(1, after_rename);
                state.start_native_boot_direct(worker_env, 1);
            })
            .unwrap();
        assert!(state.snapshot().storage_fault.is_some());
        assert!(!env.state_file.exists());
        assert!(matches!(
            state.native_boot_status().unwrap().outcome,
            vpnhide_core_rpc::NativeBootOutcome::Unknown { .. }
        ));
        state.shutdown();
    }
}

#[test]
fn native_startup_completion_storage_failure_preserves_uncertainty() {
    for after_rename in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let env = startup_env(root.path());
        let state = load(root.path(), 1);
        state
            .call_writer(move |state| {
                journal::fail_write(2, after_rename);
                state.start_native_boot_direct(env, 1);
            })
            .unwrap();
        assert!(state.snapshot().storage_fault.is_some());
        assert!(
            state
                .snapshot()
                .held_resources
                .contains(&ConfigPhase::Native)
        );
        let disk = journal::read(&root.path().join("operations.json")).unwrap();
        assert_eq!(disk.native_startup_uncertain(), !after_rename);
        state.shutdown();
    }
}

#[test]
fn native_startup_respects_existing_persist_native_and_secret_holds() {
    for phase in [
        ConfigPhase::Persist,
        ConfigPhase::Native,
        ConfigPhase::Secret,
    ] {
        let root = tempfile::tempdir().unwrap();
        let env = startup_env(root.path());
        let j = Journal {
            expired_holds: vec![phase],
            ..Journal::default()
        };
        journal::write_atomic(&root.path().join("operations.json"), &j).unwrap();
        let state = load(root.path(), 1);
        state.start_native_boot(env.clone(), 1);
        assert!(!env.state_file.exists());
        assert!(
            journal::read(&root.path().join("operations.json"))
                .unwrap()
                .native_startup
                .is_none()
        );
        state.shutdown();
    }
}

#[test]
fn native_startup_without_durable_journal_or_boot_identity_refuses_effects() {
    let root = tempfile::tempdir().unwrap();
    let mut env = startup_env(root.path());
    let state = CoreState::load(
        Publisher::new(1, StateSnapshot::default()),
        root.path().join("config.json"),
        None,
    )
    .unwrap();
    state.start_native_boot(env.clone(), 1);
    assert!(!env.state_file.exists());
    state.shutdown();
    let state = load(root.path(), 2);
    env.boot_id_path = root.path().join("missing-boot-id");
    state.start_native_boot(env.clone(), 2);
    assert!(!env.state_file.exists());
    state.shutdown();
}

#[test]
fn durable_native_selection_is_rechecked_against_live_identity() {
    let root = tempfile::tempdir().unwrap();
    let env = startup_env(root.path());
    let mut boot = crate::native_boot::NativeBoot::blocked(env.clone(), 1, "fixture".into());
    let selection = crate::native_boot::Selection {
        backend: vpnhide_core_rpc::NativeBackend::Builtin,
        mode: vpnhide_core_rpc::SelectionMode::Adopted,
        filesystem_hiding: None,
    };
    boot.status.outcome = vpnhide_core_rpc::NativeBootOutcome::Selected {
        backend: selection.backend,
        mode: selection.mode,
        filesystem_hiding: None,
    };
    let j = Journal {
        native_startup: Some(journal::NativeStartup {
            intent: None,
            repair_op: None,
            boot_id: "test-boot".into(),
            config_revision: 0,
            filesystem_hiding: false,
            completed: Some(journal::NativeStartupResult {
                status: boot.status,
                selection: Some(selection),
                rejection: None,
            }),
        }),
        ..Journal::default()
    };
    journal::write_atomic(&root.path().join("operations.json"), &j).unwrap();
    std::fs::write(
        &env.ctl,
        "vpnhide 1 status\nbackend 0x4\nkver 0x0\nhooks 0x0\nerror 0x0\n",
    )
    .unwrap();
    let state = load(root.path(), 2);
    state.start_native_boot(env.clone(), 2);
    assert!(matches!(
        state.native_boot_status().unwrap().outcome,
        vpnhide_core_rpc::NativeBootOutcome::Selected { .. }
    ));
    state.shutdown();
    std::fs::remove_file(&env.ctl).unwrap();
    let state = load(root.path(), 3);
    state.start_native_boot(env.clone(), 3);
    assert!(matches!(
        state.native_boot_status().unwrap().outcome,
        vpnhide_core_rpc::NativeBootOutcome::Unknown { .. }
    ));
    assert!(!env.state_file.exists());
    state.shutdown();
}

#[test]
fn startup_hold_survives_receipt_retention_and_schema_two_upgrade() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("operations.json");
    let old = Journal {
        version: 2,
        issued_through: 42,
        ..Journal::default()
    };
    let mut json = serde_json::to_value(old).unwrap();
    json.as_object_mut().unwrap().remove("native_startup");
    std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    let mut upgraded = journal::read(&path).unwrap();
    assert_eq!(upgraded.version, vpnhide_build_contract::JOURNAL_VERSION);
    assert_eq!(upgraded.issued_through, 42);
    assert!(upgraded.native_startup.is_none());
    upgraded.native_startup = Some(journal::NativeStartup {
        intent: None,
        repair_op: None,
        boot_id: "old-boot".into(),
        config_revision: 0,
        filesystem_hiding: false,
        completed: None,
    });
    upgraded.retain_settled = 0;
    upgraded.prune();
    journal::write_atomic(&path, &upgraded).unwrap();
    assert_eq!(
        journal::read(&path).unwrap().held_resources(),
        vec![ConfigPhase::Native]
    );
    upgraded.version = vpnhide_build_contract::JOURNAL_VERSION + 1;
    journal::write_atomic(&path, &upgraded).unwrap();
    assert!(journal::read(&path).is_err());
}

#[test]
fn explicit_startup_repair_after_reboot_preserves_unknown_history_and_current_config() {
    let root = tempfile::tempdir().unwrap();
    let env = startup_env(root.path());
    let config = CanonicalConfig {
        debug: true,
        ..CanonicalConfig::default()
    };
    config_file::write_atomic(&root.path().join("config.json"), &config).unwrap();
    let j = Journal {
        native_startup: Some(journal::NativeStartup {
            boot_id: "previous-boot".into(),
            config_revision: 1,
            filesystem_hiding: false,
            intent: None,
            repair_op: None,
            completed: None,
        }),
        ..Journal::default()
    };
    journal::write_atomic(&root.path().join("operations.json"), &j).unwrap();
    let state = CoreState::load_with_journal(
        Publisher::new(2, StateSnapshot::default()),
        root.path().join("config.json"),
        Some(root.path().join("operations.json")),
        Some(env.ctl.clone()),
    )
    .unwrap();
    state.start_native_boot(env, 2);
    assert!(state.has_unknown_hold());
    let op = command(&state, ConfigEdit::RepairActivation);
    assert_eq!(
        state.get_operation_record(op).unwrap().settled,
        Some(PhaseOutcome::Confirmed)
    );
    assert!(!state.has_unknown_hold());
    assert_eq!(
        config_file::read(&root.path().join("config.json")).unwrap(),
        Some(config)
    );
    let disk = journal::read(&root.path().join("operations.json")).unwrap();
    assert_eq!(disk.native_attempts.len(), 1);
    assert!(
        disk.native_attempts[0].attempt.completed.is_none(),
        "old uncertainty is not rewritten as success"
    );
    assert_eq!(disk.native_attempts[0].superseded_by, Some(op));
    assert_eq!(disk.native_startup.unwrap().repair_op, Some(op));
    state.shutdown();
}

#[test]
fn interrupted_startup_without_config_allows_explicit_import_then_repair() {
    let root = tempfile::tempdir().unwrap();
    let env = startup_env(root.path());
    let j = Journal {
        native_startup: Some(journal::NativeStartup {
            boot_id: "previous-boot".into(),
            config_revision: 0,
            filesystem_hiding: false,
            intent: None,
            repair_op: None,
            completed: None,
        }),
        ..Journal::default()
    };
    journal::write_atomic(&root.path().join("operations.json"), &j).unwrap();
    let state = CoreState::load_with_journal(
        Publisher::new(2, StateSnapshot::default()),
        root.path().join("config.json"),
        Some(root.path().join("operations.json")),
        Some(env.ctl.clone()),
    )
    .unwrap();
    state.start_native_boot(env, 2);
    let config = CanonicalConfig {
        debug_switch: true,
        ..CanonicalConfig::default()
    };
    let import = command(
        &state,
        ConfigEdit::Replace {
            config: config.clone(),
        },
    );
    assert_eq!(
        state.get_operation_record(import).unwrap().phases[0].outcome,
        PhaseOutcome::Confirmed
    );
    assert!(
        state.has_unknown_hold(),
        "import does not invent startup completion"
    );
    let repair = command(&state, ConfigEdit::RepairActivation);
    assert_eq!(
        state.get_operation_record(repair).unwrap().settled,
        Some(PhaseOutcome::Confirmed)
    );
    assert!(!state.has_unknown_hold());
    assert_eq!(
        config_file::read(&root.path().join("config.json")).unwrap(),
        Some(config)
    );
    state.shutdown();
}

#[test]
fn core_migration_derives_intent_from_sources_and_retained_retry_never_replays() {
    for mode in [
        vpnhide_core_rpc::LegacyImportMode::Merge,
        vpnhide_core_rpc::LegacyImportMode::Replace,
        vpnhide_core_rpc::LegacyImportMode::Bootstrap,
    ] {
        let root = tempfile::tempdir().unwrap();
        let state = load(root.path(), 1);
        bootstrap(&state);
        let effects = root.path().join("root");
        let source = effects.join("data/adb/vpnhide_kmod/targets.txt");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "com.bank.app\n").unwrap();
        state.set_effect_paths(Some(crate::effects::EffectPaths {
            root: effects.clone(),
        }));
        let op = state.issue_operation_id().unwrap();
        let request = CoreCommand::MigrateLegacy {
            base: BaseRef {
                revision: state.config_revision(),
            },
            mode,
        };
        state.submit_command(op, request.clone()).unwrap();
        assert_eq!(
            state.get_operation_record(op).unwrap().settled,
            Some(PhaseOutcome::Confirmed)
        );
        assert!(
            state.snapshot().config.unwrap().apps["com.bank.app"]
                .native
                .enabled
        );
        assert!(!source.exists());
        let journal = journal::read(&root.path().join("operations.json")).unwrap();
        assert!(journal.get(op).unwrap().maintenance.is_some());
        assert!(
            journal
                .get(op)
                .unwrap()
                .maintenance_report
                .as_ref()
                .unwrap()
                .confirmed()
        );
        command(
            &state,
            ConfigEdit::Toggle {
                field: ConfigToggle::DebugSwitch,
                enabled: true,
            },
        );
        std::fs::write(&source, "com.new.user\n").unwrap();
        assert_eq!(
            state.submit_command(op, request.clone()),
            Ok(Accepted { op })
        );
        assert_eq!(std::fs::read_to_string(&source).unwrap(), "com.new.user\n");
        drop(state);
        let restarted = load(root.path(), 2);
        // Retained command recognition does not depend on re-reading sources or
        // on the privileged effects adapter being enabled in this new instance.
        assert_eq!(restarted.submit_command(op, request), Ok(Accepted { op }));
        assert_eq!(std::fs::read_to_string(source).unwrap(), "com.new.user\n");
    }
}

#[test]
fn empty_or_incomplete_replace_does_not_erase_configured_choices() {
    for contents in ["", "com.imported.app\nmalformed\n"] {
        let root = tempfile::tempdir().unwrap();
        let state = load(root.path(), 1);
        bootstrap(&state);
        let mut config = CanonicalConfig::default();
        config.apps.insert(
            "com.existing.app".into(),
            vpnhide_core_rpc::AppConfig {
                java: true,
                ..Default::default()
            },
        );
        command(
            &state,
            ConfigEdit::Replace {
                config: config.clone(),
            },
        );
        let effects = root.path().join("root");
        std::fs::create_dir_all(&effects).unwrap();
        let source = effects.join("data/adb/vpnhide_kmod/targets.txt");
        if !contents.is_empty() {
            std::fs::create_dir_all(source.parent().unwrap()).unwrap();
            std::fs::write(&source, contents).unwrap();
        }
        state.set_effect_paths(Some(crate::effects::EffectPaths { root: effects }));
        let op = state.issue_operation_id().unwrap();
        assert!(matches!(
            state.submit_command(
                op,
                CoreCommand::MigrateLegacy {
                    base: BaseRef {
                        revision: state.config_revision()
                    },
                    mode: vpnhide_core_rpc::LegacyImportMode::Replace
                }
            ),
            Err(SubmitError::Rejected { .. })
        ));
        assert_eq!(state.snapshot().config, Some(config));
        if !contents.is_empty() {
            assert_eq!(std::fs::read_to_string(source).unwrap(), contents);
        }
    }
}

#[test]
fn missing_config_repair_revokes_only_exact_pending_grant_and_preserves_other_holds() {
    use crate::journal::revocation::{RevocationState, ZygiskRevocation};
    let root = tempfile::tempdir().unwrap();
    let module = root.path().join("module");
    let (mut journal, authority, _) = crate::native_boot::transition::test_fixture(&module);
    journal.config_revision = 17;
    journal.zygisk_revocation = Some(ZygiskRevocation {
        operation: Some(OperationId(43)),
        current_boot: authority.reference().unwrap().boot_id,
        authority: authority.clone(),
        state: RevocationState::Unknown,
    });
    journal.upsert(uncertain(
        10,
        ConfigPhase::Native,
        CanonicalConfig::default(),
        17,
    ));
    journal.upsert(uncertain(
        11,
        ConfigPhase::Cleanup,
        CanonicalConfig::default(),
        17,
    ));
    journal::write_atomic(&root.path().join("operations.json"), &journal).unwrap();
    let state = load(root.path(), 2);
    assert_eq!(
        vpnhide_core_rpc::recovery::refusal(
            &state.snapshot(),
            vpnhide_core_rpc::recovery::RecoveryAction::Repair
        ),
        None
    );
    let op = command(&state, ConfigEdit::RepairActivation);
    let receipt = state.get_operation_record(op).unwrap();
    assert_eq!(receipt.settled, Some(PhaseOutcome::Confirmed));
    assert!(state.snapshot().config.is_none());
    assert!(!root.path().join("config.json").exists());
    assert_eq!(state.config_revision(), 17);
    let stored = journal::read(&root.path().join("operations.json")).unwrap();
    assert!(stored.native_startup.is_none());
    assert_eq!(stored.zygisk_authority, Some(authority));
    assert!(stored.get(op).unwrap().is_revocation_only_repair());
    assert!(!state.snapshot().apply_evidence.iter().any(|e| e.op == op));
    for old in [10, 11] {
        assert!(
            stored
                .get(OperationId(old))
                .unwrap()
                .superseded_phases
                .is_empty()
        );
    }
    assert!(stored.held_resources().contains(&ConfigPhase::Native));
    assert!(stored.held_resources().contains(&ConfigPhase::Cleanup));
    assert!(!stored.revocation_uncertain());
    assert!(
        !module
            .join(vpnhide_protocol::zygisk_activation::FILE_NAME)
            .exists()
    );
    let before = fs::read(root.path().join("operations.json")).unwrap();
    state
        .submit_command(
            op,
            CoreCommand::Edit {
                base: BaseRef { revision: 17 },
                edit: ConfigEdit::RepairActivation,
            },
        )
        .unwrap();
    assert_eq!(
        fs::read(root.path().join("operations.json")).unwrap(),
        before
    );
}

#[test]
fn missing_config_revocation_repair_recovers_a_durable_completion_without_new_policy() {
    use crate::journal::revocation::{RevocationState, ZygiskRevocation};
    let root = tempfile::tempdir().unwrap();
    let module = root.path().join("module");
    let (mut journal, authority, _) = crate::native_boot::transition::test_fixture(&module);
    journal.config_revision = 7;
    journal.zygisk_revocation = Some(ZygiskRevocation {
        operation: Some(OperationId(43)),
        current_boot: authority.reference().unwrap().boot_id,
        authority: authority.clone(),
        state: RevocationState::Unknown,
    });
    journal::write_atomic(&root.path().join("operations.json"), &journal).unwrap();
    let state = load(root.path(), 1);
    let op = state.issue_operation_id().unwrap();
    state
        .call_writer(move |state| {
            // Paired admission/intent + revocation completion are durable;
            // fail only final operation settlement, after the effect's own receipt.
            journal::fail_write(3, false);
            state.execute_edit_opts(
                op,
                BaseRef { revision: 7 },
                &ConfigEdit::RepairActivation,
                SubmitOptions::default(),
                CommandSpec::default(),
            )
        })
        .unwrap()
        .unwrap();
    assert!(state.snapshot().storage_fault.is_some());
    drop(state);
    let recovered = load(root.path(), 2);
    assert_eq!(
        recovered.get_operation_record(op).unwrap().settled,
        Some(PhaseOutcome::Confirmed)
    );
    assert!(recovered.snapshot().config.is_none());
    assert_eq!(recovered.config_revision(), 7);
    assert!(
        recovered
            .snapshot()
            .zygisk_revocation
            .as_ref()
            .is_some_and(|r| !r.pending())
    );
    assert_eq!(
        journal::read(&root.path().join("operations.json"))
            .unwrap()
            .zygisk_authority,
        Some(authority)
    );
}

#[test]
fn full_reset_revokes_bundled_grant_and_withdraws_prior_publication_and_applied_state() {
    use vpnhide_core_rpc::{
        NativeBackend, NativeBootOutcome, SelectionMode, ZygiskLifecycle, ZygiskPublication,
        ZygiskTargets,
    };
    let root = tempfile::tempdir().unwrap();
    let module = root.path().join("module");
    let (mut stored, mut authority, target) = crate::native_boot::transition::test_fixture(&module);
    let boot =
        crate::native_boot::read_boot_id(std::path::Path::new(crate::native_boot::BOOT_ID_PATH));
    authority
        .intent
        .zygisk_authorization
        .as_mut()
        .unwrap()
        .boot_id = boot.clone();
    target
        .publish_authorized(&vpnhide_protocol::zygisk_activation::Authorization {
            boot_id: boot.clone(),
            generation: 42,
            wire: authority.intent.wire.clone(),
        })
        .unwrap();
    stored.zygisk_authority = Some(authority.clone());
    stored.config_revision = 3;
    config_file::write_atomic(
        &root.path().join("config.json"),
        &CanonicalConfig::default(),
    )
    .unwrap();
    journal::write_atomic(&root.path().join("operations.json"), &stored).unwrap();
    let state = CoreState::load_with_journal(
        Publisher::new(1, StateSnapshot::default()),
        root.path().join("config.json"),
        Some(root.path().join("operations.json")),
        Some(root.path().join("unused-ctl")),
    )
    .unwrap();
    native_none(&state, root.path());
    fs::write(root.path().join("boot-id"), &boot).unwrap();
    {
        let handle = state.native_handle().unwrap();
        let mut native = handle.lock().unwrap();
        native.env.zygisk_module = Some(module.clone());
        native.status.boot_id = boot;
        native.selection = Some(crate::native_boot::Selection {
            backend: NativeBackend::Zygisk,
            mode: SelectionMode::Authorized,
            filesystem_hiding: None,
        });
        native.status.outcome = NativeBootOutcome::Selected {
            backend: NativeBackend::Zygisk,
            mode: SelectionMode::Authorized,
            filesystem_hiding: None,
        };
        native.status.zygisk_targets = ZygiskTargets::Published;
    }
    state.set_effect_paths(Some(crate::effects::EffectPaths {
        root: root.path().join("effects"),
    }));
    state.publisher.update(|s| {
        s.backend.applied = Some(NativeBackend::Zygisk);
        let mut lifecycle = ZygiskLifecycle::pending(authority.reference().unwrap(), 1);
        lifecycle.publication = ZygiskPublication::Matching;
        s.zygisk_lifecycle = Some(lifecycle);
    });
    let op = state.issue_operation_id().unwrap();
    state.submit_command(op, CoreCommand::FullReset).unwrap();
    assert_eq!(
        state.get_operation_record(op).unwrap().settled,
        Some(PhaseOutcome::Confirmed)
    );
    let snapshot = state.snapshot();
    assert!(snapshot.config.is_none());
    assert_eq!(snapshot.backend.applied, None);
    assert!(
        !snapshot
            .apply_evidence
            .iter()
            .any(|e| e.phase == ConfigPhase::Native)
    );
    assert_eq!(
        snapshot.backend.boot.unwrap().zygisk_targets,
        ZygiskTargets::Retired
    );
    assert_eq!(
        snapshot.zygisk_lifecycle.unwrap().publication,
        ZygiskPublication::NotObserved
    );
    assert!(
        snapshot
            .zygisk_revocation
            .as_ref()
            .is_some_and(|r| !r.pending())
    );
    state.refresh_zygisk_lifecycle();
    assert_eq!(
        state.snapshot().zygisk_lifecycle.unwrap().publication,
        ZygiskPublication::Absent
    );
    assert!(target.authorization_absent().unwrap());
    let stored = journal::read(&root.path().join("operations.json")).unwrap();
    assert_eq!(
        stored.zygisk_authority.unwrap().operation,
        op,
        "empty publication is historical, not current apply"
    );
}

#[test]
fn narrow_repair_first_write_and_completion_faults_leave_no_orphan_hold_even_at_zero_retention() {
    use crate::journal::revocation::{RevocationState, ZygiskRevocation};
    for fail in [1, 2] {
        let root = tempfile::tempdir().unwrap();
        let module = root.path().join("module");
        let (mut stored, authority, target) = crate::native_boot::transition::test_fixture(&module);
        stored.retain_settled = 0;
        stored.zygisk_revocation = Some(ZygiskRevocation {
            operation: Some(OperationId(43)),
            current_boot: authority.reference().unwrap().boot_id,
            authority,
            state: RevocationState::Unknown,
        });
        journal::write_atomic(&root.path().join("operations.json"), &stored).unwrap();
        let state = load(root.path(), 1);
        let op = state.issue_operation_id().unwrap();
        let result = state
            .call_writer(move |state| {
                journal::fail_write(fail, false);
                state.execute_edit_opts(
                    op,
                    BaseRef { revision: 0 },
                    &ConfigEdit::RepairActivation,
                    SubmitOptions::default(),
                    CommandSpec::default(),
                )
            })
            .unwrap();
        assert_eq!(result.is_ok(), fail == 2);
        assert_eq!(target.authorization_absent().unwrap(), fail == 2);
        drop(state);
        let recovered = load(root.path(), 2);
        let repair = command(&recovered, ConfigEdit::RepairActivation);
        assert_eq!(
            recovered.get_operation_record(repair).unwrap().settled,
            Some(PhaseOutcome::Confirmed)
        );
        let stored = journal::read(&root.path().join("operations.json")).unwrap();
        assert!(stored.held_resources().is_empty());
        assert!(stored.expired_holds.is_empty());
        assert!(stored.native_startup.is_none());
        assert!(recovered.snapshot().config.is_none());
        assert!(
            !recovered
                .snapshot()
                .apply_evidence
                .iter()
                .any(|e| e.phase == ConfigPhase::Native)
        );
        if fail == 2 {
            assert_eq!(
                recovered.get_operation_record(op).unwrap().settled,
                Some(PhaseOutcome::Unknown)
            );
        }
    }
}

#[test]
fn old_revoked_zygisk_grant_does_not_withdraw_current_builtin_delivery() {
    use crate::journal::revocation::{RevocationState, ZygiskRevocation};
    use vpnhide_core_rpc::{NativeBackend, NativeBootOutcome, SelectionMode};
    let root = tempfile::tempdir().unwrap();
    let (mut stored, authority, _) =
        crate::native_boot::transition::test_fixture(&root.path().join("module"));
    stored.config_revision = 3;
    stored.zygisk_revocation = Some(ZygiskRevocation {
        operation: Some(OperationId(43)),
        current_boot: "previous-boot".into(),
        authority,
        state: RevocationState::Confirmed,
    });
    let mut applied = uncertain(50, ConfigPhase::Native, CanonicalConfig::default(), 3);
    applied.finalized_unknown = false;
    applied.native_intent = Some(journal::NativeIntent {
        backend: NativeBackend::Builtin,
        wire: "vpnhide 2 config\ndebug 0\ndefault 0\nend 0\n".into(),
        zygisk_module: None,
        zygisk_authorization: None,
    });
    applied.record.phases[0].outcome = PhaseOutcome::Confirmed;
    applied.record.settled = Some(PhaseOutcome::Confirmed);
    stored.upsert(applied);
    config_file::write_atomic(
        &root.path().join("config.json"),
        &CanonicalConfig::default(),
    )
    .unwrap();
    journal::write_atomic(&root.path().join("operations.json"), &stored).unwrap();
    let state = load(root.path(), 2);
    native_none(&state, root.path());
    {
        let handle = state.native_handle().unwrap();
        let mut boot = handle.lock().unwrap();
        boot.selection = Some(crate::native_boot::Selection {
            backend: NativeBackend::Builtin,
            mode: SelectionMode::Adopted,
            filesystem_hiding: None,
        });
        boot.status.outcome = NativeBootOutcome::Selected {
            backend: NativeBackend::Builtin,
            mode: SelectionMode::Adopted,
            filesystem_hiding: None,
        };
    }
    state.publish_native_boot(Some(NativeBackend::Builtin));
    let store = state.engine.store.lock().unwrap().clone();
    state.publish_store(&store);
    let snapshot = state.snapshot();
    assert_eq!(snapshot.backend.applied, Some(NativeBackend::Builtin));
    assert!(
        snapshot
            .apply_evidence
            .iter()
            .any(|e| e.op == OperationId(50)
                && e.phase == ConfigPhase::Native
                && e.outcome == PhaseOutcome::Confirmed)
    );
}
