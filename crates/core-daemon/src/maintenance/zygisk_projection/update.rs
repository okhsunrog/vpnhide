//! Companion replacement uses only authenticated incoming Core bytes. Old code
//! and its exact grant remain active until the manager's normal boot promotion.
use super::withdrawal::PreparedWithdrawal;
use super::*;
use crate::journal::{Journal, ZygiskAuthorityRecord};
use crate::maintenance::{package::PreparedPackage, replacement::ReplacementWork};
use vpnhide_core_rpc::{CoreMaintenanceStage, OperationId, PhaseOutcome};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionUpdate {
    pub previous: Descriptor,
    pub history: Vec<Descriptor>,
    pub active: ProjectionImage,
    pub authority: Option<ZygiskAuthorityRecord>,
    pub descriptor: Descriptor,
    pub dispatch_intent: bool,
    pub staged: Option<ProjectionImage>,
}
impl ProjectionUpdate {
    pub(crate) fn valid(&self, replacement: &ReplacementWork) -> bool {
        !self.history.is_empty()
            && self.history.len() <= 32
            && self.history.last() == Some(&self.previous)
            && self
                .history
                .iter()
                .all(|descriptor| descriptor.validate().is_ok())
            && self
                .history
                .windows(2)
                .all(|pair| pair[0].operation < pair[1].operation)
            && self.previous.validate().is_ok()
            && self.active.valid_for(&self.previous)
            && self.descriptor.validate().is_ok()
            && self.descriptor.operation == replacement.status.operation.0
            && self.descriptor.operation > self.previous.operation
            && self.descriptor.source_commit == replacement.request.expected_commit
            && self.authority.as_ref().is_none_or(|a| {
                a.reference().is_some_and(|reference| {
                    a.operation.0 < self.descriptor.operation
                        && (reference.directory_device, reference.directory_inode)
                            == self.active.directory
                })
            })
            && self
                .staged
                .as_ref()
                .is_none_or(|image| self.dispatch_intent && image.valid_for(&self.descriptor))
            && (!matches!(
                replacement.status.stage,
                CoreMaintenanceStage::PendingRestart
                    | CoreMaintenanceStage::Verified
                    | CoreMaintenanceStage::RetiringLegacy
            ) || self.staged.is_some())
    }
    pub(crate) fn require_old_owned(&self, root: &Path, uid: u32) -> io::Result<()> {
        // Policy may legitimately advance while a verified replacement waits
        // for the user's reboot. Boot binding separately checks the current
        // journal-owned grant; the update's old snapshot remains historical.
        let path = root.join("data/adb/modules").join(MANAGED_MODULE_ID);
        let observed = ProjectionImage::inspect_code_owned(&path, &self.previous, uid)?;
        if observed != self.active {
            return Err(invalid());
        }
        Ok(())
    }
    pub(crate) fn require_new_owned(&self, root: &Path, uid: u32) -> io::Result<()> {
        let path = root.join("data/adb/modules").join(MANAGED_MODULE_ID);
        let observed = ProjectionImage::inspect_owned(&path, &self.descriptor, uid)?;
        if self.staged.as_ref().is_none_or(|image| {
            image.descriptor != observed.descriptor
                || image.files.len() != observed.files.len()
                || image.files.iter().any(|(name, file)| {
                    observed.files.get(name).is_none_or(|actual| {
                        file.identity != actual.identity || file.mode != actual.mode
                    })
                })
        }) {
            return Err(invalid());
        }
        absent(&root.join("data/adb/modules_update").join(MANAGED_MODULE_ID))
    }
    pub(crate) fn linked(&self, journal: &Journal) -> bool {
        journal.zygisk_projection.as_ref().is_some_and(|published| {
            published.outcome == PhaseOutcome::Confirmed
                && self.history.first() == Some(&published.descriptor)
        })
    }
}

pub struct PreparedProjectionUpdate {
    pub work: ProjectionUpdate,
    previous: PreparedWithdrawal,
    paths: ProjectionPaths,
    package: FrozenPayload,
    uid: u32,
}
impl PreparedProjectionUpdate {
    pub fn prepare(
        paths: ProjectionPaths,
        op: OperationId,
        journal: &Journal,
        package: &PreparedPackage,
    ) -> io::Result<Self> {
        let previous = PreparedWithdrawal::prepare(paths.clone(), op.0, journal)?;
        Self::prepare_from_previous(paths, op, journal, package, previous, 0)
    }
    fn prepare_from_previous(
        paths: ProjectionPaths,
        op: OperationId,
        journal: &Journal,
        package: &PreparedPackage,
        previous: PreparedWithdrawal,
        uid: u32,
    ) -> io::Result<Self> {
        let files = package.verified_files().map_err(io::Error::other)?;
        let descriptor = Descriptor::bind(
            files.get("payloads/manifest.json").ok_or_else(invalid)?,
            op.0,
        )?;
        let mut libraries = Vec::new();
        for payload in &descriptor.payloads {
            libraries.push(
                files
                    .get(&payload.implementation.relative_path()?)
                    .ok_or_else(invalid)?
                    .clone(),
            );
        }
        let mut libraries = libraries.into_iter();
        let files = descriptor.files_owned(
            libraries.next().ok_or_else(invalid)?,
            libraries.next().ok_or_else(invalid)?,
        )?;
        let package = seal_files(&directory(&paths.persistent)?, "projection-update-", &files)?;
        Ok(Self {
            work: ProjectionUpdate {
                history: journal
                    .replacement
                    .as_ref()
                    .and_then(|replacement| {
                        replacement
                            .projection
                            .as_ref()
                            .map(|update| (replacement.status.stage, update))
                    })
                    .map_or_else(
                        || vec![previous.descriptor.published.clone()],
                        |(stage, update)| {
                            let mut history = update.history.clone();
                            if stage == CoreMaintenanceStage::Verified {
                                history.push(update.descriptor.clone());
                            }
                            history
                        },
                    ),
                previous: previous.descriptor.published.clone(),
                active: previous.active.clone(),
                authority: previous.authority.clone(),
                descriptor,
                dispatch_intent: false,
                staged: None,
            },
            previous,
            paths,
            package,
            uid,
        })
    }
    pub fn require_fresh(&self) -> io::Result<()> {
        self.previous.require_fresh()
    }
    pub fn require_active(&self) -> io::Result<()> {
        self.previous.require_active()
    }
    pub fn package(&self) -> &File {
        self.package.file()
    }
    pub fn verify_staged(&self) -> io::Result<ProjectionImage> {
        self.require_active()?;
        let image =
            ProjectionImage::inspect_owned(&self.paths.staged(), &self.work.descriptor, self.uid)?;
        self.require_active()?;
        Ok(image)
    }
}

#[cfg(test)]
pub(crate) mod tests;
