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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionWork {
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
        let state_valid = match (self.outcome, self.failure, self.staging.as_ref()) {
            (PhaseOutcome::Running, None, None) => true,
            (
                PhaseOutcome::FailedKnown,
                Some(ProjectionFailure::SourcesChangedBeforeSpawn),
                None,
            ) => true,
            (
                PhaseOutcome::FailedKnown,
                Some(ProjectionFailure::InterruptedBeforeDispatch),
                None,
            ) => !self.dispatch_intent,
            (
                PhaseOutcome::Unknown,
                Some(ProjectionFailure::ManagerOutcomeUnknown | ProjectionFailure::ReadbackUnknown),
                None,
            ) => self.dispatch_intent,
            (PhaseOutcome::Confirmed, None, Some(staging)) => {
                self.dispatch_intent && staging.valid_for(&self.descriptor)
            }
            _ => false,
        };
        let source = &self.source.files;
        self.operation.0 > 0
            && self.operation.0 <= issued
            && self.operation.0 == self.descriptor.operation
            && self.request.expected_revision < u64::MAX
            && self.request.validate().is_ok()
            && self.request.choice
                == (BackendChoice::Fixed {
                    backend: NativeBackend::Zygisk,
                })
            && self.descriptor.validate().is_ok()
            && valid_boot(&self.boot_id)
            && self.source.root.1 > 0
            && source.len() <= 32
            && source.contains_key("vpnhide-core")
            && source
                .get("payloads/manifest.json")
                .is_some_and(|f| f.identity.sha256 == self.descriptor.manifest_file_sha256)
            && source
                .values()
                .all(|f| f.valid(crate::payloads::MAX_PAYLOAD_BYTES))
            && self.manager.valid()
            && state_valid
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
pub(super) fn valid_boot(boot: &str) -> bool {
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
        if !work.valid(work.operation.0)
            || work.dispatch_intent
            || work.outcome != PhaseOutcome::Running
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

/// Projection receipts share the same writer document and cannot be interpreted
/// by an older preference-only reader. Receipt retention keeps this exact intent.
pub(crate) fn validate_journal(journal: &crate::journal::Journal) -> bool {
    use vpnhide_core_rpc::ConfigPhase;
    let Some(work) = &journal.zygisk_projection else {
        return true;
    };
    if journal.version < 17 || !work.valid(journal.issued_through) {
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
    let choice = journal
        .backend_choice
        .as_ref()
        .filter(|choice| choice.operation == work.operation);
    if work.outcome == PhaseOutcome::Confirmed {
        choice.is_some_and(|choice| {
            choice.request == work.request && choice.requested_in_boot == work.boot_id
        })
    } else {
        choice.is_none()
    }
}
