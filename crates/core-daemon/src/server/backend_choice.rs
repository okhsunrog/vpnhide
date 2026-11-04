//! Next-boot device-local family intent on the existing durable writer. Recording
//! an intent does not claim runtime delivery or bypass a native lifecycle hold.
use super::*;
use crate::native_boot::prepared::{ChoicePreview, preview_choice};
use crate::payloads::selection::Ownership;
use vpnhide_core_rpc::{
    BackendChoice, BackendChoiceRequest, BackendChoiceStage, BackendChoiceStatus, NativeBackend,
};

pub(crate) struct PreparedBackendChoice {
    pub(super) preview: ChoicePreview,
    pub(super) config: CanonicalConfig,
}

pub(super) fn status(journal: &Journal) -> BackendChoiceStatus {
    let record = journal.backend_choice.as_ref();
    BackendChoiceStatus {
        revision: record.map_or(0, |r| r.revision),
        operation: record.map(|r| r.operation),
        desired: record.map_or(BackendChoice::Auto, |r| r.request.choice),
        requested_in_boot: record.map(|r| r.requested_in_boot.clone()),
        effective: None,
        capacity_backends: vec![],
        stage: BackendChoiceStage::Unverified,
        options: implemented_options(),
    }
}

pub(crate) fn implemented_options() -> Vec<BackendChoice> {
    vec![
        BackendChoice::Auto,
        BackendChoice::Fixed {
            backend: NativeBackend::Builtin,
        },
        BackendChoice::Fixed {
            backend: NativeBackend::Kmod,
        },
        BackendChoice::Fixed {
            backend: NativeBackend::Kpm,
        },
    ]
}

impl CoreState {
    pub(super) fn prepare_backend_choice(
        &self,
        op: OperationId,
        command: &CoreCommand,
        deadline: std::time::Instant,
        cancelled: &CancellationToken,
    ) -> Result<Option<PreparedBackendChoice>, SubmitError> {
        let CoreCommand::StageBackendChoice { base, request } = command else {
            return Ok(None);
        };
        let config = {
            let store = self.engine.store.lock().expect("config mutex");
            if store.journal.get(op).is_some() {
                return Ok(None);
            }
            self.check_backend_choice_request(&store, op, *base, request)?;
            store.config.clone().ok_or(SubmitError::Unavailable)?
        };
        let handle = self.native_handle().ok_or(SubmitError::Unavailable)?;
        let boot = handle.lock().expect("native boot");
        if boot.selected() == Some(NativeBackend::Zygisk) {
            return Err(SubmitError::Rejected {
                reason: "zygisk_retirement_preparation_required".into(),
            });
        }
        let env = boot.env.clone();
        drop(boot);
        let packages =
            vpnhide_activator::PackageUidMap::from_pm_before(deadline, || cancelled.is_cancelled())
                .map_err(|_| SubmitError::Rejected {
                    reason: "backend_choice_inventory_unavailable".into(),
                })?;
        let preview =
            preview_choice(&env, &config, &packages).map_err(|e| SubmitError::Rejected {
                reason: format!("backend_choice_unavailable: {}", e.reason),
            })?;
        let prepared = PreparedBackendChoice { preview, config };
        prepared.check(request)?;
        Ok(Some(prepared))
    }

    fn check_backend_choice_request(
        &self,
        store: &ConfigStore,
        op: OperationId,
        base: BaseRef,
        request: &BackendChoiceRequest,
    ) -> Result<(), SubmitError> {
        request.validate().map_err(|reason| SubmitError::Rejected {
            reason: reason.into(),
        })?;
        if self.snapshot().startup_error.is_some() || self.snapshot().storage_fault.is_some() {
            return Err(SubmitError::Unavailable);
        }
        if store.journal.is_retired(op) {
            return Err(SubmitError::Expired);
        }
        if op.0 == 0 || op.0 > store.journal.issued_through {
            return Err(SubmitError::Rejected {
                reason: "operation_not_allocated".into(),
            });
        }
        if store.journal.has_unknown()
            || store
                .journal
                .replacement
                .as_ref()
                .is_some_and(|w| w.retained())
        {
            return Err(SubmitError::Busy);
        }
        if base.revision != store.config_revision {
            return Err(SubmitError::Conflict {
                field: "backend_choice_config_base".into(),
            });
        }
        let revision = store
            .journal
            .backend_choice
            .as_ref()
            .map_or(0, |r| r.revision);
        if request.expected_revision != revision {
            return Err(SubmitError::Conflict {
                field: "backend_choice_revision".into(),
            });
        }
        if revision == u64::MAX {
            return Err(SubmitError::Rejected {
                reason: "backend_choice_revision_exhausted".into(),
            });
        }
        // Never acknowledge new Zygisk publication until its managed projection
        // is actually prepared. This kernel slice does not create a consent grant.
        if matches!(
            request.choice,
            BackendChoice::Fixed {
                backend: NativeBackend::Zygisk
            }
        ) {
            return Err(SubmitError::Rejected {
                reason: "zygisk_projection_preparation_required".into(),
            });
        }
        Ok(())
    }

    pub(super) fn execute_backend_choice(
        &self,
        op: OperationId,
        base: BaseRef,
        request: BackendChoiceRequest,
        admitted: Option<crate::command_engine::AdmissionReply>,
        prepared: Option<PreparedBackendChoice>,
    ) -> Result<Accepted, SubmitError> {
        let mut store = self.engine.store.lock().expect("config mutex").clone();
        let intent = journal::CommandIntent::StageBackendChoice {
            request: request.clone(),
        };
        if let Some(entry) = store.journal.get(op) {
            return if entry.base == base && entry.command.as_ref() == Some(&intent) {
                Ok(Accepted { op })
            } else {
                Err(SubmitError::IdReused)
            };
        }
        self.check_backend_choice_request(&store, op, base, &request)?;
        let prepared = prepared.ok_or(SubmitError::Rejected {
            reason: "backend_choice_preparation_required".into(),
        })?;
        prepared.check(&request)?;
        if store.config.as_ref() != Some(&prepared.config) {
            return Err(SubmitError::Conflict {
                field: "backend_choice_config_changed".into(),
            });
        }
        let handle = self.native_handle().ok_or(SubmitError::Unavailable)?;
        let boot = handle.lock().expect("native boot");
        if boot.selected() == Some(NativeBackend::Zygisk)
            || crate::native_boot::read_boot_id(&boot.env.boot_id_path) != prepared.preview.boot_id
        {
            return Err(SubmitError::Rejected {
                reason: "backend_choice_boot_or_recipient_changed".into(),
            });
        }
        drop(boot);
        crate::native_boot::prepared::validate_bundle_binding(Some(&prepared.preview.binding))
            .map_err(|_| SubmitError::Rejected {
                reason: "backend_choice_bundle_changed".into(),
            })?;
        let path = self.journal_path.as_ref().ok_or(SubmitError::Unavailable)?;
        store.journal.upsert(JournalEntry {
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
                phases: vec![PhaseRecord {
                    phase: ConfigPhase::Maintenance,
                    outcome: PhaseOutcome::Running,
                }],
                settled: None,
                ports: None,
            },
        });
        let previous = self.engine.store.lock().expect("config mutex").clone();
        if let Err(error) = journal::write_atomic(path, &store.journal) {
            self.freeze_storage_fault(&previous, op, &error);
            return Err(SubmitError::Unavailable);
        }
        *self.engine.store.lock().expect("config mutex") = store.clone();
        self.publish_store(&store);
        if let Some(reply) = admitted {
            let _ = reply.send(Ok(Accepted { op }));
        }
        crate::recovery::maybe_crash_for_test("backend-choice-intent");
        let accepted = store.clone();
        store.journal.backend_choice = Some(journal::backend_choice::BackendChoiceRecord {
            revision: request.expected_revision + 1,
            request,
            operation: op,
            requested_in_boot: prepared.preview.boot_id,
        });
        let entry = store.journal.get_mut(op).expect("choice intent admitted");
        entry.record.phases[0].outcome = PhaseOutcome::Confirmed;
        entry.record.settled = Some(PhaseOutcome::Confirmed);
        store.journal.prune();
        // The preference and its confirmation share one atomic document. A
        // landed rename cannot leave a new choice with an old Running receipt.
        if let Err(error) = journal::write_atomic(path, &store.journal) {
            self.freeze_storage_fault(&accepted, op, &error);
            return Ok(Accepted { op });
        }
        crate::recovery::maybe_crash_for_test("backend-choice-commit");
        *self.engine.store.lock().expect("config mutex") = store.clone();
        self.publish_store(&store);
        Ok(Accepted { op })
    }
}

impl PreparedBackendChoice {
    fn check(&self, request: &BackendChoiceRequest) -> Result<(), SubmitError> {
        if matches!(
            self.preview.ownership,
            Ownership::Unknown | Ownership::Foreign
        ) {
            return Err(SubmitError::Rejected {
                reason: "backend_choice_ownership_unknown".into(),
            });
        }
        if self.preview.ownership == Ownership::Resident(NativeBackend::Builtin)
            && matches!(request.choice, BackendChoice::Fixed { backend } if backend != NativeBackend::Builtin)
        {
            return Err(SubmitError::Rejected {
                reason: "builtin_backend_requires_kernel_change".into(),
            });
        }
        let eligible = self.preview.assessment.candidates.iter().any(|c| {
            let wanted = match request.choice {
                BackendChoice::Auto => {
                    matches!(c.backend, NativeBackend::Builtin | NativeBackend::Kmod)
                }
                BackendChoice::Fixed { backend } => c.backend == backend,
            };
            wanted && c.eligibility.is_ok()
        });
        if !eligible {
            return Err(SubmitError::Rejected {
                reason: "backend_choice_not_eligible_for_complete_policy".into(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
