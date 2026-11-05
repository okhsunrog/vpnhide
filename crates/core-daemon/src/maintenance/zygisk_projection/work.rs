//! Durable preparation effect, deliberately separate from hook authorization.
//! The existing command writer supplies the atomic journal/receipt commit. No
//! recovery path calls the manager or converts observed bytes into user consent.
use super::super::manager::ManagerIdentity;
use super::*;
use vpnhide_core_rpc::{
    BackendChoice, BackendChoiceRequest, NativeBackend, OperationId, PhaseOutcome,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectionFailure {
    InterruptedBeforeDispatch,
    SourcesChangedBeforeSpawn,
    ManagerOutcomeUnknown,
    ReadbackUnknown,
}

pub(super) fn valid_dispatch_state(
    dispatch: bool,
    outcome: PhaseOutcome,
    failure: Option<ProjectionFailure>,
    output: Option<bool>,
) -> bool {
    match (outcome, failure, output) {
        (PhaseOutcome::Running, None, None) => true,
        (PhaseOutcome::FailedKnown, Some(ProjectionFailure::SourcesChangedBeforeSpawn), None) => {
            true
        }
        (PhaseOutcome::FailedKnown, Some(ProjectionFailure::InterruptedBeforeDispatch), None) => {
            !dispatch
        }
        (
            PhaseOutcome::Unknown,
            Some(ProjectionFailure::ManagerOutcomeUnknown | ProjectionFailure::ReadbackUnknown),
            None,
        ) => dispatch,
        (PhaseOutcome::Confirmed, None, Some(true)) => dispatch,
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionWork {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_continuity: Option<super::install::InstallContinuity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_adoption: Option<super::legacy::LegacyAdoption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_withdrawal: Option<super::withdrawal::WithdrawalImage>,
    pub operation: OperationId,
    pub request: BackendChoiceRequest,
    pub boot_id: String,
    pub descriptor: Descriptor,
    pub source: ImageFence,
    pub manager: ManagerIdentity,
    /// Sticky before-effect record. Never reset after a failed manager call.
    pub dispatch_intent: bool,
    pub outcome: PhaseOutcome,
    pub failure: Option<ProjectionFailure>,
    pub staging: Option<ProjectionStaging>,
}

impl ProjectionWork {
    pub(crate) fn valid(&self, issued: u64) -> bool {
        let state_valid = valid_dispatch_state(
            self.dispatch_intent,
            self.outcome,
            self.failure,
            self.staging
                .as_ref()
                .map(|staging| staging.valid_for(&self.descriptor)),
        );
        self.previous_withdrawal.as_ref().is_none_or(|image| {
            image.valid_for(&image.descriptor) && image.descriptor.operation < self.operation.0
        }) && self.operation.0 > 0
            && self.operation.0 <= issued
            && (self.operation.0 == self.descriptor.operation || self.install_continuity.is_some())
            && self.request.expected_revision < u64::MAX
            && (self.request.validate().is_ok() || self.install_continuity.is_some())
            && self.request.choice
                == (BackendChoice::Fixed {
                    backend: NativeBackend::Zygisk,
                })
            && self.descriptor.validate().is_ok()
            && valid_boot(&self.boot_id)
            && self
                .source
                .valid_source(&self.descriptor.manifest_file_sha256)
            && self.manager.valid()
            && self
                .install_continuity
                .as_ref()
                .is_none_or(|origin| origin.valid(self))
            && self.staging.as_ref().is_none_or(|staging| {
                match (&self.previous_withdrawal, &staging.active) {
                    (Some(expected), Some(super::ActiveProjection::Withdrawn { image })) => {
                        expected == image.as_ref()
                    }
                    (None, Some(super::ActiveProjection::Withdrawn { .. })) => false,
                    (Some(_), _) => false,
                    _ => true,
                }
            })
            && self
                .legacy_adoption
                .as_ref()
                .is_none_or(|legacy| legacy.valid(self))
            && (state_valid
                || self
                    .legacy_adoption
                    .as_ref()
                    .is_some_and(|legacy| legacy.intermediate_state(self)))
    }
    pub(crate) fn permits_successor(&self, next: &Self) -> bool {
        if self == next {
            return true;
        }
        if match (&self.legacy_adoption, &next.legacy_adoption) {
            (Some(previous), Some(next)) => !previous.permits(next),
            (None, None) => false,
            _ => true,
        } || (self.staging.is_some() && self.staging != next.staging)
            || self.previous_withdrawal != next.previous_withdrawal
            || self.operation != next.operation
            || self.request != next.request
            || self.boot_id != next.boot_id
            || self.descriptor != next.descriptor
            || self.source != next.source
            || self.manager != next.manager
            || (self.dispatch_intent && !next.dispatch_intent)
            || self.outcome != PhaseOutcome::Running
        {
            return false;
        }
        if self.dispatch_intent {
            (next.outcome == PhaseOutcome::Running
                && self
                    .legacy_adoption
                    .as_ref()
                    .is_some_and(|legacy| !legacy.retirement_intent)
                && next
                    .legacy_adoption
                    .as_ref()
                    .is_some_and(|legacy| legacy.retirement_intent && !legacy.retired)
                && self.staging.is_none()
                && next.staging.is_some())
                || matches!(
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
            if self.dispatch_intent {
                self.outcome = PhaseOutcome::Unknown;
                self.failure = Some(ProjectionFailure::ManagerOutcomeUnknown);
            } else {
                self.outcome = PhaseOutcome::FailedKnown;
                self.failure = Some(ProjectionFailure::InterruptedBeforeDispatch);
            }
        }
    }
    /// Read-only physical evidence. This never rewrites a returned Unknown,
    /// commits a backend preference, grants hooks, or redispatches the manager.
    pub fn observe(
        &self,
        paths: &ProjectionPaths,
        boot: &str,
        running: &File,
    ) -> io::Result<ProjectionObservation> {
        self.observe_owned(paths, boot, running, 0)
    }
    pub(super) fn observe_owned(
        &self,
        paths: &ProjectionPaths,
        boot: &str,
        running: &File,
        uid: u32,
    ) -> io::Result<ProjectionObservation> {
        self.observe_with_authority_owned(paths, boot, running, uid, None)
    }
    pub(crate) fn observe_with_authority_owned(
        &self,
        paths: &ProjectionPaths,
        boot: &str,
        running: &File,
        uid: u32,
        authority: Option<&crate::journal::ZygiskAuthorityRecord>,
    ) -> io::Result<ProjectionObservation> {
        if !self.valid(self.operation.0) || !valid_boot(boot) || paths.boot()? != boot {
            return Err(invalid());
        }
        if !self.dispatch_intent || self.outcome == PhaseOutcome::FailedKnown {
            return Ok(ProjectionObservation::NotDispatched);
        }
        paths.require_source_enabled(uid)?;
        let source_directory = std::sync::Arc::new(directory(&paths.source())?);
        let source = ImageFence::inspect(&paths.source(), None, crate::payloads::Abi::Arm64, uid)?;
        if source.root != identity(&source_directory)? {
            return Err(invalid());
        }
        source.require_running_owned(running, uid)?;
        if boot == self.boot_id {
            if source != self.source {
                return Err(invalid());
            }
            let observed = ProjectionStaging::inspect_with_authority_owned(
                paths,
                &self.descriptor,
                uid,
                authority,
            )?;
            if self
                .staging
                .as_ref()
                .is_some_and(|expected| expected != &observed)
            {
                return Err(invalid());
            }
            Ok(ProjectionObservation::PreparedInSameBoot(observed))
        } else {
            if self.legacy_adoption.as_ref().is_some_and(|legacy| {
                !legacy.retired
                    || !legacy
                        .inventory
                        .retirement_observed_owned(&paths.root, uid)
                        .unwrap_or(false)
            }) {
                return Err(invalid());
            }
            if !source.same_content(&self.source) {
                return Err(invalid());
            }
            absent(&paths.staged())?;
            let recipient_directory = std::sync::Arc::new(directory(&paths.active())?);
            let active = ProjectionImage::inspect_with_authority_owned(
                &paths.active(),
                &self.descriptor,
                uid,
                authority,
            )?;
            if active.directory != identity(&recipient_directory)?
                || identity(&directory(&paths.source())?)? != source.root
            {
                return Err(invalid());
            }
            Ok(ProjectionObservation::ActiveAfterBoot(
                ActiveProjectionReadback {
                    source,
                    image: active,
                    source_directory,
                    recipient_directory,
                },
            ))
        }
    }
}

/// Packaging observations only, never backend delivery or behavior proof.
#[derive(Debug, Clone)]
pub enum ProjectionObservation {
    NotDispatched,
    PreparedInSameBoot(ProjectionStaging),
    ActiveAfterBoot(ActiveProjectionReadback),
}
/// Ephemeral leases close the readback-to-binding directory ABA window. These
/// handles are not persisted and never substitute for current byte inspection.
#[derive(Debug, Clone)]
pub struct ActiveProjectionReadback {
    pub source: ImageFence,
    pub image: ProjectionImage,
    pub(crate) source_directory: std::sync::Arc<File>,
    pub(crate) recipient_directory: std::sync::Arc<File>,
}
pub(crate) fn valid_boot(boot: &str) -> bool {
    vpnhide_protocol::zygisk_activation::Authorization {
        boot_id: boot.into(),
        generation: 1,
        wire: vpnhide_protocol::format_config(false, 0, &[]),
    }
    .encode()
    .is_some()
}

impl PreparedProjection {
    pub fn work(&self, request: BackendChoiceRequest) -> io::Result<ProjectionWork> {
        let work = ProjectionWork {
            install_continuity: None,
            legacy_adoption: self.legacy.as_ref().map(|legacy| legacy.work()),
            previous_withdrawal: self.previous_withdrawal.clone(),
            operation: OperationId(self.descriptor.operation),
            request,
            boot_id: self.boot_id.clone(),
            descriptor: self.descriptor.clone(),
            source: self.source.clone(),
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
    /// Call only on the existing writer. `persist` must atomically fsync this work
    /// with its operation receipt through journal::backend_choice::record_projection;
    /// Confirmed also commits the device-local choice. Never replace a terminal
    /// work record with a freshly rebuilt initial intent for the same operation.
    /// Any persistence error stops further work, including before manager spawn.
    /// A final error has uncertain durability, not a known failed installation.
    pub fn dispatch(
        &self,
        mut work: ProjectionWork,
        mut persist: impl FnMut(&ProjectionWork) -> io::Result<()>,
    ) -> io::Result<ProjectionWork> {
        if work.install_continuity.is_some()
            || !work.valid(work.operation.0)
            || work.dispatch_intent
            || work.outcome != PhaseOutcome::Running
            || work.legacy_adoption != self.legacy.as_ref().map(|legacy| legacy.work())
            || work.previous_withdrawal != self.previous_withdrawal
            || work.descriptor != self.descriptor
            || work.source != self.source
            || work.manager != self.manager.identity
            || work.boot_id != self.boot_id
        {
            return Err(invalid());
        }
        self.require_fresh()?;
        persist(&work)?;
        if self.require_fresh().is_err() {
            work.outcome = PhaseOutcome::FailedKnown;
            work.failure = Some(ProjectionFailure::SourcesChangedBeforeSpawn);
            persist(&work)?;
            return Ok(work);
        }
        work.dispatch_intent = true;
        persist(&work)?;
        // The callback may block on fsync. Check the pinned source/manager/output
        // boundary again after it; no child has been started on this branch.
        if self.require_fresh().is_err() {
            work.outcome = PhaseOutcome::FailedKnown;
            work.failure = Some(ProjectionFailure::SourcesChangedBeforeSpawn);
        } else {
            match self.manager.stage(self.package()) {
                Ok(result)
                    if !result.timed_out && !result.truncated && result.output.status.success() =>
                {
                    match self
                        .manager
                        .require_unchanged()
                        .and_then(|()| self.verify_staged())
                    {
                        Ok(staging) => {
                            work.staging = Some(staging);
                            if let Some(legacy) = &mut work.legacy_adoption {
                                legacy.retirement_intent = true;
                                persist(&work)?;
                                if self.retire_legacy(&work).is_ok() {
                                    work.legacy_adoption
                                        .as_mut()
                                        .expect("adoption prepared")
                                        .retired = true;
                                    work.outcome = PhaseOutcome::Confirmed;
                                } else {
                                    work.outcome = PhaseOutcome::Unknown;
                                    work.failure = Some(ProjectionFailure::ReadbackUnknown);
                                }
                            } else {
                                work.outcome = PhaseOutcome::Confirmed;
                            }
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

/// Projection receipts share the same writer document and cannot be interpreted
/// by an older preference-only reader. Receipt retention keeps this exact intent.
pub(crate) fn validate_journal(journal: &crate::journal::Journal) -> bool {
    use vpnhide_core_rpc::ConfigPhase;
    let Some(work) = &journal.zygisk_projection else {
        return true;
    };
    if ((work.legacy_adoption.is_some() || work.install_continuity.is_some())
        && journal.version < 21)
        || journal.version < 17
        || !work.valid(journal.issued_through)
        || !super::history::predecessor_valid(journal, work)
    {
        return false;
    }
    let Some(entry) = journal.get(work.operation) else {
        return false;
    };
    let command = match &work.install_continuity {
        Some(origin) => crate::journal::CommandIntent::AdoptInstallHandoff {
            intent_sha256: origin.intent.digest(),
        },
        None => crate::journal::CommandIntent::StageBackendChoice {
            request: work.request.clone(),
        },
    };
    if entry.command != Some(command)
        || entry.record.phases.len() != 1
        || entry.record.phases[0].phase != ConfigPhase::Maintenance
        || entry.record.phases[0].outcome != work.outcome
        || entry.record.settled != (work.outcome != PhaseOutcome::Running).then_some(work.outcome)
        || !entry.superseded_phases.is_empty()
    {
        return false;
    }
    let choice = journal
        .backend_choice
        .as_ref()
        .filter(|choice| choice.operation == work.operation);
    if work.outcome == PhaseOutcome::Confirmed {
        choice.is_some_and(|choice| {
            choice.request == work.request && choice.requested_in_boot == work.boot_id
        }) || super::withdrawal::work::supersedes_choice(journal, work)
    } else {
        choice.is_none()
    }
}
