//! Explicit readback settlement. Historical Unknown and boot exposure survive;
//! no recovery path republishes a grant or infers that hooks exited.
use crate::journal::{Journal, projection_bootstrap::ProjectionBootstrap};
use crate::native_boot::{NativeEnv, prepared::validate_bundle_binding_owned};
use serde::{Deserialize, Serialize};
use vpnhide_core_rpc::{ConfigEdit, ConfigPhase, OperationId, PhaseOutcome};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapResolution {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub withdrawal: Option<withdrawal::BootstrapWithdrawalReadback>,
    pub operation: OperationId,
    pub observed: ProjectionBootstrap,
}
impl BootstrapResolution {
    pub(crate) fn valid(&self, journal: &Journal) -> bool {
        journal.zygisk_bootstrap.as_ref() == Some(&self.observed) && self.valid_record(journal)
    }
    pub(crate) fn valid_record(&self, journal: &Journal) -> bool {
        journal.version >= 21
            && self.observed.outcome == PhaseOutcome::Unknown
            && self.observed.publication_intent
            && self.observed.valid(journal)
            && self.operation.0 > self.observed.authority.operation.0
            && self.operation.0 <= journal.issued_through
            && match &self.withdrawal {
                Some(readback) => readback.valid(self, journal),
                None => journal.get(self.operation).is_some_and(|entry| {
                    entry.command == Some(crate::journal::CommandIntent::Edit)
                        && entry.edit == ConfigEdit::RepairActivation
                        && entry
                            .record
                            .phases
                            .iter()
                            .any(|phase| phase.phase == ConfigPhase::Native)
                }),
            }
    }
}

pub(crate) fn inspect(
    journal: &Journal,
    env: &NativeEnv,
    uid: u32,
) -> Result<ProjectionBootstrap, String> {
    inspect_owned(
        journal,
        env,
        &std::fs::File::open("/proc/self/exe").map_err(|e| e.to_string())?,
        uid,
    )
}
pub(crate) fn inspect_owned(
    journal: &Journal,
    env: &NativeEnv,
    running: &std::fs::File,
    uid: u32,
) -> Result<ProjectionBootstrap, String> {
    let work = journal
        .zygisk_bootstrap
        .as_ref()
        .ok_or("bootstrap unavailable")?;
    let grant = work
        .authority
        .reference()
        .ok_or("bootstrap grant identity unavailable")?;
    if work.outcome != PhaseOutcome::Unknown
        || !work.publication_intent
        || !work.valid(journal)
        || env
            .modules
            .as_ref()
            .is_none_or(|modules| modules.join("vpnhide_core") != work.binding.root)
        || crate::native_boot::read_boot_id(&env.boot_id_path) != grant.boot_id
        || !journal.zygisk_exposed_in(&grant.boot_id)
    {
        return Err("bootstrap recovery requires this boot's exact retained grant".into());
    }
    require_source(journal, work, running, uid)?;
    let (recipient, _) = work.binding.recipient();
    crate::zygisk::ZygiskTarget::open(recipient)?.require_authorization(&work.authority)?;
    if crate::native_boot::read_boot_id(&env.boot_id_path) != grant.boot_id {
        return Err("bootstrap recovery boot changed during readback".into());
    }
    Ok(work.clone())
}

impl crate::server::CoreState {
    pub(crate) fn prepare_bootstrap_recovery(
        &self,
        op: OperationId,
        command: &vpnhide_core_rpc::CoreCommand,
    ) -> Result<(), vpnhide_core_rpc::SubmitError> {
        use vpnhide_core_rpc::{CoreCommand, SubmitError};
        if !matches!(
            command,
            CoreCommand::Edit {
                edit: ConfigEdit::RepairActivation,
                ..
            }
        ) {
            return Ok(());
        }
        let journal = {
            let store = self.engine.store.lock().expect("config mutex");
            if store.journal.get(op).is_some()
                || store
                    .journal
                    .zygisk_bootstrap
                    .as_ref()
                    .is_none_or(|work| work.outcome != PhaseOutcome::Unknown)
            {
                return Ok(());
            }
            if op.0 == 0 {
                return Err(SubmitError::Rejected {
                    reason: "operation_not_allocated".into(),
                });
            }
            if store.journal.is_retired(op) {
                return Err(SubmitError::Expired);
            }
            if op.0 == 0 || op.0 > store.journal.issued_through {
                return Err(SubmitError::Rejected {
                    reason: "operation_not_allocated".into(),
                });
            }
            if let CoreCommand::Edit { base, .. } = command
                && base.revision > store.config_revision
            {
                return Err(SubmitError::Conflict {
                    field: "bootstrap_recovery_base".into(),
                });
            }
            if store.config.is_none()
                || self.snapshot().startup_error.is_some()
                || self.snapshot().storage_fault.is_some()
            {
                return Err(SubmitError::Unavailable);
            }
            store.journal.clone()
        };
        if journal
            .bootstrap_resolution
            .as_ref()
            .is_some_and(|resolution| resolution.withdrawal.is_some() && resolution.valid(&journal))
        {
            return Ok(());
        }
        let env = self
            .native_handle()
            .ok_or(SubmitError::Unavailable)?
            .lock()
            .expect("native boot")
            .env
            .clone();
        inspect(&journal, &env, 0).map_err(|reason| SubmitError::Rejected { reason })?;
        // A real fresh Android specialization is required by ordinary selection.
        // The probe is off-writer and never publishes or revokes authorization.
        self.loader
            .probe_worker()
            .map_err(|reason| SubmitError::Rejected { reason })
    }
}

fn require_source(
    journal: &Journal,
    work: &ProjectionBootstrap,
    running: &std::fs::File,
    uid: u32,
) -> Result<(), String> {
    validate_bundle_binding_owned(Some(&work.binding), uid)?;
    let source = crate::maintenance::image::ImageFence::inspect(
        &work.binding.root,
        None,
        crate::payloads::Abi::Arm64,
        uid,
    )
    .map_err(|e| e.to_string())?;
    if !source.valid_source(
        work.binding
            .manifest_file_sha256
            .as_deref()
            .ok_or("bootstrap source digest missing")?,
    ) {
        return Err("bootstrap source changed".into());
    }
    if journal
        .projection_identity()
        .is_none_or(|(_, expected, _)| !source.same_content(expected))
    {
        return Err("bootstrap source differs from the retained verified generation".into());
    }
    source
        .require_running_owned(running, uid)
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub(crate) mod withdrawal;
