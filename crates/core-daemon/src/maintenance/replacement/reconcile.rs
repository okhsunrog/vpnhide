//! Read-only generation proof. No manager retry, uninstall, reboot or backend
//! selection occurs here; returned operation receipts remain historical facts.
use super::*;
use vpnhide_core_rpc::{CoreMaintenanceFailure as Failure, CoreMaintenanceStage as Stage};
use vpnhide_package_trust::{AcceptedFloor, TrustStore, VerifiedDescriptor};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum GenerationProof {
    OldRunning,
    NewRunning,
    NoDispatch,
}
impl ReplacementWork {
    pub(crate) fn reconcile(&self, paths: &ReplacementPaths) -> Result<GenerationProof, Failure> {
        let running = File::open("/proc/self/exe").map_err(|_| Failure::RunningImageMismatch)?;
        if !self.dispatch_intent || paths.boot().map_err(|_| Failure::BootMismatch)? == self.boot_id
        {
            return self.same_boot(paths, &running, 0);
        }
        let verified = TrustStore::compiled()
            .and_then(|t| t.authenticate(&self.request.descriptor))
            .map_err(|_| Failure::InvalidPackage)?;
        let floor = FloorStore::open(&paths.persistent).map_err(|_| Failure::StorageUncertain)?;
        let floor = floor.lock().map_err(|_| Failure::StorageUncertain)?;
        let accepted = floor.read().map_err(|_| Failure::StorageUncertain)?;
        self.new_boot(paths, &running, 0, &verified, &accepted)
    }
    pub(super) fn same_boot(
        &self,
        paths: &ReplacementPaths,
        running: &File,
        uid: u32,
    ) -> Result<GenerationProof, Failure> {
        if !matches!(
            self.status.stage,
            Stage::PendingRestart | Stage::FailedKnown
        ) && self.dispatch_intent
        {
            return Err(Failure::DispatchUncertain);
        }
        self.before
            .require_unchanged_owned(&paths.active(), uid)
            .and_then(|_| self.before.require_running_owned(running, uid))
            .map_err(|_| Failure::RunningImageMismatch)?;
        if !self.dispatch_intent {
            // This boundary precedes the only dispatch intent. Prove sources
            // are unchanged before releasing its hold for a NEW user command.
            require_absent(&paths.staged()).map_err(|_| Failure::StagedImageInvalid)?;
            self.legacy
                .require_unchanged_owned(&paths.root, uid)
                .map_err(|_| Failure::LegacyChanged)?;
            return Ok(GenerationProof::NoDispatch);
        }
        Ok(GenerationProof::OldRunning)
    }
    pub(super) fn new_boot(
        &self,
        paths: &ReplacementPaths,
        running: &File,
        uid: u32,
        verified: &VerifiedDescriptor,
        floor: &AcceptedFloor,
    ) -> Result<GenerationProof, Failure> {
        let d = verified.descriptor();
        verified
            .admit(
                &vpnhide_package_trust::Target {
                    source_commit: self.request.expected_commit.clone(),
                    package_sha256: self.request.expected_package_sha256.clone(),
                    rpc: vpnhide_build_contract::RPC_VERSION,
                    current_journal: vpnhide_build_contract::JOURNAL_VERSION,
                },
                floor,
            )
            .map_err(|_| Failure::Compatibility)?;
        if self.request.target_rpc != d.rpc
            || floor.build_sequence != d.build_sequence
            || floor.package_sha256.as_deref() != Some(d.package_sha256.as_str())
        {
            return Err(Failure::StorageUncertain);
        }
        if (!self.retired && !self.legacy.modules.is_empty())
            || !self
                .staged
                .as_ref()
                .is_some_and(ImageFence::verified_layout)
        {
            return Err(Failure::LegacyChanged);
        }
        let active = ImageFence::inspect_replacement(&paths.active(), verified, uid)
            .map_err(|_| Failure::StagedImageInvalid)?;
        if !self
            .staged
            .as_ref()
            .is_some_and(|staged| staged.same_content(&active))
        {
            return Err(Failure::RunningImageMismatch);
        }
        active
            .require_running_owned(running, uid)
            .map_err(|_| Failure::RunningImageMismatch)?;
        if !self
            .legacy
            .retirement_observed_owned(&paths.root, uid)
            .map_err(|_| Failure::LegacyChanged)?
        {
            return Err(Failure::LegacyChanged);
        }
        Ok(GenerationProof::NewRunning)
    }
}
