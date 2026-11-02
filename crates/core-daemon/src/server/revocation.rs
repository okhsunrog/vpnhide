//! Narrow missing-config recovery. It cannot construct policy, select a backend,
//! publish a grant or clear unrelated Native/Cleanup/Secret uncertainty.
use super::*;
impl CoreState {
    pub(super) fn execute_revocation_repair(
        &self,
        op: OperationId,
        base: BaseRef,
        mut admitted: Option<crate::command_engine::AdmissionReply>,
        mut store: ConfigStore,
    ) -> Result<Accepted, SubmitError> {
        let path = self
            .journal_path
            .as_deref()
            .ok_or(SubmitError::Unavailable)?;
        let pending = store
            .journal
            .zygisk_revocation
            .as_ref()
            .filter(|r| r.uncertain())
            .cloned()
            .ok_or(SubmitError::Unavailable)?;
        let ticket = self.dispatch_fence.fetch_add(1, Ordering::SeqCst) + 1;
        store.journal.upsert(JournalEntry {
            maintenance: None,
            maintenance_report: None,
            admission: Some(journal::Admission {
                from_revision: store.config_revision,
                commit_revision: store.config_revision,
                writes: vec![],
            }),
            command: Some(journal::CommandIntent::Edit),
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
                    phase: ConfigPhase::Native,
                    outcome: PhaseOutcome::Running,
                }],
                settled: None,
                ports: None,
            },
        });
        let boot = self
            .native_handle()
            .map(|n| {
                crate::native_boot::read_boot_id(&n.lock().expect("native boot").env.boot_id_path)
            })
            .unwrap_or_else(|| {
                crate::native_boot::read_boot_id(std::path::Path::new(
                    crate::native_boot::BOOT_ID_PATH,
                ))
            });
        let mut persisted = false;
        let result = crate::native_boot::transition::revoke(
            &mut store.journal,
            &pending.authority,
            Some(op),
            &boot,
            &mut |journal| {
                self.persist_transition_journal(path, journal, op)?;
                if !persisted {
                    crate::recovery::maybe_crash_for_test("revocation-admission");
                }
                persisted = true;
                if let Some(reply) = admitted.take() {
                    let _ = reply.send(Ok(Accepted { op }));
                }
                Ok(())
            },
            || self.dispatch_fence.load(Ordering::SeqCst) == ticket,
        );
        let outcome = result.err().unwrap_or(PhaseOutcome::Confirmed);
        let record = &mut store
            .journal
            .get_mut(op)
            .expect("revocation repair admitted")
            .record;
        record.phases[0].outcome = outcome;
        record.settled = Some(outcome);
        if self.snapshot().storage_fault.is_none() {
            if let Err(error) = journal::write_atomic(path, &store.journal) {
                let durable = self.engine.store.lock().expect("config mutex").clone();
                self.freeze_storage_fault(&durable, op, &error);
                return if persisted {
                    Ok(Accepted { op })
                } else {
                    Err(SubmitError::Unavailable)
                };
            }
            *self.engine.store.lock().expect("config mutex") = store.clone();
            self.publish_store(&store);
            if let Some(reply) = admitted.take() {
                let _ = reply.send(Ok(Accepted { op }));
            }
        } else if !persisted {
            return Err(SubmitError::Unavailable);
        }
        self.publisher.update(|snapshot| {
            snapshot.operations_in_flight.retain(|id| *id != op);
            snapshot.mutation_held =
                snapshot.storage_fault.is_some() || store.journal.has_unknown();
        });
        Ok(Accepted { op })
    }
}
