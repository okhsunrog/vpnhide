//! Boot-thread readback/binding, before any loader worker or native selection.
//! Source hashing stays outside the command writer and main IPC startup path.
use super::*;
use crate::maintenance::zygisk_projection::withdrawal::observation::{
    WithdrawalBootProof, WithdrawalObservation,
};
use crate::maintenance::zygisk_projection::{ProjectionPaths, work::ProjectionObservation};
use std::{fs, io, path::Path};
use vpnhide_core_rpc::{BackendChoice, NativeBackend};
use vpnhide_protocol::zygisk_activation::MANAGED_MODULE_ID;

impl CoreState {
    pub(crate) fn bind_projection_for_boot(
        &self,
        env: &crate::native_boot::NativeEnv,
        epoch: u64,
    ) -> Result<Option<WithdrawalBootProof>, String> {
        if self.epoch.load(Ordering::SeqCst) != epoch {
            return Err("projection boot belongs to an older epoch".into());
        }
        let result = self.inspect_projection_for_boot(env);
        if let Err(reason) = &result {
            let reason = reason.clone();
            let mut env = env.clone();
            env.loader = Some(self.loader.clone());
            let boot = crate::native_boot::read_boot_id(&env.boot_id_path);
            self.call_writer(move |state| -> Result<(), String> {
                if state.epoch.load(Ordering::SeqCst) != epoch {
                    return Err("projection failure belongs to an older Core epoch".into());
                }
                if boot.is_empty() || boot.len() > 128 {
                    return Err("projection boot identity unavailable".into());
                }
                let mut journal = state
                    .engine
                    .store
                    .lock()
                    .expect("config mutex")
                    .journal
                    .clone();
                if !journal.zygisk_exposed_in(&boot) {
                    journal.note_zygisk_exposure(&boot);
                    let path = state
                        .journal_path
                        .as_ref()
                        .ok_or("projection recovery needs durable journal")?;
                    state
                        .persist_transition_journal(path, &journal, OperationId(0))
                        .map_err(|e| e.to_string())?;
                }
                state.install_native_boot(crate::native_boot::NativeBoot::blocked(
                    env, epoch, reason,
                ));
                Ok(())
            })
            .map_err(|e| format!("projection recovery writer unavailable: {e:?}"))??;
        }
        result
    }
    fn inspect_projection_for_boot(
        &self,
        env: &crate::native_boot::NativeEnv,
    ) -> Result<Option<WithdrawalBootProof>, String> {
        self.inspect_projection_for_boot_owned(env, None, 0)
    }
    fn inspect_projection_for_boot_owned(
        &self,
        env: &crate::native_boot::NativeEnv,
        running: Option<&fs::File>,
        uid: u32,
    ) -> Result<Option<WithdrawalBootProof>, String> {
        let Some(modules) = &env.modules else {
            return Ok(None);
        };
        let active = modules.join(MANAGED_MODULE_ID);
        let exists = match fs::symlink_metadata(&active) {
            Ok(_) => true,
            Err(e) if e.kind() == io::ErrorKind::NotFound => false,
            Err(e) => return Err(format!("cannot inspect managed discovery: {e}")),
        };
        let before = self
            .engine
            .store
            .lock()
            .expect("config mutex")
            .journal
            .clone();
        let work = before
            .zygisk_projection
            .as_ref()
            .filter(|work| work.retained());
        if !exists && work.is_none() {
            // A user may change an old autoload Core to Zygisk after installing
            // a passive update. Missing handoff is not permission for Auto to
            // change that already-confirmed family to a kernel backend.
            let latest = before
                .operations
                .iter()
                .filter(|entry| {
                    entry
                        .record
                        .phases
                        .iter()
                        .any(|phase| phase.phase == ConfigPhase::Native)
                })
                .max_by_key(|entry| entry.record.op.0);
            let missing = latest.is_some_and(|entry| {
                entry.record.settled == Some(PhaseOutcome::Confirmed)
                    && entry.candidate.as_ref().is_some_and(
                        crate::maintenance::zygisk_projection::install::has_user_native,
                    )
                    && entry.native_intent.as_ref().is_some_and(|intent| {
                        intent.backend == NativeBackend::Zygisk
                            && intent.zygisk_module.as_ref() == Some(&modules.join("vpnhide_core"))
                    })
                    && !before.backend_choice.as_ref().is_some_and(|choice| {
                        choice.operation.0 > entry.record.op.0
                            && choice.request.choice
                                != (BackendChoice::Fixed {
                                    backend: NativeBackend::Zygisk,
                                })
                    })
            });
            if missing && !modules.join("vpnhide_core/zygisk").exists() {
                return Err("old Zygisk selection has no verified install handoff; choose a backend explicitly".into());
            }
            return Ok(None);
        }
        let work = work.ok_or("managed discovery has no retained Core ownership")?;
        let root = self
            .effect_paths
            .read()
            .expect("effect paths")
            .as_ref()
            .ok_or("projection profile unavailable")?
            .root
            .clone();
        if modules != &root.join("data/adb/modules") {
            return Err("projection namespace differs from Core profile".into());
        }
        let paths = ProjectionPaths {
            root,
            persistent: self
                .journal_path
                .as_deref()
                .and_then(Path::parent)
                .ok_or("projection journal unavailable")?
                .into(),
        };
        let boot = crate::native_boot::read_boot_id(&env.boot_id_path);
        let actual;
        let running = match running {
            Some(file) => file,
            None => {
                actual = fs::File::open("/proc/self/exe").map_err(|e| e.to_string())?;
                &actual
            }
        };
        let withdrawal = before
            .zygisk_withdrawal
            .as_ref()
            .filter(|work| work.retained());
        let (readback, negative) = if let Some(withdrawal) = withdrawal {
            if !journal::backend_choice::validate_entries(&before)
                || !crate::maintenance::zygisk_projection::withdrawal::work::validate_journal(
                    &before,
                )
                || !crate::maintenance::zygisk_projection::work::validate_journal(&before)
            {
                return Err("managed withdrawal journal is unverified".into());
            }
            let mut observed = withdrawal.observe_bound_owned(
                &paths,
                running,
                uid,
                None,
                before.withdrawal_source(withdrawal),
            );
            for authority in before.retained_zygisk_authorities() {
                if observed.is_ok() {
                    break;
                }
                observed = withdrawal.observe_bound_owned(
                    &paths,
                    running,
                    uid,
                    Some(authority),
                    before.withdrawal_source(withdrawal),
                );
            }
            match observed.map_err(|e| format!("managed withdrawal readback: {e}"))? {
                WithdrawalObservation::RetainedActive(active) => (Some(active), None),
                WithdrawalObservation::Withdrawn(proof) => {
                    if before.zygisk_exposed_in(&boot) {
                        return Err("current-boot Zygisk exposure prevents kernel admission".into());
                    }
                    (None, Some(proof))
                }
            }
        } else if let Some(replacement) = &before.replacement
            && let Some(update) = &replacement.projection
        {
            if !update.valid(replacement)
                || !update.linked(&before)
                || (boot != replacement.boot_id
                    && replacement.status.stage != vpnhide_core_rpc::CoreMaintenanceStage::Verified)
            {
                return Err("combined projection update is unverified".into());
            }
            let (descriptor, expected_source, _) = before
                .projection_identity()
                .ok_or("projection identity unavailable")?;
            let source = crate::maintenance::image::ImageFence::inspect(
                &paths.source(),
                None,
                crate::payloads::Abi::Arm64,
                uid,
            )
            .map_err(|e| e.to_string())?;
            if !source.same_content(expected_source) {
                return Err("updated source differs from verified Core replacement".into());
            }
            source
                .require_running_owned(running, uid)
                .map_err(|e| e.to_string())?;
            let image = crate::maintenance::zygisk_projection::ProjectionImage::inspect_code_owned(
                &paths.active(),
                descriptor,
                uid,
            )
            .map_err(|e| e.to_string())?;
            // Establish exact authority presence/absence separately; code-only
            // readback never promotes opaque authorization bytes.
            let target = crate::zygisk::ZygiskTarget::for_revocation(&paths.active())?;
            if !target.authorization_absent()?
                && !before
                    .retained_zygisk_authorities()
                    .any(|authority| target.require_authorization(authority).is_ok())
            {
                return Err("updated recipient has an unowned grant".into());
            }
            if boot == replacement.boot_id {
                update
                    .require_old_owned(&paths.root, uid)
                    .map_err(|e| e.to_string())?;
            }
            if boot != replacement.boot_id
                && replacement.status.stage == vpnhide_core_rpc::CoreMaintenanceStage::Verified
            {
                // New boot can already contain this boot's exact durable grant.
                // Initial no-grant qualification was performed by replacement reconciliation.
                image
                    .descriptor
                    .require_source(
                        &std::fs::read(paths.source().join("payloads/manifest.json"))
                            .map_err(|e| e.to_string())?,
                    )
                    .map_err(|e| e.to_string())?;
            }
            (
                Some(
                    crate::maintenance::zygisk_projection::work::ActiveProjectionReadback {
                        source,
                        image,
                        source_directory: std::sync::Arc::new(
                            fs::File::open(paths.source()).map_err(|e| e.to_string())?,
                        ),
                        recipient_directory: std::sync::Arc::new(
                            fs::File::open(paths.active()).map_err(|e| e.to_string())?,
                        ),
                    },
                ),
                None,
            )
        } else {
            if work.outcome != PhaseOutcome::Confirmed
                || !journal::backend_choice::validate_entries(&before)
                || !crate::maintenance::zygisk_projection::work::validate_journal(&before)
                || before.backend_choice.as_ref().is_none_or(|choice| {
                    choice.operation != work.operation
                        || choice.request.choice
                            != (BackendChoice::Fixed {
                                backend: NativeBackend::Zygisk,
                            })
                })
            {
                return Err("managed discovery preparation is unverified".into());
            }
            let observe = |authority| {
                work.observe_with_authority_owned(&paths, &boot, running, uid, authority)
            };
            let observation = if work.boot_id == boot {
                // A new staged choice never authorizes hooks in its requested boot.
                observe(None)
            } else {
                let mut observed = observe(None);
                for authority in before.retained_zygisk_authorities() {
                    if observed.is_ok() {
                        break;
                    }
                    observed = observe(Some(authority));
                }
                observed
            }
            .map_err(|e| format!("managed discovery source/code ownership: {e}"))?;
            match observation {
                ProjectionObservation::ActiveAfterBoot(readback) => (Some(readback), None),
                ProjectionObservation::PreparedInSameBoot(staging)
                    if work.previous_withdrawal.is_some() =>
                {
                    let cycle = before
                        .projection_history
                        .iter()
                        .find(|cycle| {
                            work.previous_withdrawal.as_ref() == Some(&cycle.tombstone)
                                && cycle.boot_id == boot
                        })
                        .ok_or("reentry predecessor is not retained")?;
                    let staged = staging.staged.ok_or("reentry stage unavailable")?;
                    if work.outcome != PhaseOutcome::Confirmed
                        || staged.descriptor != work.descriptor
                        || before.zygisk_exposed_in(&boot)
                    {
                        return Err("reentry is unverified or exposed".into());
                    }
                    let proof = WithdrawalBootProof {
                        operation: cycle.withdrawal.operation,
                        boot_id: boot.clone(),
                        root: paths.root.clone(),
                        source: work.source.clone(),
                        image: cycle.tombstone.clone(),
                        pending_reentry: Some(Box::new(staged)),
                    };
                    proof
                        .recheck_owned(running, uid)
                        .map_err(|e| e.to_string())?;
                    (None, Some(proof))
                }
                _ => (None, None),
            }
        };
        if let Some(readback) = readback {
            self.loader
                .bind_projection_owned(
                    &paths.source(),
                    &paths.active(),
                    &readback,
                    uid,
                    &boot,
                    self.epoch.load(Ordering::SeqCst),
                )
                .map_err(|e| e.to_string())?;
        }
        let after = self.engine.store.lock().expect("config mutex");
        if after.journal.projection_history != before.projection_history
            || after.journal.withdrawal_followup != before.withdrawal_followup
            || after.journal.replacement != before.replacement
            || after.journal.zygisk_withdrawal != before.zygisk_withdrawal
            || after.journal.zygisk_projection != before.zygisk_projection
            || after.journal.backend_choice != before.backend_choice
            || after.journal.zygisk_authority != before.zygisk_authority
            || after.journal.prior_zygisk_authority != before.prior_zygisk_authority
            || after.journal.zygisk_bootstrap != before.zygisk_bootstrap
            || crate::native_boot::read_boot_id(&env.boot_id_path) != boot
        {
            return Err("projection state changed during readback".into());
        }
        if negative.is_some() && after.journal.zygisk_exposed_in(&boot) {
            return Err("current-boot exposure changed during withdrawal readback".into());
        }
        Ok(negative)
    }
}

mod bootstrap;
pub(crate) mod resolution;
#[cfg(test)]
mod tests;
