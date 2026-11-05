//! Fresh physical evidence, never a rewrite of a historical manager outcome.
use super::work::WithdrawalWork;
use super::*;
use crate::maintenance::zygisk_projection::work::ActiveProjectionReadback;
use vpnhide_core_rpc::{OperationId, PhaseOutcome};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WithdrawalBootProof {
    pub operation: OperationId,
    pub boot_id: String,
    pub root: PathBuf,
    pub source: ImageFence,
    pub image: WithdrawalImage,
    /// Only a separately journaled reentry may retain an exact inert stage.
    pub pending_reentry: Option<Box<ProjectionImage>>,
}
pub(crate) enum WithdrawalObservation {
    RetainedActive(ActiveProjectionReadback),
    Withdrawn(WithdrawalBootProof),
}
impl WithdrawalBootProof {
    pub(crate) fn valid_for(&self, work: &WithdrawalWork) -> bool {
        work.outcome == PhaseOutcome::Confirmed
            && work.dispatch_intent
            && self.operation == work.operation
            && self.boot_id != work.boot_id
            && super::super::work::valid_boot(&self.boot_id)
            && self.root.is_absolute()
            && self.image.valid_for(&work.descriptor)
    }
    pub(crate) fn recheck(&self, env: &crate::native_boot::NativeEnv) -> io::Result<()> {
        if env.modules.as_ref() != Some(&self.root.join("data/adb/modules"))
            || crate::native_boot::read_boot_id(&env.boot_id_path) != self.boot_id
        {
            return Err(invalid());
        }
        self.recheck_owned(&File::open("/proc/self/exe")?, 0)
    }
    pub(crate) fn recheck_owned(&self, running: &File, uid: u32) -> io::Result<()> {
        let paths = ProjectionPaths {
            root: self.root.clone(),
            persistent: PathBuf::new(),
        };
        if paths.boot()? != self.boot_id {
            return Err(invalid());
        }
        paths.require_source_enabled(uid)?;
        self.source.require_unchanged_owned(&paths.source(), uid)?;
        self.source.require_running_owned(running, uid)?;
        if let Some(expected) = &self.pending_reentry {
            if ProjectionImage::inspect_owned(&paths.staged(), &expected.descriptor, uid)?
                != **expected
            {
                return Err(invalid());
            }
        } else {
            absent(&paths.staged())?;
        }
        if WithdrawalImage::inspect_owned(&paths.active(), &self.image.descriptor, uid)?
            != self.image
        {
            return Err(invalid());
        }
        self.source.require_unchanged_owned(&paths.source(), uid)?;
        if paths.boot()? != self.boot_id {
            return Err(invalid());
        }
        Ok(())
    }
}
impl WithdrawalWork {
    #[cfg(test)]
    pub(crate) fn observe_owned(
        &self,
        paths: &ProjectionPaths,
        running: &File,
        uid: u32,
    ) -> io::Result<WithdrawalObservation> {
        self.observe_with_authority_owned(paths, running, uid, self.authority.as_ref())
    }
    #[cfg(test)]
    pub(crate) fn observe_with_authority_owned(
        &self,
        paths: &ProjectionPaths,
        running: &File,
        uid: u32,
        authority: Option<&ZygiskAuthorityRecord>,
    ) -> io::Result<WithdrawalObservation> {
        self.observe_bound_owned(paths, running, uid, authority, &self.source)
    }
    pub(crate) fn observe_bound_owned(
        &self,
        paths: &ProjectionPaths,
        running: &File,
        uid: u32,
        authority: Option<&ZygiskAuthorityRecord>,
        expected_source: &ImageFence,
    ) -> io::Result<WithdrawalObservation> {
        if !self.valid(self.operation.0) || !self.retained() {
            return Err(invalid());
        }
        let boot = paths.boot()?;
        if !super::super::work::valid_boot(&boot) {
            return Err(invalid());
        }
        paths.require_source_enabled(uid)?;
        let source_directory = std::sync::Arc::new(directory(&paths.source())?);
        let source = ImageFence::inspect(&paths.source(), None, crate::payloads::Abi::Arm64, uid)?;
        if source.root != identity(&source_directory)? || !source.same_content(expected_source) {
            return Err(invalid());
        }
        source.require_running_owned(running, uid)?;
        let recipient_directory = std::sync::Arc::new(directory(&paths.active())?);
        let result = if boot == self.boot_id {
            if &source != expected_source {
                return Err(invalid());
            }
            let image = ProjectionImage::inspect_with_authority_owned(
                &paths.active(),
                &self.descriptor.published,
                uid,
                authority,
            )?;
            if image != self.active || image.directory != identity(&recipient_directory)? {
                return Err(invalid());
            }
            if let Some(authority) = authority {
                crate::zygisk::ZygiskTarget::for_revocation(&paths.active())
                    .map_err(io::Error::other)?
                    .require_authorization(authority)
                    .map_err(io::Error::other)?;
            }
            // Staging uncertainty never permits replacing/revoking this boot's code.
            // This is only its exact current code/grant, not staging confirmation.
            WithdrawalObservation::RetainedActive(ActiveProjectionReadback {
                source: source.clone(),
                image,
                source_directory,
                recipient_directory,
            })
        } else {
            if self.outcome != PhaseOutcome::Confirmed {
                return Err(invalid());
            }
            absent(&paths.staged())?;
            let image = WithdrawalImage::inspect_owned(&paths.active(), &self.descriptor, uid)?;
            if image.directory != identity(&recipient_directory)? {
                return Err(invalid());
            }
            let proof = WithdrawalBootProof {
                operation: self.operation,
                boot_id: boot.clone(),
                root: paths.root.clone(),
                source: source.clone(),
                image,
                pending_reentry: None,
            };
            if !proof.valid_for(self) {
                return Err(invalid());
            }
            proof.recheck_owned(running, uid)?;
            WithdrawalObservation::Withdrawn(proof)
        };
        source.require_unchanged_owned(&paths.source(), uid)?;
        if paths.boot()? != boot {
            return Err(invalid());
        }
        Ok(result)
    }
}
