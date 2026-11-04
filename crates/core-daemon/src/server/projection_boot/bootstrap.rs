//! Empty-worker publication is a separate writer effect, never ConfigPhase::Native
//! completion. Ordinary kernel startup and same-boot staging create no grant.
use super::*;
use crate::journal::{
    NativeIntent, ZygiskAuthorityRecord, ZygiskAuthorizationIntent,
    projection_bootstrap::ProjectionBootstrap,
};
use crate::native_boot::prepared::{BundleIdentity, validate_bundle_binding_owned};
use crate::payloads::{Catalog, selection::Ownership};
use crate::zygisk::{PublishError, ZygiskTarget};

fn previous_authority(
    journal: &Journal,
    target: &ZygiskTarget,
    path: &Path,
) -> Result<Option<ZygiskAuthorityRecord>, String> {
    if target.authorization_absent()? {
        return Ok(None);
    }
    journal
        .retained_zygisk_authorities()
        .find(|record| {
            record.intent.zygisk_module.as_deref() == Some(path)
                && target.check_authorization(record).is_ok()
        })
        .cloned()
        .map(Some)
        .ok_or_else(|| "managed authorization has no retained exact owner".into())
}
fn same_intent(before: &Journal, current: &Journal) -> bool {
    before.zygisk_projection == current.zygisk_projection
        && before.backend_choice == current.backend_choice
        && before.zygisk_authority == current.zygisk_authority
        && before.prior_zygisk_authority == current.prior_zygisk_authority
        && before.zygisk_bootstrap == current.zygisk_bootstrap
}
impl CoreState {
    pub(crate) fn prepare_projection_bootstrap(
        &self,
        env: &crate::native_boot::NativeEnv,
        epoch: u64,
    ) -> Result<(), String> {
        let result = self.prepare_projection_bootstrap_owned(env, epoch, 0);
        if let Err(reason) = &result {
            let mut env = env.clone();
            env.loader = Some(self.loader.clone());
            let reason = reason.clone();
            self.call_writer(move |state| {
                if state.epoch.load(Ordering::SeqCst) == epoch {
                    state.install_native_boot(crate::native_boot::NativeBoot::blocked(
                        env, epoch, reason,
                    ));
                }
            })
            .map_err(|e| format!("managed bootstrap failure writer unavailable: {e:?}"))?;
        }
        result
    }
    pub(super) fn prepare_projection_bootstrap_owned(
        &self,
        env: &crate::native_boot::NativeEnv,
        epoch: u64,
        uid: u32,
    ) -> Result<(), String> {
        let before = self
            .engine
            .store
            .lock()
            .expect("config mutex")
            .journal
            .clone();
        let Some(work) = before
            .zygisk_projection
            .as_ref()
            .filter(|work| work.retained())
        else {
            return Ok(());
        };
        if self.epoch.load(Ordering::SeqCst) != epoch {
            return Err("managed bootstrap belongs to an older Core epoch".into());
        }
        let boot = crate::native_boot::read_boot_id(&env.boot_id_path);
        if !crate::maintenance::zygisk_projection::work::valid_boot(&boot) {
            return Err("managed bootstrap boot identity unavailable".into());
        }
        if work.boot_id == boot {
            return Ok(());
        }
        if work.outcome != PhaseOutcome::Confirmed
            || before.has_unknown()
            || before.backend_choice.as_ref().is_none_or(|choice| {
                choice.operation != work.operation
                    || choice.request.choice
                        != (BackendChoice::Fixed {
                            backend: NativeBackend::Zygisk,
                        })
            })
        {
            return Err(
                "managed bootstrap requires confirmed explicit choice and no held work".into(),
            );
        }
        let modules = env
            .modules
            .as_ref()
            .ok_or("managed source namespace unavailable")?;
        let source = modules.join("vpnhide_core");
        let catalog = Catalog::open(&source).map_err(|e| e.to_string())?;
        let recipient = self
            .loader
            .managed_recipient(&catalog)
            .map_err(|e| e.to_string())?
            .ok_or("managed bootstrap has no bound recipient")?;
        let wire = vpnhide_protocol::format_config(false, 0, &[]);
        let binding = BundleIdentity {
            root: source,
            root_identity: Some(catalog.root_identity),
            manifest_sha256: recipient.image.descriptor.manifest_sha256.clone(),
            manifest_file_sha256: Some(catalog.manifest_sha256),
            source_commit: catalog.manifest.bundle_commit,
            zygisk_recipient: Some(recipient.clone()),
            wire: wire.clone(),
        };
        validate_bundle_binding_owned(Some(&binding), uid)?;
        let target = ZygiskTarget::open(&recipient.path)?;
        let previous = previous_authority(&before, &target, &recipient.path)?;
        if previous.as_ref().is_some_and(|record| {
            record
                .intent
                .zygisk_authorization
                .as_ref()
                .is_some_and(|grant| grant.boot_id == boot)
        }) {
            // Preserve an actual current user-policy grant or a previously
            // confirmed bootstrap. Never clear it just to run the worker again.
            if before.zygisk_bootstrap.as_ref().is_some_and(|work| {
                work.authority == *previous.as_ref().expect("matched authority")
                    && (work.outcome != PhaseOutcome::Confirmed || work.binding != binding)
            }) {
                return Err("current managed bootstrap is unconfirmed or changed".into());
            }
            return Ok(());
        }
        if before.zygisk_bootstrap.as_ref().is_some_and(|work| {
            work.authority
                .intent
                .zygisk_authorization
                .as_ref()
                .is_some_and(|a| a.boot_id == boot)
        }) {
            return Err("this boot's managed bootstrap cannot be replayed".into());
        }
        let mut env = env.clone();
        env.loader = Some(self.loader.clone());
        let facts = crate::native_boot::gather(
            &env,
            crate::native_boot::probe_kpm(env.kpm_lock.as_deref()).fact,
        );
        if crate::payloads::device::observe(&env, &facts).ownership != Ownership::Clear {
            return Err(
                "managed bootstrap cannot overlap unknown or resident kernel ownership".into(),
            );
        }
        self.call_writer(move |state| -> Result<(), String> {
            if state.snapshot().startup_error.is_some() || state.snapshot().storage_fault.is_some()
            {
                return Err("managed bootstrap storage unavailable".into());
            }
            let mut journal = state
                .engine
                .store
                .lock()
                .expect("config mutex")
                .journal
                .clone();
            if !same_intent(&before, &journal)
                || journal.has_unknown()
                || state.epoch.load(Ordering::SeqCst) != epoch
                || crate::native_boot::read_boot_id(&env.boot_id_path) != boot
            {
                return Err("managed bootstrap intent changed before admission".into());
            }
            validate_bundle_binding_owned(Some(&binding), uid)?;
            let facts = crate::native_boot::gather(
                &env,
                crate::native_boot::probe_kpm(env.kpm_lock.as_deref()).fact,
            );
            if crate::payloads::device::observe(&env, &facts).ownership != Ownership::Clear {
                return Err("managed bootstrap ownership changed before admission".into());
            }
            if previous_authority(&journal, &target, &recipient.path)? != previous {
                return Err("managed bootstrap previous grant changed before admission".into());
            }
            let op = OperationId(
                journal
                    .issued_through
                    .checked_add(1)
                    .ok_or("operation ids exhausted")?,
            );
            journal.issued_through = op.0;
            journal.zygisk_bootstrap = Some(ProjectionBootstrap {
                binding,
                authority: ZygiskAuthorityRecord {
                    operation: op,
                    intent: NativeIntent {
                        backend: NativeBackend::Zygisk,
                        wire: wire.clone(),
                        zygisk_module: Some(recipient.path.clone()),
                        zygisk_authorization: Some(ZygiskAuthorizationIntent {
                            boot_id: boot.clone(),
                            generation: op.0,
                            directory_identity: recipient.image.directory,
                        }),
                    },
                },
                previous_authority: previous,
                publication_intent: false,
                outcome: PhaseOutcome::Running,
            });
            if !journal
                .zygisk_bootstrap
                .as_ref()
                .expect("bootstrap prepared")
                .valid(&journal)
            {
                return Err("managed bootstrap intent failed validation".into());
            }
            let path = state
                .journal_path
                .as_deref()
                .ok_or("bootstrap needs durable journal")?;
            state
                .persist_transition_journal(path, &journal, op)
                .map_err(|e| e.to_string())?;
            crate::recovery::maybe_crash_for_test("projection-bootstrap-intent");
            journal
                .zygisk_bootstrap
                .as_mut()
                .expect("bootstrap admitted")
                .publication_intent = true;
            journal.note_zygisk_exposure(&boot);
            state
                .persist_transition_journal(path, &journal, op)
                .map_err(|e| e.to_string())?;
            crate::recovery::maybe_crash_for_test("projection-bootstrap-publication-intent");
            let authority = journal
                .zygisk_bootstrap
                .as_ref()
                .expect("bootstrap admitted")
                .authority
                .clone();
            // fsync may block. Recheck the frozen code after its intent is
            // durable, before publishing even this empty authority.
            let code = validate_bundle_binding_owned(
                Some(
                    &journal
                        .zygisk_bootstrap
                        .as_ref()
                        .expect("bootstrap admitted")
                        .binding,
                ),
                uid,
            );
            let result = if let Err(reason) = code {
                Err(PublishError::BeforeWrite(reason))
            } else if crate::native_boot::read_boot_id(&env.boot_id_path) != boot {
                Err(PublishError::BeforeWrite(
                    "boot changed before publication".into(),
                ))
            } else {
                target.publish_authorized(&vpnhide_protocol::zygisk_activation::Authorization {
                    boot_id: boot,
                    generation: op.0,
                    wire,
                })
            };
            crate::recovery::maybe_crash_for_test("projection-bootstrap-effect");
            let (outcome, reason) = match result {
                Ok(()) if target.check_authorization(&authority).is_ok() => {
                    (PhaseOutcome::Confirmed, None)
                }
                Ok(()) => (
                    PhaseOutcome::Unknown,
                    Some("bootstrap grant readback unknown".into()),
                ),
                Err(PublishError::BeforeWrite(reason)) => (PhaseOutcome::FailedKnown, Some(reason)),
                Err(PublishError::Unknown(reason)) => (PhaseOutcome::Unknown, Some(reason)),
            };
            journal
                .zygisk_bootstrap
                .as_mut()
                .expect("bootstrap admitted")
                .outcome = outcome;
            state
                .persist_transition_journal(path, &journal, op)
                .map_err(|e| e.to_string())?;
            crate::recovery::maybe_crash_for_test("projection-bootstrap-completion");
            reason.map_or(Ok(()), Err)
        })
        .map_err(|error| format!("managed bootstrap writer unavailable: {error:?}"))?
    }
}
