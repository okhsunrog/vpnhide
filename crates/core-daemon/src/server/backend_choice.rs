//! Next-boot device-local family intent on the existing durable writer. Recording
//! an intent does not claim runtime delivery or bypass a native lifecycle hold.
use super::*;
use crate::native_boot::prepared::{ChoicePreview, preview_choice};
use crate::payloads::selection::Ownership;
use vpnhide_core_rpc::{
    BackendChoice, BackendChoiceRequest, BackendChoiceStage, BackendChoiceStatus, NativeBackend,
};

pub(crate) struct PreparedBackendChoice {
    pub(super) bootstrap_resolution:
        Option<super::projection_boot::resolution::withdrawal::PreparedBootstrapWithdrawal>,
    pub(super) preview: ChoicePreview,
    pub(super) config: CanonicalConfig,
    pub(super) publication: Option<crate::maintenance::zygisk_projection::PreparedProjection>,
    pub(super) withdrawal:
        Option<crate::maintenance::zygisk_projection::withdrawal::PreparedWithdrawal>,
}

pub(super) fn status(journal: &Journal) -> BackendChoiceStatus {
    let record = journal.backend_choice.as_ref();
    BackendChoiceStatus {
        migration_offer: journal.backend_offer.clone().filter(|offer| {
            !journal.has_unknown()
                && journal
                    .replacement
                    .as_ref()
                    .is_none_or(|replacement| !replacement.retained())
                && journal
                    .projection_identity()
                    .is_some_and(|(descriptor, _, _)| {
                        descriptor.source_commit == offer.source_commit
                    })
                && record.is_some_and(|record| {
                    record.request.choice
                        == (BackendChoice::Fixed {
                            backend: NativeBackend::Zygisk,
                        })
                })
        }),
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
        BackendChoice::Fixed {
            backend: NativeBackend::Zygisk,
        },
    ]
}

impl CoreState {
    pub(super) fn projection_paths(
        &self,
    ) -> Result<crate::maintenance::zygisk_projection::ProjectionPaths, SubmitError> {
        Ok(crate::maintenance::zygisk_projection::ProjectionPaths {
            root: self
                .effect_paths
                .read()
                .expect("effect paths")
                .as_ref()
                .ok_or(SubmitError::Unavailable)?
                .root
                .clone(),
            persistent: self
                .journal_path
                .as_deref()
                .and_then(std::path::Path::parent)
                .ok_or(SubmitError::Unavailable)?
                .into(),
        })
    }
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
        let (config, journal) = {
            let store = self.engine.store.lock().expect("config mutex");
            if store.journal.get(op).is_some() {
                return Ok(None);
            }
            self.check_backend_choice_request(&store, op, *base, request)?;
            (
                store.config.clone().ok_or(SubmitError::Unavailable)?,
                store.journal.clone(),
            )
        };
        let publishing = request.choice
            == (BackendChoice::Fixed {
                backend: NativeBackend::Zygisk,
            });
        let handle = self.native_handle().ok_or(SubmitError::Unavailable)?;
        let boot = handle.lock().expect("native boot");
        let following = journal
            .zygisk_withdrawal
            .as_ref()
            .is_some_and(|work| work.outcome == PhaseOutcome::Confirmed);
        let withdrawing = !following
            && journal
                .zygisk_projection
                .as_ref()
                .is_some_and(|work| work.outcome == PhaseOutcome::Confirmed);
        if boot.selected() == Some(NativeBackend::Zygisk) && !withdrawing && !publishing {
            return Err(SubmitError::Rejected {
                reason: "zygisk_retirement_preparation_required".into(),
            });
        }
        let env = boot.env.clone();
        drop(boot);
        if following {
            crate::maintenance::zygisk_projection::withdrawal::followup::require_proof(
                &journal, &env,
            )
            .map_err(|reason| SubmitError::Rejected { reason })?;
        }
        let bootstrap_resolution = if super::projection_boot::resolution::withdrawal::can_withdraw(
            &journal, request,
        ) {
            Some(
                    super::projection_boot::resolution::withdrawal::PreparedBootstrapWithdrawal::prepare(
                        &journal, &env, op,
                    )
                    .map_err(|reason| SubmitError::Rejected { reason })?,
                )
        } else {
            None
        };
        let packages =
            vpnhide_activator::PackageUidMap::from_pm_before(deadline, || cancelled.is_cancelled())
                .map_err(|_| SubmitError::Rejected {
                    reason: "backend_choice_inventory_unavailable".into(),
                })?;
        let publication = if publishing && following {
            let proof = crate::maintenance::zygisk_projection::withdrawal::followup::require_proof(
                &journal, &env,
            )
            .map_err(|reason| SubmitError::Rejected { reason })?;
            Some(crate::maintenance::zygisk_projection::PreparedProjection::prepare_after_withdrawal(self.projection_paths()?, op.0, &proof)
                .map_err(|error| SubmitError::Rejected { reason: format!("zygisk_reentry_preparation: {error}") })?)
        } else if publishing {
            Some(
                crate::maintenance::zygisk_projection::PreparedProjection::prepare(
                    self.projection_paths()?,
                    op.0,
                )
                .map_err(|e| SubmitError::Rejected {
                    reason: format!("zygisk_publication_preparation: {e}"),
                })?,
            )
        } else {
            None
        };
        let preview = if let Some((root, legacy)) = publication
            .as_ref()
            .and_then(|publication| publication.legacy_context())
        {
            crate::native_boot::prepared::preview_projection_adoption_choice(
                &env, &config, &packages, root, legacy,
            )
        } else {
            let preview_fn = if publishing {
                crate::native_boot::prepared::preview_projection_choice
            } else {
                preview_choice
            };
            preview_fn(&env, &config, &packages)
        }
        .map_err(|e| SubmitError::Rejected {
            reason: format!("backend_choice_unavailable: {}", e.reason),
        })?;
        let withdrawal = if withdrawing {
            Some(
                crate::maintenance::zygisk_projection::withdrawal::PreparedWithdrawal::prepare(
                    self.projection_paths()?,
                    op.0,
                    &journal,
                )
                .map_err(|e| SubmitError::Rejected {
                    reason: format!("zygisk_withdrawal_preparation: {e}"),
                })?,
            )
        } else {
            None
        };
        let prepared = PreparedBackendChoice {
            bootstrap_resolution,
            preview,
            config,
            withdrawal,
            publication,
        };
        prepared.check(request)?;
        Ok(Some(prepared))
    }

    pub(super) fn persist_choice_store(
        &self,
        store: &mut ConfigStore,
        next: ConfigStore,
        op: OperationId,
        reply: &mut Option<crate::command_engine::AdmissionReply>,
        accepted: &mut bool,
    ) -> std::io::Result<()> {
        let path = self
            .journal_path
            .as_ref()
            .ok_or_else(|| std::io::Error::other("journal unavailable"))?;
        if let Err(error) = journal::write_atomic(path, &next.journal) {
            self.freeze_storage_fault(store, op, &error);
            return Err(error);
        }
        *store = next;
        *self.engine.store.lock().expect("config mutex") = store.clone();
        self.publish_store(store);
        *accepted = true;
        if let Some(reply) = reply.take() {
            let _ = reply.send(Ok(Accepted { op }));
        }
        Ok(())
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
        let following = store
            .journal
            .zygisk_withdrawal
            .as_ref()
            .is_some_and(|work| work.outcome == PhaseOutcome::Confirmed);
        if store
            .journal
            .zygisk_projection
            .as_ref()
            .is_some_and(|work| {
                work.retained()
                    && !following
                    && (work.outcome != PhaseOutcome::Confirmed
                        || (store.journal.zygisk_authority.is_none()
                            && store.journal.zygisk_bootstrap.is_none()
                            && !self
                                .effect_paths
                                .read()
                                .expect("effect paths")
                                .as_ref()
                                .is_some_and(|paths| {
                                    self.loader
                                        .owns_projection(&paths.root.join("data/adb/modules"))
                                }))
                        || request.choice
                            == (BackendChoice::Fixed {
                                backend: NativeBackend::Zygisk,
                            }))
            })
            || store
                .journal
                .zygisk_withdrawal
                .as_ref()
                .is_some_and(|work| work.retained() && !following)
            || (store.journal.has_unknown()
                && !super::projection_boot::resolution::withdrawal::can_withdraw(
                    &store.journal,
                    request,
                ))
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
        let mut prepared = prepared.ok_or(SubmitError::Rejected {
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
        if (boot.selected() == Some(NativeBackend::Zygisk)
            && prepared.withdrawal.is_none()
            && prepared
                .publication
                .as_ref()
                .is_none_or(|publication| publication.legacy_context().is_none()))
            || crate::native_boot::read_boot_id(&boot.env.boot_id_path) != prepared.preview.boot_id
        {
            return Err(SubmitError::Rejected {
                reason: "backend_choice_boot_or_recipient_changed".into(),
            });
        }
        let followup_proof = if store
            .journal
            .zygisk_withdrawal
            .as_ref()
            .is_some_and(|work| work.outcome == PhaseOutcome::Confirmed)
        {
            Some(
                crate::maintenance::zygisk_projection::withdrawal::followup::require_proof(
                    &store.journal,
                    &boot.env,
                )
                .map_err(|reason| SubmitError::Rejected { reason })?,
            )
        } else {
            None
        };
        let resolution_env = boot.env.clone();
        drop(boot);
        if let Some(expected) = &prepared.bootstrap_resolution {
            expected
                .require_fresh(&store.journal, &resolution_env)
                .map_err(|reason| SubmitError::Rejected { reason })?;
        } else if store.journal.bootstrap_uncertain() {
            return Err(SubmitError::Rejected {
                reason: "bootstrap_withdrawal_readback_required".into(),
            });
        }
        crate::native_boot::prepared::validate_bundle_binding(Some(&prepared.preview.binding))
            .map_err(|_| SubmitError::Rejected {
                reason: "backend_choice_bundle_changed".into(),
            })?;
        let path = self.journal_path.as_ref().ok_or(SubmitError::Unavailable)?;
        // An imported canonical file starts at revision1 even when the journal
        // still has its initial revision0. Bind the already checked store base
        // in the same atomic document as this command, without rewriting config.
        store.journal.config_revision = store.config_revision;
        if let Some(publication) = prepared.publication.take() {
            publication
                .require_fresh()
                .map_err(|_| SubmitError::Rejected {
                    reason: "zygisk_publication_sources_changed".into(),
                })?;
            let work = publication
                .work(request)
                .map_err(|_| SubmitError::Rejected {
                    reason: "zygisk_publication_intent_invalid".into(),
                })?;
            let mut reply = admitted;
            let mut accepted = false;
            let result = publication.dispatch(work, |work| {
                let mut next = store.clone();
                journal::backend_choice::record_projection(&mut next.journal, base, work)?;
                self.persist_choice_store(&mut store, next, op, &mut reply, &mut accepted)?;
                crate::recovery::maybe_crash_for_test(if !work.dispatch_intent {
                    "projection-accepted"
                } else if work.outcome == PhaseOutcome::Running {
                    "projection-before-dispatch"
                } else {
                    "projection-completed"
                });
                Ok(())
            });
            if result.is_err() && !accepted {
                return Err(SubmitError::Unavailable);
            }
            return Ok(Accepted { op });
        }
        if let Some(withdrawal) = prepared.withdrawal.take() {
            withdrawal
                .require_fresh()
                .map_err(|_| SubmitError::Rejected {
                    reason: "zygisk_withdrawal_sources_changed".into(),
                })?;
            let work = withdrawal
                .work(request)
                .map_err(|_| SubmitError::Rejected {
                    reason: "zygisk_withdrawal_intent_invalid".into(),
                })?;
            let mut reply = admitted;
            let mut accepted = false;
            let result = withdrawal.dispatch(work, |work| {
                let mut next = store.clone();
                journal::backend_choice::record_withdrawal(&mut next.journal, base, work)?;
                if let Some(resolution) = &prepared.bootstrap_resolution {
                    next.journal.bootstrap_resolution = (work.outcome != PhaseOutcome::FailedKnown)
                        .then(|| resolution.receipt.clone());
                }
                self.persist_choice_store(&mut store, next, op, &mut reply, &mut accepted)?;
                crate::recovery::maybe_crash_for_test(if !work.dispatch_intent {
                    "withdrawal-accepted"
                } else if work.outcome == PhaseOutcome::Running {
                    "withdrawal-before-dispatch"
                } else {
                    "withdrawal-completed"
                });
                Ok(())
            });
            if result.is_err() && !accepted {
                return Err(SubmitError::Unavailable);
            }
            return Ok(Accepted { op });
        }
        if request.choice
            == (BackendChoice::Fixed {
                backend: NativeBackend::Zygisk,
            })
        {
            return Err(SubmitError::Rejected {
                reason: "zygisk_projection_preparation_required".into(),
            });
        }
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
        if let Some(expected) = &followup_proof {
            let current = self.native_handle().and_then(|handle| {
                let env = handle.lock().expect("native boot").env.clone();
                crate::maintenance::zygisk_projection::withdrawal::followup::require_proof(
                    &store.journal,
                    &env,
                )
                .ok()
            });
            if current.as_ref() != Some(expected) {
                let accepted = store.clone();
                let entry = store.journal.get_mut(op).expect("choice admitted");
                entry.record.phases[0].outcome = PhaseOutcome::FailedKnown;
                entry.record.settled = Some(PhaseOutcome::FailedKnown);
                if let Err(error) = journal::write_atomic(path, &store.journal) {
                    self.freeze_storage_fault(&accepted, op, &error);
                    return Ok(Accepted { op });
                }
                *self.engine.store.lock().expect("config mutex") = store.clone();
                self.publish_store(&store);
                return Ok(Accepted { op });
            }
        }
        let accepted = store.clone();
        store.journal.backend_choice = Some(journal::backend_choice::BackendChoiceRecord {
            inherited: false,
            revision: request.expected_revision + 1,
            request,
            operation: op,
            requested_in_boot: prepared.preview.boot_id,
        });
        let entry = store.journal.get_mut(op).expect("choice intent admitted");
        entry.record.phases[0].outcome = PhaseOutcome::Confirmed;
        entry.record.settled = Some(PhaseOutcome::Confirmed);
        if let Some(proof) = followup_proof {
            store.journal.withdrawal_followup = Some(
                crate::maintenance::zygisk_projection::withdrawal::followup::WithdrawalFollowup {
                    choice: store
                        .journal
                        .backend_choice
                        .clone()
                        .expect("choice committed"),
                    withdrawal: proof.operation,
                    source: proof.source,
                    image: proof.image,
                },
            );
        }
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

pub(crate) mod offer;
