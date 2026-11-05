//! The new boot's existing writer consumes installer evidence. The installer is
//! never a second canonical/journal writer and cannot create a user warning ack.
use super::*;
use crate::maintenance::zygisk_projection::{
    ActiveProjection, ProjectionPaths, ProjectionStaging, install::InstallReadback,
    work::ProjectionWork,
};
use std::fs::File;
use vpnhide_core_rpc::{BackendChoice, BackendChoiceRequest, NativeBackend};

impl CoreState {
    pub(crate) fn adopt_install_handoff(
        &self,
        env: &crate::native_boot::NativeEnv,
        epoch: u64,
    ) -> Result<(), String> {
        self.adopt_install_handoff_owned(env, epoch, None, 0)
    }
    fn adopt_install_handoff_owned(
        &self,
        env: &crate::native_boot::NativeEnv,
        epoch: u64,
        running: Option<File>,
        uid: u32,
    ) -> Result<(), String> {
        let before = self.engine.store.lock().expect("config mutex").clone();
        if before.journal.zygisk_projection.is_some() {
            return Ok(());
        }
        // Ordinary startup has no installer evidence and needs no installer
        // profile. In particular, a passive/fresh boot must not acquire new
        // namespace requirements before the usual Native preparation.
        let persistent = self
            .journal_path
            .as_deref()
            .and_then(std::path::Path::parent)
            .ok_or("install handoff journal unavailable")?;
        if std::fs::symlink_metadata(persistent.join("install-handoff"))
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        {
            return Ok(());
        }
        let root = self
            .effect_paths
            .read()
            .expect("effect paths")
            .as_ref()
            .ok_or("install handoff profile unavailable")?
            .root
            .clone();
        if env.modules.as_ref() != Some(&root.join("data/adb/modules")) {
            return Err("install handoff namespace mismatch".into());
        }
        let paths = ProjectionPaths {
            root,
            persistent: self
                .journal_path
                .as_deref()
                .and_then(std::path::Path::parent)
                .ok_or("install handoff journal unavailable")?
                .to_path_buf(),
        };
        let running = running
            .map_or_else(|| File::open("/proc/self/exe"), Ok)
            .map_err(|e| e.to_string())?;
        let Some(prepared) = InstallReadback::prepare_owned(&paths, &running, uid)
            .map_err(|e| format!("install handoff readback: {e}"))?
        else {
            return Ok(());
        };
        self.call_writer(move |state| {
            if state.epoch.load(Ordering::SeqCst) != epoch
                || state.snapshot().storage_fault.is_some()
                || state.snapshot().startup_error.is_some()
            {
                return Err("install handoff epoch/storage unavailable".into());
            }
            let mut store = state.engine.store.lock().expect("config mutex").clone();
            if store.journal.zygisk_projection.is_some() {
                return Ok(());
            }
            let config = store
                .config
                .as_ref()
                .ok_or("install handoff needs preserved canonical state")?;
            if store.config != before.config
                || store.config_revision != before.config_revision
                || store.journal != before.journal
                || !prepared.origin.intent.accepts(&store.journal, config)
            {
                return Err("install handoff intent changed or remains held".into());
            }
            prepared
                .require_owned(&paths, &running, uid)
                .map_err(|e| e.to_string())?;
            let op = OperationId(
                store
                    .journal
                    .issued_through
                    .checked_add(1)
                    .ok_or("operation ids exhausted")?,
            );
            let revision = store
                .journal
                .backend_choice
                .as_ref()
                .map_or(0, |choice| choice.revision);
            let next_revision = revision
                .checked_add(1)
                .ok_or("choice revisions exhausted")?;
            let work = ProjectionWork {
                install_continuity: Some(prepared.origin.clone()),
                legacy_adoption: None,
                previous_withdrawal: None,
                operation: op,
                request: BackendChoiceRequest {
                    expected_revision: revision,
                    choice: BackendChoice::Fixed {
                        backend: NativeBackend::Zygisk,
                    },
                    acknowledged_warning: None,
                },
                boot_id: prepared.origin.intent.boot_id.clone(),
                descriptor: prepared.origin.intent.descriptor.clone(),
                source: prepared.source,
                manager: prepared.origin.intent.manager.clone(),
                dispatch_intent: true,
                outcome: PhaseOutcome::Confirmed,
                failure: None,
                staging: Some(ProjectionStaging {
                    staged: None,
                    active: Some(ActiveProjection::Complete {
                        image: Box::new(prepared.image),
                    }),
                }),
            };
            store.journal.issued_through = op.0;
            if !work.valid(op.0) {
                return Err("install handoff ownership is invalid".into());
            }
            store.journal.backend_choice = Some(journal::backend_choice::BackendChoiceRecord {
                inherited: true,
                revision: next_revision,
                request: work.request.clone(),
                operation: op,
                requested_in_boot: work.boot_id.clone(),
            });
            store.journal.zygisk_projection = Some(work);
            store.journal.upsert(JournalEntry {
                admission: Some(journal::Admission {
                    from_revision: store.config_revision,
                    commit_revision: store.config_revision,
                    writes: vec![],
                }),
                command: Some(journal::CommandIntent::AdoptInstallHandoff {
                    intent_sha256: prepared.origin.intent.digest(),
                }),
                maintenance: None,
                maintenance_report: None,
                superseded_by: None,
                superseded_phases: vec![],
                finalized_unknown: false,
                base: BaseRef {
                    revision: store.config_revision,
                },
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
            if !crate::maintenance::zygisk_projection::work::validate_journal(&store.journal)
                || !journal::backend_choice::validate_entries(&store.journal)
            {
                return Err("install handoff receipt linkage is invalid".into());
            }
            let path = state
                .journal_path
                .as_ref()
                .ok_or("install handoff requires durable journal")?;
            if let Err(error) = journal::write_atomic(path, &store.journal) {
                let old = state.engine.store.lock().expect("config mutex").clone();
                state.freeze_storage_fault(&old, op, &error);
                return Err(format!("install handoff journal unavailable: {error}"));
            }
            *state.engine.store.lock().expect("config mutex") = store.clone();
            state.publish_store(&store);
            Ok(())
        })
        .map_err(|e| format!("install handoff writer unavailable: {e:?}"))?
    }
}

#[cfg(test)]
mod tests;
