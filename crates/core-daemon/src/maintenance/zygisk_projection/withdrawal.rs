//! Prepare a boot-bound discovery withdrawal without revoking, disabling or
//! removing the active runtime. Readback is packaging evidence, not hook exit.
use super::*;
use crate::journal::ZygiskAuthorityRecord;
use vpnhide_artifact_model::zygisk_projection::{WITHDRAWAL_FILE_NAME, WithdrawalDescriptor};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WithdrawalImage {
    pub directory: (u64, u64),
    pub descriptor: WithdrawalDescriptor,
    pub metadata: ImageFile,
}
impl WithdrawalImage {
    pub fn inspect(path: &Path, descriptor: &WithdrawalDescriptor) -> io::Result<Self> {
        Self::inspect_owned(path, descriptor, 0)
    }
    pub(super) fn inspect_owned(
        path: &Path,
        descriptor: &WithdrawalDescriptor,
        uid: u32,
    ) -> io::Result<Self> {
        descriptor.validate()?;
        let root = directory(path)?;
        let stat = root.metadata()?;
        if stat.uid() != uid || stat.mode() & 0o022 != 0 {
            return Err(invalid());
        }
        let deadline = Instant::now() + BUDGET;
        let allowed = BTreeSet::from([
            "module.prop".into(),
            WITHDRAWAL_FILE_NAME.into(),
            "update".into(),
        ]);
        require_layout(&root, &allowed, &BTreeSet::new(), uid, deadline)?;
        managed_properties(&root, uid, deadline)?;
        let (raw, metadata) = bytes(
            &root,
            WITHDRAWAL_FILE_NAME,
            MAX_DESCRIPTOR_BYTES as u64,
            uid,
            deadline,
        )?;
        if WithdrawalDescriptor::parse(&raw)? != *descriptor {
            return Err(invalid());
        }
        require_layout(&root, &allowed, &BTreeSet::new(), uid, deadline)?;
        if identity(&directory(path)?)? != identity(&root)? {
            return Err(invalid());
        }
        Ok(Self {
            directory: identity(&root)?,
            descriptor: descriptor.clone(),
            metadata,
        })
    }
    pub(crate) fn valid_for(&self, descriptor: &WithdrawalDescriptor) -> bool {
        self.directory.1 > 0
            && self.descriptor == *descriptor
            && descriptor.validate().is_ok()
            && self.metadata.valid(MAX_DESCRIPTOR_BYTES as u64)
    }
}

pub struct PreparedWithdrawal {
    pub descriptor: WithdrawalDescriptor,
    pub source: ImageFence,
    pub active: ProjectionImage,
    pub authority: Option<ZygiskAuthorityRecord>,
    pub manager: Manager,
    paths: ProjectionPaths,
    uid: u32,
    boot: String,
    running: File,
    _source: File,
    _recipient: File,
    package: FrozenPayload,
}
impl PreparedWithdrawal {
    pub fn prepare(
        paths: ProjectionPaths,
        operation: u64,
        journal: &crate::journal::Journal,
    ) -> io::Result<Self> {
        let manager = Manager::discover(&paths.root)?;
        let running = File::open("/proc/self/exe")?;
        Self::prepare_owned(paths, operation, journal, manager, running, 0)
    }
    pub(crate) fn prepare_owned(
        paths: ProjectionPaths,
        operation: u64,
        journal: &crate::journal::Journal,
        manager: Manager,
        running: File,
        uid: u32,
    ) -> io::Result<Self> {
        let published = journal.zygisk_projection.as_ref().ok_or_else(invalid)?;
        if published.outcome != vpnhide_core_rpc::PhaseOutcome::Confirmed
            || !super::work::validate_journal(journal)
            || operation > journal.issued_through
        {
            return Err(invalid());
        }
        let (current_descriptor, current_source, _) =
            journal.projection_identity().ok_or_else(invalid)?;
        let descriptor = WithdrawalDescriptor::bind(current_descriptor.clone(), operation)?;
        let boot = paths.boot()?;
        paths.require_source_enabled(uid)?;
        absent(&paths.staged())?;
        let source_directory = directory(&paths.source())?;
        let source = ImageFence::inspect(&paths.source(), None, crate::payloads::Abi::Arm64, uid)?;
        if identity(&source_directory)? != source.root || !source.same_content(current_source) {
            return Err(invalid());
        }
        source.require_running_owned(&running, uid)?;
        let recipient = directory(&paths.active())?;
        let target = crate::zygisk::ZygiskTarget::for_revocation(&paths.active())
            .map_err(io::Error::other)?;
        let authority = if target.authorization_absent().map_err(io::Error::other)? {
            None
        } else {
            crate::native_boot::transition::revocable_authority(journal, &paths.active())
                .map_err(|_| invalid())?
        };
        let active = ProjectionImage::inspect_with_authority_owned(
            &paths.active(),
            &descriptor.published,
            uid,
            authority.as_ref(),
        )?;
        if identity(&recipient)? != active.directory {
            return Err(invalid());
        }
        let persistent = directory(&paths.persistent)?;
        let stat = persistent.metadata()?;
        if stat.uid() != uid || stat.mode() & 0o077 != 0 {
            return Err(invalid());
        }
        let package = seal_files(&persistent, "withdrawal-prepare-", &descriptor.files()?)?;
        let prepared = Self {
            descriptor,
            source,
            active,
            authority,
            manager,
            paths,
            uid,
            boot,
            running,
            _source: source_directory,
            _recipient: recipient,
            package,
        };
        prepared.require_fresh()?;
        Ok(prepared)
    }
    pub fn package(&self) -> &File {
        self.package.file()
    }
    pub fn boot_id(&self) -> &str {
        &self.boot
    }
    pub(crate) fn require_active(&self) -> io::Result<()> {
        if paths_changed(self)? {
            return Err(invalid());
        }
        self.paths.require_source_enabled(self.uid)?;
        self.source
            .require_unchanged_owned(&self.paths.source(), self.uid)?;
        self.source.require_running_owned(&self.running, self.uid)?;
        let active = ProjectionImage::inspect_with_authority_owned(
            &self.paths.active(),
            &self.descriptor.published,
            self.uid,
            self.authority.as_ref(),
        )?;
        if active != self.active {
            return Err(invalid());
        }
        let target = crate::zygisk::ZygiskTarget::for_revocation(&self.paths.active())
            .map_err(io::Error::other)?;
        match &self.authority {
            Some(authority) => target
                .require_authorization(authority)
                .map_err(io::Error::other)?,
            None if !target.authorization_absent().map_err(io::Error::other)? => {
                return Err(invalid());
            }
            None => {}
        }
        self.manager.require_unchanged()
    }
    pub fn require_fresh(&self) -> io::Result<()> {
        self.require_active()?;
        absent(&self.paths.staged())
    }
    /// Full old code and exact prior authorization must remain active. A
    /// tombstone in active, missing stage or exit0 alone cannot confirm staging.
    pub fn verify_staged(&self) -> io::Result<WithdrawalImage> {
        self.require_active()?;
        let image =
            WithdrawalImage::inspect_owned(&self.paths.staged(), &self.descriptor, self.uid)?;
        self.require_active()?;
        if !image.valid_for(&self.descriptor) {
            return Err(invalid());
        }
        Ok(image)
    }
}
fn paths_changed(prepared: &PreparedWithdrawal) -> io::Result<bool> {
    Ok(prepared.paths.boot()? != prepared.boot
        || identity(&prepared._source)? != prepared.source.root
        || identity(&prepared._recipient)? != prepared.active.directory)
}

pub(crate) mod followup;
pub(crate) mod observation;
pub mod source;
#[cfg(test)]
pub(crate) mod tests;
pub mod work;
