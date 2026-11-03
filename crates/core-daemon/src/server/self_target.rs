//! Managed diagnostic target preparation is a journaled Core operation, including
//! boot without an APK opening. No legacy cleanup or backend selection is hidden here.
use super::*;
use vpnhide_migration_policy::{APP_PACKAGE, ImportMode, apply, has_user_apps, with_self_target};

impl CoreState {
    /// Run after PM readiness, before the first backend preparation freezes policy.
    /// Only persistence is dispatched; the existing boot Native/Ports passes then
    /// deliver the resulting config and retain their own evidence.
    pub fn prepare_boot_self_targets(&self) -> Result<Option<OperationId>, SubmitError> {
        // Only the production app endpoint configures autonomous execution.
        // A host/dev socket must not manufacture Android user configuration.
        if !self.diagnostics.configured() {
            return Ok(None);
        }
        let snapshot = self.snapshot();
        let base = BaseRef {
            revision: snapshot.config_revision,
        };
        let current = snapshot.config;
        if current.as_ref().is_some_and(|c| {
            has_user_apps(c, APP_PACKAGE) && with_self_target(c.clone(), APP_PACKAGE) == *c
        }) {
            return Ok(None);
        }
        let op = self.issue_operation_id()?;
        let prepared = self.prepare_maintenance(op, &CoreCommand::PrepareSelfTargets { base })?;
        self.call_writer(move |state| state.execute_self_targets(op, base, prepared, true, None))??;
        let record = self
            .get_operation_record(op)
            .ok_or(SubmitError::Unavailable)?;
        if record.settled != Some(PhaseOutcome::Confirmed) {
            // No new attempt is issued for an uncertain persist. Existing journal
            // holds/readback remain the recovery authority.
            return Err(SubmitError::Unavailable);
        }
        Ok(Some(op))
    }

    pub(super) fn execute_self_targets(
        &self,
        op: OperationId,
        base: BaseRef,
        prepared: Option<crate::maintenance::PreparedMaintenance>,
        persist_only: bool,
        admitted: Option<crate::command_engine::AdmissionReply>,
    ) -> Result<Accepted, SubmitError> {
        let store = self.engine.store.lock().expect("config mutex").clone();
        if let Some(entry) = store.journal.get(op) {
            return if entry.base == base
                && entry.command == Some(journal::CommandIntent::PrepareSelfTargets)
            {
                Ok(Accepted { op })
            } else {
                Err(SubmitError::IdReused)
            };
        }
        let current = store.config.clone().unwrap_or_default();
        let next = if has_user_apps(&current, APP_PACKAGE) {
            with_self_target(current.clone(), APP_PACKAGE)
        } else {
            let candidate = &prepared.as_ref().ok_or(SubmitError::Unavailable)?.candidate;
            if !candidate.fully_interpreted() {
                return Err(SubmitError::Rejected {
                    reason: "self-target bootstrap requires complete legacy observations".into(),
                });
            }
            apply(&current, candidate, ImportMode::Merge, APP_PACKAGE)
        };
        let edit = if store.config.is_some() {
            ConfigEdit::Batch {
                edits: config_edit::diff_fields(&current, &next),
            }
        } else {
            ConfigEdit::Replace { config: next }
        };
        // The APK may open while boot waits for PM/loader readiness. Preparing
        // its managed role must not start an independent activation path.
        let persist_only = persist_only
            || self
                .native_handle()
                .is_some_and(|native| native.lock().expect("native boot").selected().is_none());
        self.execute_edit_opts(
            op,
            base,
            &edit,
            SubmitOptions {
                plan: persist_only.then_some(PlanKind::PersistOnly),
                ..Default::default()
            },
            CommandSpec {
                intent: journal::CommandIntent::PrepareSelfTargets,
                admitted,
                maintenance: prepared.map(|p| p.plan),
                ..Default::default()
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn state(root: &std::path::Path) -> Arc<CoreState> {
        let state = CoreState::load_with_journal(
            Publisher::new(7, Default::default()),
            root.join("config.json"),
            Some(root.join("journal.json")),
            None,
        )
        .unwrap();
        let legacy = root.join("legacy");
        fs::create_dir_all(legacy.join("data/adb/vpnhide_kmod")).unwrap();
        state.set_effect_paths(Some(crate::effects::EffectPaths { root: legacy }));
        state.configure_autonomous_diagnostics(root.join("app"));
        state
    }

    #[test]
    fn captured_legacy_cannot_resurrect_configuration_after_a_concurrent_reset() {
        let root = tempfile::tempdir().unwrap();
        let state = state(root.path());
        fs::write(
            root.path().join("legacy/data/adb/vpnhide_kmod/targets.txt"),
            "com.old.app\n",
        )
        .unwrap();
        let base = BaseRef {
            revision: state.config_revision(),
        };
        let op = state.issue_operation_id().unwrap();
        let prepared = state
            .prepare_maintenance(op, &CoreCommand::PrepareSelfTargets { base })
            .unwrap();
        let reset = state.issue_operation_id().unwrap();
        state.submit_command(reset, CoreCommand::FullReset).unwrap();
        assert_eq!(
            state.get_operation_record(reset).unwrap().settled,
            Some(PhaseOutcome::Confirmed)
        );
        let result = state
            .call_writer(move |s| s.execute_self_targets(op, base, prepared, true, None))
            .unwrap();
        assert!(matches!(result, Err(SubmitError::Conflict { .. })));
        assert!(!state.has_config());
        assert!(!root.path().join("config.json").exists());
        state.shutdown();
    }

    #[test]
    fn a_future_base_is_refused_before_preparing_legacy_sources() {
        let root = tempfile::tempdir().unwrap();
        let state = state(root.path());
        let op = state.issue_operation_id().unwrap();
        let future = BaseRef {
            revision: state.config_revision() + 1,
        };
        assert!(matches!(
            state.prepare_maintenance(op, &CoreCommand::PrepareSelfTargets { base: future }),
            Err(SubmitError::Conflict { .. })
        ));
        assert!(state.get_operation_record(op).is_none());
        assert!(!state.has_config());
        state.shutdown();
    }

    #[test]
    fn captured_bootstrap_yields_to_fresh_configured_user_choices() {
        let root = tempfile::tempdir().unwrap();
        let state = state(root.path());
        state.prepare_boot_self_targets().unwrap();
        fs::write(
            root.path().join("legacy/data/adb/vpnhide_kmod/targets.txt"),
            "com.old.app\n",
        )
        .unwrap();
        let base = BaseRef {
            revision: state.config_revision(),
        };
        let op = state.issue_operation_id().unwrap();
        let prepared = state
            .prepare_maintenance(op, &CoreCommand::PrepareSelfTargets { base })
            .unwrap();
        let edit = state.issue_operation_id().unwrap();
        state
            .submit_edit_public(
                edit,
                base,
                &ConfigEdit::App {
                    pkg: "com.fresh.app".into(),
                    field: vpnhide_core_rpc::ConfigAppField::Java,
                    value: vpnhide_core_rpc::AppConfig {
                        java: true,
                        ..Default::default()
                    },
                },
            )
            .unwrap();
        state
            .call_writer(move |s| s.execute_self_targets(op, base, prepared, true, None))
            .unwrap()
            .unwrap();
        let config = state.snapshot().config.unwrap();
        assert!(config.apps["com.fresh.app"].java);
        assert!(!config.apps.contains_key("com.old.app"));
        state.shutdown();
    }
}
