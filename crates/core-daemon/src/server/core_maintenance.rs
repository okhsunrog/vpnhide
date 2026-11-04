//! Replacement effects stay on the existing durable command writer.
use super::*;
use crate::maintenance::replacement::{
    PreparationPermit, PreparedReplacement, ReplacementPaths, ReplacementWork,
};
use vpnhide_core_rpc::{
    CoreMaintenanceFailure as Failure, CoreMaintenanceStage as Stage, CoreReplacementRequest,
};

use crate::maintenance::replacement::request_hash;
impl CoreState {
    fn replacement_paths(&self) -> Result<ReplacementPaths, SubmitError> {
        let root = self
            .effect_paths
            .read()
            .expect("effect paths")
            .as_ref()
            .ok_or(SubmitError::Unavailable)?
            .root
            .clone();
        let persistent = self
            .journal_path
            .as_ref()
            .and_then(|p| p.parent())
            .ok_or(SubmitError::Unavailable)?
            .to_path_buf();
        let app = self
            .probe_env
            .read()
            .expect("probe env")
            .app_dir
            .clone()
            .ok_or(SubmitError::Unavailable)?;
        Ok(ReplacementPaths {
            root,
            persistent,
            app,
        })
    }
    pub(super) fn reconcile_core_replacement(&self, store: &mut ConfigStore) -> Result<(), String> {
        use crate::maintenance::replacement::GenerationProof;
        let Some(mut work) = store.journal.replacement.clone() else {
            return Ok(());
        };
        if !work.retained() {
            return Ok(());
        }
        let paths = self
            .replacement_paths()
            .map_err(|_| "maintenance paths unavailable")?;
        let proof = work.reconcile(&paths);
        match proof {
            Ok(GenerationProof::OldRunning) => return Ok(()),
            Ok(GenerationProof::NewRunning) => {
                work.status.stage = Stage::Verified;
                work.status.failure = None;
                work.retired = true;
            }
            Ok(GenerationProof::NoDispatch) => {
                work.status.stage = Stage::FailedKnown;
                work.status.failure = Some(Failure::InterruptedBeforeDispatch);
            }
            Err(failure) => {
                work.status.stage = Stage::Unknown;
                work.status.failure = Some(failure);
                store.journal.replacement = Some(work.clone());
                self.persist_replacement_store(store, work.status.operation)
                    .map_err(|_| "maintenance reconciliation storage unavailable")?;
                return Err(format!(
                    "Core replacement generation unverified: {failure:?}"
                ));
            }
        }
        // A later observation releases the resource hold; it does not rewrite
        // an Unknown outcome already returned to a disconnected caller.
        if let Some(entry) = store.journal.get_mut(work.status.operation)
            && !entry.superseded_phases.contains(&ConfigPhase::Maintenance)
        {
            entry.superseded_phases.push(ConfigPhase::Maintenance);
        }
        store.journal.replacement = Some(work.clone());
        self.persist_replacement_store(store, work.status.operation)
            .map_err(|_| "maintenance reconciliation storage unavailable".into())
    }
    pub(super) fn prepare_replacement(
        &self,
        op: OperationId,
        command: &CoreCommand,
        caller: Option<u32>,
    ) -> Result<Option<PreparedReplacement>, SubmitError> {
        let CoreCommand::StageCoreReplacement { base, request } = command else {
            return Ok(None);
        };
        if request.descriptor.len() > vpnhide_package_trust::MAX_ENVELOPE_BYTES {
            return Err(SubmitError::Rejected {
                reason: "maintenance_descriptor_too_large".into(),
            });
        }
        {
            let store = self.engine.store.lock().expect("config mutex");
            if store.journal.get(op).is_some() {
                return Ok(None);
            }
            if op.0 == 0 || op.0 > store.journal.issued_through {
                return Err(SubmitError::Rejected {
                    reason: "operation_not_allocated".into(),
                });
            }
            if store.journal.is_retired(op) {
                return Err(SubmitError::Expired);
            }
            if base.revision > store.config_revision {
                return Err(SubmitError::Conflict {
                    field: "future maintenance base".into(),
                });
            }
            if store
                .journal
                .replacement
                .as_ref()
                .is_some_and(|w| w.retained())
            {
                return Err(SubmitError::Busy);
            }
        }
        let permit = PreparationPermit::acquire(self.replacement_preparing.clone())
            .ok_or(SubmitError::Busy)?;
        PreparedReplacement::prepare(
            self.replacement_paths()?,
            request.clone(),
            op,
            caller,
            permit,
        )
        .map(Some)
        .map_err(|reason| SubmitError::Rejected { reason })
    }
    pub(crate) fn execute_command_prepared(
        &self,
        op: OperationId,
        command: CoreCommand,
        admitted: Option<crate::command_engine::AdmissionReply>,
        legacy: Option<crate::maintenance::PreparedMaintenance>,
        replacement: Option<PreparedReplacement>,
        backend_choice: Option<super::backend_choice::PreparedBackendChoice>,
    ) -> Result<Accepted, SubmitError> {
        match command {
            CoreCommand::StageCoreReplacement { base, request } => {
                self.execute_core_replacement(op, base, request, admitted, replacement)
            }
            CoreCommand::StageBackendChoice { base, request } => {
                self.execute_backend_choice(op, base, request, admitted, backend_choice)
            }
            command => self.execute_command_admitted(op, command, admitted, legacy),
        }
    }
    fn persist_replacement_store(
        &self,
        store: &ConfigStore,
        op: OperationId,
    ) -> Result<(), SubmitError> {
        let path = self.journal_path.as_ref().ok_or(SubmitError::Unavailable)?;
        if let Err(error) = journal::write_atomic(path, &store.journal) {
            let previous = self.engine.store.lock().expect("config mutex").clone();
            self.freeze_storage_fault(&previous, op, &error);
            return Err(SubmitError::Unavailable);
        }
        *self.engine.store.lock().expect("config mutex") = store.clone();
        self.publish_store(store);
        Ok(())
    }
    fn replacement_progress(
        &self,
        store: &mut ConfigStore,
        work: &mut ReplacementWork,
        stage: Stage,
        failure: Option<Failure>,
        outcome: Option<PhaseOutcome>,
    ) -> Result<(), SubmitError> {
        work.status.stage = stage;
        work.status.failure = failure;
        let entry = store
            .journal
            .get_mut(work.status.operation)
            .ok_or(SubmitError::Unavailable)?;
        entry
            .record
            .phases
            .first_mut()
            .ok_or(SubmitError::Unavailable)?
            .outcome = outcome.unwrap_or(PhaseOutcome::Running);
        entry.record.settled = outcome;
        entry.finalized_unknown = outcome == Some(PhaseOutcome::Unknown);
        store.journal.replacement = Some(work.clone());
        self.persist_replacement_store(store, work.status.operation)
    }
    fn replacement_refused(
        &self,
        store: &mut ConfigStore,
        work: &mut ReplacementWork,
        failure: Failure,
        unknown: bool,
    ) -> Result<Accepted, SubmitError> {
        self.replacement_progress(
            store,
            work,
            if unknown {
                Stage::Unknown
            } else {
                Stage::FailedKnown
            },
            Some(failure),
            Some(if unknown {
                PhaseOutcome::Unknown
            } else {
                PhaseOutcome::FailedKnown
            }),
        )?;
        Ok(Accepted {
            op: work.status.operation,
        })
    }
    fn execute_core_replacement(
        &self,
        op: OperationId,
        base: BaseRef,
        request: CoreReplacementRequest,
        admitted: Option<crate::command_engine::AdmissionReply>,
        prepared: Option<PreparedReplacement>,
    ) -> Result<Accepted, SubmitError> {
        if self.snapshot().startup_error.is_some() || self.snapshot().storage_fault.is_some() {
            return Err(SubmitError::Unavailable);
        }
        let mut store = self.engine.store.lock().expect("config mutex").clone();
        let intent = journal::CommandIntent::StageCoreReplacement {
            request_sha256: request_hash(&request),
        };
        if let Some(old) = store.journal.get(op) {
            return if old.base == base && old.command.as_ref() == Some(&intent) {
                Ok(Accepted { op })
            } else {
                Err(SubmitError::IdReused)
            };
        }
        if store.journal.is_retired(op) {
            return Err(SubmitError::Expired);
        }
        if op.0 == 0 || op.0 > store.journal.issued_through {
            return Err(SubmitError::Rejected {
                reason: "operation_not_allocated".into(),
            });
        }
        if base.revision > store.config_revision {
            return Err(SubmitError::Conflict {
                field: "future maintenance base".into(),
            });
        }
        if store
            .journal
            .replacement
            .as_ref()
            .is_some_and(|w| w.retained())
        {
            return Err(SubmitError::Busy);
        }
        let prepared = prepared.ok_or(SubmitError::Rejected {
            reason: "replacement_preparation_required".into(),
        })?;
        let paths = self.replacement_paths()?;
        if prepared.work.request != request
            || prepared.work.status.operation != op
            || prepared.paths.root != paths.root
            || prepared.paths.persistent != paths.persistent
            || prepared.paths.app != paths.app
        {
            return Err(SubmitError::Rejected {
                reason: "maintenance_preparation_mismatch".into(),
            });
        }
        prepared
            .require_fresh()
            .map_err(|_| SubmitError::Rejected {
                reason: "maintenance_sources_changed".into(),
            })?;
        let floor = prepared.floor.lock().map_err(|_| SubmitError::Busy)?;
        let mut work = prepared.work.clone();
        store.journal.replacement = Some(work.clone());
        store.journal.upsert(journal::JournalEntry {
            maintenance: None,
            maintenance_report: None,
            admission: Some(journal::Admission {
                from_revision: store.config_revision,
                commit_revision: store.config_revision,
                writes: vec![],
            }),
            command: Some(intent),
            superseded_by: None,
            superseded_phases: vec![],
            finalized_unknown: false,
            base,
            edit: ConfigEdit::RepairActivation,
            candidate: None,
            native_intent: None,
            record: OperationRecord {
                op,
                phases: vec![vpnhide_core_rpc::PhaseRecord {
                    phase: ConfigPhase::Maintenance,
                    outcome: PhaseOutcome::NotAttempted,
                }],
                settled: None,
                ports: None,
            },
        });
        self.persist_replacement_store(&store, op)?;
        if let Some(reply) = admitted {
            let _ = reply.send(Ok(Accepted { op }));
        }
        crate::recovery::maybe_crash_for_test("maintenance-accepted");
        if let Err(error) = floor.accept_prepared(&prepared.package) {
            let known = matches!(
                error,
                crate::maintenance::package::PackageError::Trust(_)
                    | crate::maintenance::package::PackageError::SourceChanged
                    | crate::maintenance::package::PackageError::UnsafeStorage
            );
            return self.replacement_refused(
                &mut store,
                &mut work,
                if known {
                    Failure::InvalidPackage
                } else {
                    Failure::StorageUncertain
                },
                !known,
            );
        }
        work.dispatch_intent = true;
        self.replacement_progress(&mut store, &mut work, Stage::Dispatching, None, None)?;
        crate::recovery::maybe_crash_for_test("maintenance-before-dispatch");
        if prepared.require_fresh().is_err() {
            return self.replacement_refused(
                &mut store,
                &mut work,
                Failure::LiveImageChanged,
                false,
            );
        }
        let complete = prepared
            .manager
            .stage(prepared.package.file())
            .is_ok_and(|r| !r.timed_out && r.output.status.success());
        crate::recovery::maybe_crash_for_test("maintenance-after-dispatch");
        if !complete {
            return self.replacement_refused(
                &mut store,
                &mut work,
                Failure::DispatchUncertain,
                true,
            );
        }
        self.replacement_progress(&mut store, &mut work, Stage::VerifyingStaged, None, None)?;
        let staged = prepared.verify_staged();
        let failure = if staged.is_err() {
            Some(Failure::StagedImageInvalid)
        } else if prepared.live_unchanged().is_err() {
            Some(Failure::LiveImageChanged)
        } else if prepared.manager.require_unchanged().is_err() {
            Some(Failure::ManagerChanged)
        } else if prepared.legacy_unchanged().is_err() {
            Some(Failure::LegacyChanged)
        } else {
            None
        };
        if let Some(failure) = failure {
            return self.replacement_refused(&mut store, &mut work, failure, true);
        }
        work.staged = Some(staged.map_err(|_| SubmitError::Unavailable)?);
        self.replacement_progress(&mut store, &mut work, Stage::RetiringLegacy, None, None)?;
        crate::recovery::maybe_crash_for_test("maintenance-before-retirement");
        let retired = prepared.retire(work.staged.as_ref().expect("verified staged image"));
        if retired.is_err() {
            return self.replacement_refused(&mut store, &mut work, Failure::LegacyChanged, true);
        }
        work.retired = true;
        crate::recovery::maybe_crash_for_test("maintenance-after-retirement");
        self.replacement_progress(
            &mut store,
            &mut work,
            Stage::PendingRestart,
            None,
            Some(PhaseOutcome::Confirmed),
        )?;
        Ok(Accepted { op })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpnhide_core_rpc::StateSnapshot;
    fn fixture() -> (tempfile::TempDir, Arc<CoreState>, PreparedReplacement) {
        let root = tempfile::tempdir().unwrap();
        let (state, prepared) = fixture_at(root.path());
        (root, state, prepared)
    }
    fn fixture_at(root: &std::path::Path) -> (Arc<CoreState>, PreparedReplacement) {
        let persistent = root.join("persistent");
        std::fs::create_dir(&persistent).unwrap();
        let state = CoreState::load_with_journal(
            Publisher::new(1, StateSnapshot::default()),
            persistent.join("config.json"),
            Some(persistent.join("operations.json")),
            None,
        )
        .unwrap();
        state.set_effect_paths(Some(crate::effects::EffectPaths { root: root.into() }));
        state.set_probe_env(crate::diagnostics::ProbeEnv {
            core_exe: "/unavailable".into(),
            app_dir: Some(root.join("app")),
        });
        let op = state.issue_operation_id().unwrap();
        let prepared = crate::maintenance::replacement::tests::prepared(
            state.replacement_paths().unwrap(),
            op,
        );
        (state, prepared)
    }
    #[test]
    fn actual_writer_stages_once_keeps_config_and_reboot_pending_receipt_durable() {
        let (root, state, p) = fixture();
        let op = p.work.status.operation;
        let request = p.work.request.clone();
        let command = CoreCommand::StageCoreReplacement {
            base: BaseRef { revision: 0 },
            request: request.clone(),
        };
        let copy = command.clone();
        state
            .call_writer(move |state| {
                state.execute_command_prepared(op, command, None, None, Some(p), None)
            })
            .unwrap()
            .unwrap();
        assert_eq!(
            state.snapshot().maintenance.unwrap().stage,
            Stage::PendingRestart
        );
        assert!(state.snapshot().config.is_none());
        assert_eq!(state.config_revision(), 0);
        let durable = journal::read(&root.path().join("persistent/operations.json")).unwrap();
        assert!(durable.replacement.as_ref().unwrap().retired);
        for stage in [Stage::PendingRestart, Stage::Verified, Stage::FailedKnown] {
            let mut retained = durable.clone();
            retained.retain_settled = 0;
            let work = retained.replacement.as_mut().unwrap();
            work.status.stage = stage;
            work.status.failure = if stage == Stage::FailedKnown {
                Some(Failure::InvalidPackage)
            } else {
                None
            };
            retained.prune();
            assert!(retained.get(op).is_some());
            let saved = root.path().join("pruned.json");
            journal::write_atomic(&saved, &retained).unwrap();
            assert!(journal::read(&saved).is_ok());
        }

        assert_eq!(
            durable.get(op).unwrap().record.settled,
            Some(PhaseOutcome::Confirmed)
        );
        state.submit_command(op, copy).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.path().join("data/adb/dispatch-count")).unwrap(),
            "1"
        );
        assert!(matches!(
            state.submit_command(state.issue_operation_id().unwrap(), CoreCommand::FullReset),
            Err(SubmitError::Busy)
        ));
    }
    #[test]
    fn exit_zero_missing_image_is_unknown_and_restart_never_repeats_dispatch() {
        let (root, state, p) = fixture();
        std::fs::write(root.path().join("data/adb/mode"), "none").unwrap();
        let op = p.work.status.operation;
        let command = CoreCommand::StageCoreReplacement {
            base: BaseRef { revision: 0 },
            request: p.work.request.clone(),
        };
        let copy = command.clone();
        state
            .call_writer(move |s| {
                s.execute_command_prepared(op, command, None, None, Some(p), None)
            })
            .unwrap()
            .unwrap();
        assert_eq!(state.snapshot().maintenance.unwrap().stage, Stage::Unknown);
        assert!(state.snapshot().mutation_held);
        for edit in [
            ConfigEdit::RepairActivation,
            ConfigEdit::Replace {
                config: CanonicalConfig::default(),
            },
        ] {
            let next = state.issue_operation_id().unwrap();
            assert!(matches!(
                state.submit_command(
                    next,
                    CoreCommand::Edit {
                        base: BaseRef { revision: 0 },
                        edit
                    }
                ),
                Err(SubmitError::Busy)
            ));
        }
        let path = root.path().join("persistent/operations.json");
        let mut durable = journal::read(&path).unwrap();
        durable.fence_unresolved();
        assert_eq!(
            durable.get(op).unwrap().record.settled,
            Some(PhaseOutcome::Unknown)
        );
        assert!(durable.held_resources().contains(&ConfigPhase::Maintenance));
        state.submit_command(op, copy).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.path().join("data/adb/dispatch-count")).unwrap(),
            "1"
        );
    }
    #[test]
    fn acknowledged_admission_survives_waiter_drop_and_state_reads_stay_available() {
        let (root, state, p) = fixture();
        std::fs::write(root.path().join("data/adb/mode"), "delay").unwrap();
        let op = p.work.status.operation;
        let command = CoreCommand::StageCoreReplacement {
            base: BaseRef { revision: 0 },
            request: p.work.request.clone(),
        };
        let accepted = state
            .engine
            .submit(
                op,
                command,
                crate::command_engine::AdmissionTicket::new(
                    std::time::Instant::now() + std::time::Duration::from_secs(2),
                ),
                None,
                Some(p),
                None,
            )
            .unwrap();
        assert_eq!(accepted.op, op);
        let read_started = std::time::Instant::now();
        assert!(state.snapshot().maintenance.is_some());
        assert!(read_started.elapsed() < std::time::Duration::from_millis(100));
        // A barrier waits for accepted work, not the now-dropped admission waiter.
        state.call_writer(|_| ()).unwrap();
        assert_eq!(
            state.snapshot().maintenance.unwrap().stage,
            Stage::PendingRestart
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("data/adb/dispatch-count")).unwrap(),
            "1"
        );
    }
    #[test]
    fn crash_child() {
        let Some(root) = std::env::var_os("VPNHIDE_MAINTENANCE_CHILD_ROOT") else {
            return;
        };
        let (state, p) = fixture_at(std::path::Path::new(&root));
        let op = p.work.status.operation;
        let command = CoreCommand::StageCoreReplacement {
            base: BaseRef { revision: 0 },
            request: p.work.request.clone(),
        };
        state
            .call_writer(move |s| {
                s.execute_command_prepared(op, command, None, None, Some(p), None)
            })
            .unwrap()
            .unwrap();
        panic!("expected crash point was not reached");
    }
    #[test]
    fn real_process_crashes_retain_identity_and_never_replay_manager() {
        for point in [
            "maintenance-accepted",
            "maintenance-before-dispatch",
            "maintenance-after-dispatch",
            "maintenance-before-retirement",
            "maintenance-after-retirement",
        ] {
            let root = tempfile::tempdir().unwrap();
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "server::core_maintenance::tests::crash_child",
                    "--nocapture",
                ])
                .env("VPNHIDE_MAINTENANCE_CHILD_ROOT", root.path())
                .env("VPNHIDE_TEST_CRASH_AFTER", point)
                .output()
                .unwrap();
            assert_eq!(
                result.status.code(),
                Some(97),
                "{point}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            let path = root.path().join("persistent/operations.json");
            let mut durable = journal::read(&path).unwrap();
            durable.fence_unresolved();
            let work = durable.replacement.as_ref().unwrap();
            assert_eq!(work.status.stage, Stage::Unknown, "{point}");
            assert_eq!(work.dispatch_intent, point != "maintenance-accepted");
            assert!(durable.held_resources().contains(&ConfigPhase::Maintenance));
            let count = root.path().join("data/adb/dispatch-count");
            let before = std::fs::read_to_string(&count).unwrap_or_default();
            journal::write_atomic(&path, &durable).unwrap();
            let state = CoreState::load_with_journal(
                Publisher::new(2, StateSnapshot::default()),
                root.path().join("persistent/config.json"),
                Some(path),
                None,
            )
            .unwrap();
            assert_eq!(state.snapshot().maintenance.unwrap().stage, Stage::Unknown);
            state.call_writer(|_| ()).unwrap();
            assert_eq!(std::fs::read_to_string(count).unwrap_or_default(), before);
        }
    }
    #[test]
    fn retained_work_refuses_corrupt_target_stage_and_receipt_identity() {
        let (root, state, p) = fixture();
        let op = p.work.status.operation;
        let command = CoreCommand::StageCoreReplacement {
            base: BaseRef { revision: 0 },
            request: p.work.request.clone(),
        };
        state
            .call_writer(move |s| {
                s.execute_command_prepared(op, command, None, None, Some(p), None)
            })
            .unwrap()
            .unwrap();
        let path = root.path().join("persistent/operations.json");
        let valid: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let mut cases = Vec::new();
        let mut v = valid.clone();
        v["replacement"]["request"]["expected_package_sha256"] = "bad".into();
        cases.push(v);
        let mut v = valid.clone();
        v["replacement"]["status"]["operation"] = 999.into();
        cases.push(v);
        let mut v = valid.clone();
        v["replacement"]["retired"] = false.into();
        cases.push(v);
        let mut v = valid.clone();
        v["replacement"]["dispatch_intent"] = false.into();
        cases.push(v);
        let mut v = valid.clone();
        v["replacement"]["staged"] = serde_json::Value::Null;
        cases.push(v);
        let mut v = valid.clone();
        v["version"] = 12.into();
        cases.push(v);
        for invalid in cases {
            std::fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
            assert!(journal::read(&path).is_err());
        }
        std::fs::write(&path, serde_json::to_vec(&valid).unwrap()).unwrap();
        assert!(journal::read(&path).is_ok());
    }
}
