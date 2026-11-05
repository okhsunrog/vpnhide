//! Authenticated Core source replacement after an already verified discovery
//! withdrawal. The tombstone stays inert; no companion code is reintroduced.
use super::*;
use crate::journal::Journal;
use crate::maintenance::{
    image::ImageFence,
    replacement::{ReplacementPaths, ReplacementWork},
};
use vpnhide_core_rpc::{CoreMaintenanceStage, OperationId, PhaseOutcome};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WithdrawnSource {
    /// Exact previously authenticated source generations, captured only while
    /// preparing a signed replacement against the current withdrawal proof.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_history: Vec<ImageFence>,
    pub operation: OperationId,
    pub descriptor: WithdrawalDescriptor,
    pub image: WithdrawalImage,
    pub origin_source: ImageFence,
    pub history: Vec<Descriptor>,
}
impl WithdrawnSource {
    pub(crate) fn valid(&self, replacement: &ReplacementWork) -> bool {
        self.source_history.len() <= 32
            && self.source_history.iter().all(|source| {
                source
                    .files
                    .get("payloads/manifest.json")
                    .is_some_and(|manifest| source.valid_source(&manifest.identity.sha256))
            })
            && self.operation.0 == self.descriptor.operation
            && self.operation.0 < replacement.status.operation.0
            && self.image.valid_for(&self.descriptor)
            && self
                .origin_source
                .valid_source(&self.descriptor.published.manifest_file_sha256)
            && !self.history.is_empty()
            && self.history.len() <= 32
            && self.history.last() == Some(&self.descriptor.published)
            && self
                .history
                .iter()
                .all(|descriptor| descriptor.validate().is_ok())
            && self
                .history
                .windows(2)
                .all(|pair| pair[0].operation < pair[1].operation)
    }
    pub(crate) fn linked(&self, journal: &Journal) -> bool {
        (self.source_history.is_empty() || journal.version >= 21)
            && journal.zygisk_withdrawal.as_ref().is_some_and(|work| {
                work.outcome == PhaseOutcome::Confirmed
                    && self.operation == work.operation
                    && self.descriptor == work.descriptor
                    && self.origin_source == work.source
            })
            && journal
                .zygisk_projection
                .as_ref()
                .is_some_and(|work| self.history.first() == Some(&work.descriptor))
    }
    pub(crate) fn capture(
        paths: &ReplacementPaths,
        journal: &Journal,
        source: &ImageFence,
    ) -> io::Result<Self> {
        Self::capture_owned(paths, journal, source, 0)
    }
    pub(crate) fn capture_owned(
        paths: &ReplacementPaths,
        journal: &Journal,
        source: &ImageFence,
        uid: u32,
    ) -> io::Result<Self> {
        let work = journal.zygisk_withdrawal.as_ref().ok_or_else(invalid)?;
        let boot = paths.boot()?;
        if work.outcome != PhaseOutcome::Confirmed
            || work.boot_id == boot
            || journal.zygisk_exposed_in(&boot)
            || !source.same_content(journal.withdrawal_source(work))
        {
            return Err(invalid());
        }
        let image = WithdrawalImage::inspect_owned(
            &paths.root.join("data/adb/modules").join(MANAGED_MODULE_ID),
            &work.descriptor,
            uid,
        )?;
        absent(
            &paths
                .root
                .join("data/adb/modules_update")
                .join(MANAGED_MODULE_ID),
        )?;
        let history = match journal.replacement.as_ref() {
            Some(replacement) if replacement.withdrawn_source.is_some() => replacement
                .withdrawn_source
                .as_ref()
                .ok_or_else(invalid)?
                .history
                .clone(),
            Some(replacement) if replacement.projection.is_some() => {
                let update = replacement.projection.as_ref().ok_or_else(invalid)?;
                let mut history = update.history.clone();
                if replacement.status.stage == CoreMaintenanceStage::Verified {
                    history.push(update.descriptor.clone());
                }
                history
            }
            _ => vec![work.descriptor.published.clone()],
        };
        let mut source_history = journal
            .replacement
            .as_ref()
            .and_then(|replacement| replacement.withdrawn_source.as_ref())
            .map_or_else(Vec::new, |prior| prior.source_history.clone());
        if !source_history
            .iter()
            .any(|prior| prior.same_content(source))
        {
            source_history.push(source.clone());
        }
        if source_history.len() > 32 {
            return Err(invalid());
        }
        Ok(Self {
            source_history,
            operation: work.operation,
            descriptor: work.descriptor.clone(),
            image,
            origin_source: work.source.clone(),
            history,
        })
    }
    pub(crate) fn require_unchanged_owned(&self, root: &Path, uid: u32) -> io::Result<()> {
        let current = WithdrawalImage::inspect_owned(
            &root.join("data/adb/modules").join(MANAGED_MODULE_ID),
            &self.descriptor,
            uid,
        )?;
        if current != self.image {
            return Err(invalid());
        }
        absent(&root.join("data/adb/modules_update").join(MANAGED_MODULE_ID))
    }
}
