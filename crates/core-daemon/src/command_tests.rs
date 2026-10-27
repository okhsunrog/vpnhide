//! Command-owner tests use the actual durable queue and receipt store.
use super::*;
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
            let mut store = state.config.lock().unwrap().clone();
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
        .writer
        .get()
        .unwrap()
        .try_send(Box::new(move |_| {
            started.send(()).unwrap();
            wait.recv().unwrap();
        }))
        .unwrap();
    entered
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    for _ in 0..16 {
        state
            .writer
            .get()
            .unwrap()
            .try_send(Box::new(|_| {}))
            .unwrap();
    }
    assert_eq!(state.call_writer(|_| ()), Err(SubmitError::Busy));
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
        .writer
        .get()
        .unwrap()
        .try_send(Box::new(move |state| {
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
        .writer
        .get()
        .unwrap()
        .try_send(Box::new(move |_| {
            started.send(()).unwrap();
            wait.recv().unwrap();
        }))
        .unwrap();
    entered
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    state
        .writer
        .get()
        .unwrap()
        .try_send(Box::new(move |state| {
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
    assert!(state.writer_join.lock().unwrap().is_none());
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
        .writer
        .get()
        .unwrap()
        .try_send(Box::new(move |_| {
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
            let mut store = state.config.lock().unwrap().clone();
            store.journal.debug_leases[0].expires_at_ms = 0;
            state.persist_store(&store).unwrap();
            state.expire_debug_leases();
        })
        .unwrap();
    assert!(!state.snapshot().config.unwrap().debug);
    assert!(!state.debug_lease_held());
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
