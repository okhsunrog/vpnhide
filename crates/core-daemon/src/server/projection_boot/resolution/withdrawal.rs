//! Negative/old grant readback is useful only for an explicit withdrawal, never
//! permission to republish Zygisk or to overlap kernel hooks in this boot.
use super::*;
use crate::journal::ZygiskAuthorityRecord;
use vpnhide_core_rpc::{BackendChoice, BackendChoiceRequest, NativeBackend};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapWithdrawalReadback {
    pub boot_id: String,
    pub grant: Option<ZygiskAuthorityRecord>,
}
impl BootstrapWithdrawalReadback {
    pub(crate) fn valid(&self, resolution: &BootstrapResolution, journal: &Journal) -> bool {
        crate::maintenance::zygisk_projection::work::valid_boot(&self.boot_id)
            && self.grant.as_ref().is_none_or(|grant| {
                grant == &resolution.observed.authority || resolution.observed.previous_authority.as_ref() == Some(grant)
            })
            && journal.get(resolution.operation).is_some_and(|entry| {
                matches!(&entry.command, Some(crate::journal::CommandIntent::StageBackendChoice { request }) if request.choice != (BackendChoice::Fixed { backend: NativeBackend::Zygisk }))
                    && entry.record.phases.len() == 1 && entry.record.phases[0].phase == ConfigPhase::Maintenance
            })
            && journal.zygisk_withdrawal.iter().chain(journal.projection_history.iter().map(|cycle| &cycle.withdrawal)).any(|work| {
                work.operation == resolution.operation && work.retained() && work.boot_id == self.boot_id
                    && work.authority == self.grant
                    && resolution.observed.binding.zygisk_recipient.as_ref().is_some_and(|recipient| recipient.image.descriptor == work.descriptor.published)
            })
    }
}

/// Only this one uncertainty may be settled by the same scoped withdrawal.
pub(crate) fn can_withdraw(journal: &Journal, request: &BackendChoiceRequest) -> bool {
    if request.choice
        == (BackendChoice::Fixed {
            backend: NativeBackend::Zygisk,
        })
        || !journal.bootstrap_uncertain()
        || journal
            .zygisk_projection
            .as_ref()
            .is_none_or(|work| work.outcome != PhaseOutcome::Confirmed)
        || journal
            .zygisk_withdrawal
            .as_ref()
            .is_some_and(|work| work.retained())
    {
        return false;
    }
    let mut other = journal.clone();
    other.zygisk_bootstrap = None;
    other.bootstrap_resolution = None;
    !other.has_unknown()
}

pub(crate) fn inspect_withdrawal_owned(
    journal: &Journal,
    env: &NativeEnv,
    operation: OperationId,
    running: &std::fs::File,
    uid: u32,
) -> Result<BootstrapResolution, String> {
    let work = journal
        .zygisk_bootstrap
        .as_ref()
        .ok_or("bootstrap unavailable")?;
    let boot = crate::native_boot::read_boot_id(&env.boot_id_path);
    if !crate::maintenance::zygisk_projection::work::valid_boot(&boot)
        || work.outcome != PhaseOutcome::Unknown
        || !work.publication_intent
        || !work.valid(journal)
        || env
            .modules
            .as_ref()
            .is_none_or(|modules| modules.join("vpnhide_core") != work.binding.root)
    {
        return Err("bootstrap withdrawal requires the retained managed generation".into());
    }
    super::require_source(journal, work, running, uid)?;
    let (recipient, _) = work.binding.recipient();
    let target = crate::zygisk::ZygiskTarget::open(recipient)?;
    let grant = if target.authorization_absent()? {
        None
    } else {
        Some(
            [Some(&work.authority), work.previous_authority.as_ref()]
                .into_iter()
                .flatten()
                .find(|authority| target.require_authorization(authority).is_ok())
                .cloned()
                .ok_or("bootstrap withdrawal found an unowned grant")?,
        )
    };
    if crate::native_boot::read_boot_id(&env.boot_id_path) != boot {
        return Err("bootstrap withdrawal boot changed during readback".into());
    }
    Ok(BootstrapResolution {
        operation,
        observed: work.clone(),
        withdrawal: Some(BootstrapWithdrawalReadback {
            boot_id: boot,
            grant,
        }),
    })
}

pub(crate) struct PreparedBootstrapWithdrawal {
    pub receipt: BootstrapResolution,
    running: std::fs::File,
    uid: u32,
}
impl PreparedBootstrapWithdrawal {
    pub(crate) fn prepare(
        journal: &Journal,
        env: &NativeEnv,
        operation: OperationId,
    ) -> Result<Self, String> {
        Self::prepare_owned(
            journal,
            env,
            operation,
            std::fs::File::open("/proc/self/exe").map_err(|e| e.to_string())?,
            0,
        )
    }
    pub(crate) fn prepare_owned(
        journal: &Journal,
        env: &NativeEnv,
        operation: OperationId,
        running: std::fs::File,
        uid: u32,
    ) -> Result<Self, String> {
        let receipt = inspect_withdrawal_owned(journal, env, operation, &running, uid)?;
        Ok(Self {
            receipt,
            running,
            uid,
        })
    }
    pub(crate) fn require_fresh(&self, journal: &Journal, env: &NativeEnv) -> Result<(), String> {
        if inspect_withdrawal_owned(
            journal,
            env,
            self.receipt.operation,
            &self.running,
            self.uid,
        )? != self.receipt
        {
            return Err("bootstrap withdrawal readback changed".into());
        }
        Ok(())
    }
}
