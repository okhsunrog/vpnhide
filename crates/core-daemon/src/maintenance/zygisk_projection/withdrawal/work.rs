//! Durable manager attempt, distinct from the earlier publication and from
//! current hooks. Physical observations never rewrite terminal Unknown.
use super::super::work::{ProjectionFailure, valid_dispatch_state};
use super::*;
use crate::maintenance::manager::ManagerIdentity;
use vpnhide_core_rpc::{
    BackendChoice, BackendChoiceRequest, NativeBackend, OperationId, PhaseOutcome,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WithdrawalWork {
    pub operation: OperationId,
    pub request: BackendChoiceRequest,
    pub boot_id: String,
    pub descriptor: WithdrawalDescriptor,
    pub source: ImageFence,
    pub active: ProjectionImage,
    pub authority: Option<ZygiskAuthorityRecord>,
    pub manager: ManagerIdentity,
    pub dispatch_intent: bool,
    pub outcome: PhaseOutcome,
    pub failure: Option<ProjectionFailure>,
    pub staging: Option<WithdrawalImage>,
}
impl WithdrawalWork {
    pub(crate) fn valid(&self, issued: u64) -> bool {
        self.operation.0 > 0
            && self.operation.0 <= issued
            && self.operation.0 == self.descriptor.operation
            && self.descriptor.validate().is_ok()
            && self.request.expected_revision < u64::MAX
            && self.request.validate().is_ok()
            && self.request.choice
                != (BackendChoice::Fixed {
                    backend: NativeBackend::Zygisk,
                })
            && super::super::work::valid_boot(&self.boot_id)
            && self
                .source
                .valid_source(&self.descriptor.published.manifest_file_sha256)
            && self.active.valid_for(&self.descriptor.published)
            && self.authority.as_ref().is_none_or(|authority| {
                authority.reference().is_some_and(|reference| {
                    authority.operation.0 < self.operation.0
                        && authority.intent.zygisk_module.as_ref().is_some_and(|path| {
                            path.is_absolute()
                                && path
                                    .file_name()
                                    .is_some_and(|name| name == MANAGED_MODULE_ID)
                        })
                        && (reference.directory_device, reference.directory_inode)
                            == self.active.directory
                })
            })
            && self.manager.valid()
            && valid_dispatch_state(
                self.dispatch_intent,
                self.outcome,
                self.failure,
                self.staging
                    .as_ref()
                    .map(|image| image.valid_for(&self.descriptor)),
            )
    }
    pub(crate) fn permits_successor(&self, next: &Self) -> bool {
        if self == next {
            return true;
        }
        if self.operation != next.operation
            || self.request != next.request
            || self.boot_id != next.boot_id
            || self.descriptor != next.descriptor
            || self.source != next.source
            || self.active != next.active
            || self.authority != next.authority
            || self.manager != next.manager
            || (self.dispatch_intent && !next.dispatch_intent)
            || self.outcome != PhaseOutcome::Running
        {
            return false;
        }
        if self.dispatch_intent {
            matches!(
                next.outcome,
                PhaseOutcome::Confirmed | PhaseOutcome::Unknown | PhaseOutcome::FailedKnown
            )
        } else {
            (next.outcome == PhaseOutcome::Running && next.dispatch_intent)
                || (next.outcome == PhaseOutcome::FailedKnown && !next.dispatch_intent)
        }
    }
    pub fn retained(&self) -> bool {
        self.outcome != PhaseOutcome::FailedKnown
    }
    pub(crate) fn blocks_runtime(&self) -> bool {
        matches!(self.outcome, PhaseOutcome::Running | PhaseOutcome::Unknown)
    }
    pub(crate) fn fence_restart(&mut self) {
        if self.outcome == PhaseOutcome::Running {
            (self.outcome, self.failure) = if self.dispatch_intent {
                (
                    PhaseOutcome::Unknown,
                    Some(ProjectionFailure::ManagerOutcomeUnknown),
                )
            } else {
                (
                    PhaseOutcome::FailedKnown,
                    Some(ProjectionFailure::InterruptedBeforeDispatch),
                )
            };
        }
    }
}
impl PreparedWithdrawal {
    pub fn work(&self, request: BackendChoiceRequest) -> io::Result<WithdrawalWork> {
        let work = WithdrawalWork {
            operation: OperationId(self.descriptor.operation),
            request,
            boot_id: self.boot.clone(),
            descriptor: self.descriptor.clone(),
            source: self.source.clone(),
            active: self.active.clone(),
            authority: self.authority.clone(),
            manager: self.manager.identity.clone(),
            dispatch_intent: false,
            outcome: PhaseOutcome::Running,
            failure: None,
            staging: None,
        };
        if !work.valid(work.operation.0) {
            return Err(invalid());
        }
        Ok(work)
    }
    /// The callback must sync the existing writer's work and receipt before
    /// returning. There is no recovery manager call and no in-place revocation.
    pub fn dispatch(
        &self,
        mut work: WithdrawalWork,
        mut persist: impl FnMut(&WithdrawalWork) -> io::Result<()>,
    ) -> io::Result<WithdrawalWork> {
        if !work.valid(work.operation.0)
            || work.dispatch_intent
            || work.outcome != PhaseOutcome::Running
            || work.descriptor != self.descriptor
            || work.source != self.source
            || work.active != self.active
            || work.authority != self.authority
            || work.manager != self.manager.identity
            || work.boot_id != self.boot
        {
            return Err(invalid());
        }
        persist(&work)?;
        if self.require_fresh().is_err() {
            work.outcome = PhaseOutcome::FailedKnown;
            work.failure = Some(ProjectionFailure::SourcesChangedBeforeSpawn);
            persist(&work)?;
            return Ok(work);
        }
        work.dispatch_intent = true;
        persist(&work)?;
        if self.require_fresh().is_err() {
            work.outcome = PhaseOutcome::FailedKnown;
            work.failure = Some(ProjectionFailure::SourcesChangedBeforeSpawn);
        } else {
            match self.manager.stage(self.package()) {
                Ok(result)
                    if !result.timed_out && !result.truncated && result.output.status.success() =>
                {
                    match self.verify_staged() {
                        Ok(image) => {
                            work.staging = Some(image);
                            work.outcome = PhaseOutcome::Confirmed;
                        }
                        Err(_) => {
                            work.outcome = PhaseOutcome::Unknown;
                            work.failure = Some(ProjectionFailure::ReadbackUnknown);
                        }
                    }
                }
                _ => {
                    work.outcome = PhaseOutcome::Unknown;
                    work.failure = Some(ProjectionFailure::ManagerOutcomeUnknown);
                }
            }
        }
        persist(&work)?;
        Ok(work)
    }
}

/// A linked withdrawal preserves the original publication receipt. Only its
/// confirmed choice may supersede the publication's choice; Unknown never can.
pub(crate) fn supersedes_choice(
    journal: &crate::journal::Journal,
    published: &super::super::work::ProjectionWork,
) -> bool {
    journal.zygisk_withdrawal.as_ref().is_some_and(|work| {
        work.outcome == PhaseOutcome::Confirmed
            && journal.owns_projection_descriptor(&work.descriptor.published)
            && work.request.expected_revision == published.request.expected_revision + 1
            && validate_journal(journal)
    })
}

pub(crate) fn validate_journal(journal: &crate::journal::Journal) -> bool {
    use vpnhide_core_rpc::ConfigPhase;
    let Some(work) = &journal.zygisk_withdrawal else {
        return true;
    };
    if journal.version < 20 || !work.valid(journal.issued_through) {
        return false;
    }
    let Some(published) = &journal.zygisk_projection else {
        return false;
    };
    if !published.valid(journal.issued_through)
        || published.outcome != PhaseOutcome::Confirmed
        || journal
            .projection_identity()
            .is_none_or(|(descriptor, source, _)| {
                descriptor != &work.descriptor.published || !source.same_content(&work.source)
            })
        || work.request.expected_revision != published.request.expected_revision + 1
    {
        return false;
    }
    let Some(entry) = journal.get(work.operation) else {
        return false;
    };
    if entry.command
        != Some(crate::journal::CommandIntent::StageBackendChoice {
            request: work.request.clone(),
        })
        || entry.record.phases.len() != 1
        || entry.record.phases[0].phase != ConfigPhase::Maintenance
        || entry.record.phases[0].outcome != work.outcome
        || entry.record.settled != (work.outcome != PhaseOutcome::Running).then_some(work.outcome)
        || !entry.superseded_phases.is_empty()
    {
        return false;
    }
    journal.backend_choice.as_ref().is_some_and(|choice| {
        if work.outcome == PhaseOutcome::Confirmed {
            (choice.operation == work.operation
                && choice.request == work.request
                && choice.requested_in_boot == work.boot_id
                && journal.withdrawal_followup.is_none())
                || super::followup::valid(journal)
        } else {
            choice.operation == published.operation
                && choice.request == published.request
                && choice.requested_in_boot == published.boot_id
        }
    })
}
