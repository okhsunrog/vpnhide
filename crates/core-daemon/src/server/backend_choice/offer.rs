//! Upgrade offer is a retained Core fact, never a diagnostic fallback or effect.
use super::*;
use vpnhide_core_rpc::{BackendChoice, BackendMigrationOffer, NativeBackend};

pub(crate) fn valid_offer(journal: &Journal) -> bool {
    for entry in &journal.operations {
        if let Some(journal::CommandIntent::AcknowledgeBackendOffer { source_commit, .. }) =
            &entry.command
            && (journal.version < 20
                || source_commit.len() != 40
                || !source_commit
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                || entry.record.settled != Some(PhaseOutcome::Confirmed)
                || entry.record.phases.len() != 1
                || entry.record.phases[0].phase != ConfigPhase::Maintenance
                || entry.record.phases[0].outcome != PhaseOutcome::Confirmed
                || entry.candidate.is_some()
                || entry.native_intent.is_some()
                || entry.admission.as_ref().is_none_or(|admission| {
                    admission.from_revision != entry.base.revision
                        || admission.commit_revision != entry.base.revision
                        || !admission.writes.is_empty()
                }))
        {
            return false;
        }
    }
    let Some(offer) = &journal.backend_offer else {
        return true;
    };
    if journal.version < 20
        || offer.source_commit.len() != 40
        || !offer
            .source_commit
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || !matches!(
            offer.target,
            BackendChoice::Fixed {
                backend: NativeBackend::Builtin | NativeBackend::Kmod
            }
        )
        || (offer.kept && !offer.displayed)
    {
        return false;
    }
    offer.operation.is_none_or(|operation| {
        journal.get(operation).is_some_and(|entry| {
            entry.record.settled == Some(PhaseOutcome::Confirmed)
                && entry.command
                    == Some(journal::CommandIntent::AcknowledgeBackendOffer {
                        source_commit: offer.source_commit.clone(),
                        keep: offer.kept,
                    })
        })
    })
}
impl CoreState {
    pub(crate) fn prepare_backend_offer(&self) -> Result<(), String> {
        if self.snapshot().startup_error.is_some() || self.snapshot().storage_fault.is_some() {
            return Ok(());
        }
        let store = self.engine.store.lock().expect("config mutex").clone();
        if store.journal.has_unknown()
            || store
                .journal
                .zygisk_withdrawal
                .as_ref()
                .is_some_and(|work| work.retained())
        {
            return Ok(());
        }
        let Some(source_commit) = upgrade_source(&store.journal) else {
            return Ok(());
        };
        if store
            .journal
            .backend_offer
            .as_ref()
            .is_some_and(|offer| offer.source_commit == source_commit)
        {
            return Ok(());
        }
        let config = store
            .config
            .clone()
            .ok_or("migration offer needs current config")?;
        let Some(handle) = self.native_handle() else {
            return Ok(());
        };
        let boot = handle.lock().expect("native boot");
        if boot.selected() != Some(NativeBackend::Zygisk) {
            return Ok(());
        }
        let env = boot.env.clone();
        drop(boot);
        let current_boot = crate::native_boot::read_boot_id(&env.boot_id_path);
        let Some(authority) = &store.journal.zygisk_authority else {
            return Ok(());
        };
        if authority
            .reference()
            .is_none_or(|reference| reference.boot_id != current_boot)
        {
            return Ok(());
        }
        if !store.journal.operations.iter().any(|entry| {
            entry.candidate.as_ref() == Some(&config)
                && entry.native_intent.as_ref() == Some(&authority.intent)
                && entry.record.phases.iter().any(|phase| {
                    phase.phase == ConfigPhase::Native && phase.outcome == PhaseOutcome::Confirmed
                })
        }) {
            return Ok(());
        }
        let recipient = authority
            .intent
            .zygisk_module
            .as_ref()
            .ok_or("migration offer recipient missing")?;
        crate::zygisk::ZygiskTarget::open(recipient)?.require_authorization(authority)?;
        let packages = vpnhide_activator::PackageUidMap::from_pm_before(
            std::time::Instant::now() + std::time::Duration::from_secs(5),
            || false,
        )
        .map_err(|e| e.to_string())?;
        let preview = crate::native_boot::prepared::preview_choice(&env, &config, &packages)
            .map_err(|e| e.reason)?;
        let target =
            [NativeBackend::Builtin, NativeBackend::Kmod]
                .into_iter()
                .find(|backend| {
                    preview.assessment.candidates.iter().any(|candidate| {
                        candidate.backend == *backend && candidate.eligibility.is_ok()
                    })
                });
        let Some(backend) = target else {
            return Ok(());
        };
        self.call_writer(move |state| {
            let mut current = state.engine.store.lock().expect("config mutex").clone();
            if current.config != store.config
                || current.config_revision != store.config_revision
                || current.journal.replacement != store.journal.replacement
                || current.journal.zygisk_projection != store.journal.zygisk_projection
                || current.journal.zygisk_withdrawal != store.journal.zygisk_withdrawal
                || current.journal.zygisk_authority != store.journal.zygisk_authority
                || current.journal.backend_choice != store.journal.backend_choice
                || current.journal.has_unknown()
                || current
                    .journal
                    .backend_offer
                    .as_ref()
                    .is_some_and(|offer| offer.source_commit == source_commit)
            {
                return Ok(());
            }
            crate::native_boot::prepared::validate_bundle_binding(Some(&preview.binding))?;
            if crate::native_boot::read_boot_id(&env.boot_id_path) != current_boot {
                return Err("migration offer boot changed".into());
            }
            current.journal.backend_offer = Some(BackendMigrationOffer {
                source_commit,
                target: BackendChoice::Fixed { backend },
                displayed: false,
                kept: false,
                operation: None,
            });
            let path = state
                .journal_path
                .as_ref()
                .ok_or("migration offer needs durable journal")?;
            journal::write_atomic(path, &current.journal).map_err(|e| e.to_string())?;
            *state.engine.store.lock().expect("config mutex") = current.clone();
            state.publish_store(&current);
            Ok(())
        })
        .map_err(|e| format!("migration offer writer unavailable: {e:?}"))?
    }

    pub(crate) fn acknowledge_backend_offer(
        &self,
        op: OperationId,
        base: BaseRef,
        source_commit: String,
        keep: bool,
    ) -> Result<Accepted, SubmitError> {
        let mut store = self.engine.store.lock().expect("config mutex").clone();
        let intent = journal::CommandIntent::AcknowledgeBackendOffer {
            source_commit: source_commit.clone(),
            keep,
        };
        if let Some(entry) = store.journal.get(op) {
            return if entry.command.as_ref() == Some(&intent) && entry.base == base {
                Ok(Accepted { op })
            } else {
                Err(SubmitError::IdReused)
            };
        }
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
        if base.revision > store.config_revision {
            return Err(SubmitError::Conflict {
                field: "backend_offer_config_base".into(),
            });
        }
        let offer = store
            .journal
            .backend_offer
            .as_mut()
            .filter(|offer| offer.source_commit == source_commit)
            .ok_or(SubmitError::Rejected {
                reason: "backend_offer_not_current".into(),
            })?;
        if offer.kept && !keep {
            return Err(SubmitError::Rejected {
                reason: "backend_offer_already_kept".into(),
            });
        }
        offer.displayed = true;
        offer.kept |= keep;
        offer.operation = Some(op);
        store.journal.upsert(JournalEntry {
            admission: Some(journal::Admission {
                from_revision: base.revision,
                commit_revision: base.revision,
                writes: vec![],
            }),
            command: Some(intent),
            maintenance: None,
            maintenance_report: None,
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
                    outcome: PhaseOutcome::Confirmed,
                }],
                settled: Some(PhaseOutcome::Confirmed),
                ports: None,
            },
        });
        let mut reply = None;
        let mut accepted = false;
        let mut previous = self.engine.store.lock().expect("config mutex").clone();
        self.persist_choice_store(&mut previous, store, op, &mut reply, &mut accepted)
            .map_err(|_| SubmitError::Unavailable)?;
        Ok(Accepted { op })
    }
}

pub(crate) fn upgrade_source(journal: &Journal) -> Option<String> {
    let published = journal.zygisk_projection.as_ref()?;
    if let Some(replacement) = &journal.replacement {
        return (replacement.status.stage == vpnhide_core_rpc::CoreMaintenanceStage::Verified
            && replacement.projection.is_some()
            && published.descriptor.source_commit != replacement.request.expected_commit)
            .then(|| replacement.request.expected_commit.clone());
    }
    let origin = published.install_continuity.as_ref()?;
    let old = crate::payloads::Manifest::parse(origin.intent.previous_manifest.as_bytes()).ok()?;
    (origin.valid(published) && old.bundle_commit != published.descriptor.source_commit)
        .then(|| published.descriptor.source_commit.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpnhide_core_rpc::{CoreCommand, StateSnapshot};
    fn fixture() -> (tempfile::TempDir, Arc<CoreState>) {
        let root = tempfile::tempdir().unwrap();
        let state = CoreState::load_with_journal(
            Publisher::new(1, StateSnapshot::default()),
            root.path().join("config.json"),
            Some(root.path().join("journal.json")),
            None,
        )
        .unwrap();
        let path = root.path().join("journal.json");
        state
            .call_writer(move |state| {
                let mut store = state.engine.store.lock().unwrap().clone();
                store.journal.backend_offer = Some(BackendMigrationOffer {
                    source_commit: "a".repeat(40),
                    target: BackendChoice::Fixed {
                        backend: NativeBackend::Kmod,
                    },
                    displayed: false,
                    kept: false,
                    operation: None,
                });
                journal::write_atomic(&path, &store.journal).unwrap();
                *state.engine.store.lock().unwrap() = store.clone();
                state.publish_store(&store);
            })
            .unwrap();
        (root, state)
    }
    #[test]
    fn offer_display_and_keep_are_atomic_idempotent_and_never_backend_commands() {
        let (root, state) = fixture();
        for keep in [false, true] {
            let op = state.issue_operation_id().unwrap();
            let command = CoreCommand::AcknowledgeBackendOffer {
                base: BaseRef { revision: 0 },
                source_commit: "a".repeat(40),
                keep,
            };
            state.submit_command(op, command.clone()).unwrap();
            state.submit_command(op, command).unwrap();
            let mut saved = journal::read(&root.path().join("journal.json")).unwrap();
            saved.retain_settled = 0;
            saved.prune();
            assert!(saved.get(op).is_some());
            let offer = saved.backend_offer.as_ref().unwrap();
            assert!(offer.displayed);
            assert_eq!(offer.kept, keep);
            assert_eq!(offer.operation, Some(op));
            assert!(saved.backend_choice.is_none());
            assert!(saved.zygisk_projection.is_none());
            assert!(saved.zygisk_authority.is_none());
            assert!(!root.path().join("config.json").exists());
            assert!(!root.path().join("data/adb/dispatch-count").exists());
            saved.version = 19;
            let invalid = root.path().join("old-schema.json");
            journal::write_atomic(&invalid, &saved).unwrap();
            assert!(journal::read(&invalid).is_err());
        }
        let old = state.issue_operation_id().unwrap();
        assert!(
            state
                .submit_command(
                    old,
                    CoreCommand::AcknowledgeBackendOffer {
                        base: BaseRef { revision: 0 },
                        source_commit: "b".repeat(40),
                        keep: false
                    }
                )
                .is_err()
        );
        assert!(state.get_operation_record(old).is_none());
        state.shutdown();
        let restored = CoreState::load_with_journal(
            Publisher::new(2, StateSnapshot::default()),
            root.path().join("config.json"),
            Some(root.path().join("journal.json")),
            None,
        )
        .unwrap();
        assert!(
            restored
                .engine
                .store
                .lock()
                .unwrap()
                .journal
                .backend_offer
                .as_ref()
                .unwrap()
                .kept
        );
        restored.shutdown();
    }
    #[test]
    fn lost_offer_ack_write_is_recovered_without_replaying_a_backend_effect() {
        for after in [false, true] {
            let (root, state) = fixture();
            let op = state.issue_operation_id().unwrap();
            let later = state.issue_operation_id().unwrap();
            state
                .call_writer(move |state| {
                    journal::fail_write(1, after);
                    assert!(
                        state
                            .acknowledge_backend_offer(
                                op,
                                BaseRef { revision: 0 },
                                "a".repeat(40),
                                false
                            )
                            .is_err()
                    );
                })
                .unwrap();
            let saved = journal::read(&root.path().join("journal.json")).unwrap();
            assert_eq!(saved.backend_offer.as_ref().unwrap().displayed, after);
            assert_eq!(saved.get(op).is_some(), after);
            let before = std::fs::read(root.path().join("journal.json")).unwrap();
            assert_eq!(
                state.submit_command(
                    later,
                    CoreCommand::AcknowledgeBackendOffer {
                        base: BaseRef { revision: 0 },
                        source_commit: "a".repeat(40),
                        keep: true
                    }
                ),
                Err(SubmitError::Unavailable)
            );
            assert_eq!(
                std::fs::read(root.path().join("journal.json")).unwrap(),
                before
            );
            assert!(saved.backend_choice.is_none());
            assert!(saved.zygisk_authority.is_none());
            state.shutdown();
        }
    }
}
