//! Scoped installer publication. The installer never writes canonical/journal or
//! grants hooks. The next boot's ordinary writer alone adopts historical consent.
use super::*;
use crate::journal::{Journal, JournalEntry, ZygiskAuthorityRecord};
use std::io::Write;
use vpnhide_core_rpc::{BackendChoice, ConfigPhase, NativeBackend, PhaseOutcome};

const MAX_INTENT: u64 = 2 * 1024 * 1024;
const STORE: &str = "install-handoff";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallIntent {
    pub boot_id: String,
    pub previous_source: ImageFence,
    pub previous_manifest: String,
    pub native: Box<JournalEntry>,
    pub authority: ZygiskAuthorityRecord,
    pub incoming: ImageFence,
    pub descriptor: Descriptor,
    pub manager: super::super::manager::ManagerIdentity,
}
impl InstallIntent {
    pub(crate) fn valid(&self) -> bool {
        let Some(reference) = self.authority.reference() else {
            return false;
        };
        let entry = &self.native;
        let native_command = matches!(
            entry.command,
            Some(
                crate::journal::CommandIntent::Edit
                    | crate::journal::CommandIntent::Import { .. }
                    | crate::journal::CommandIntent::MigrateLegacy { .. }
                    | crate::journal::CommandIntent::PrepareSelfTargets
            )
        );
        let old_manifest_valid = self.previous_manifest.len()
            <= crate::payloads::MAX_MANIFEST_BYTES as usize
            && crate::payloads::Manifest::parse(self.previous_manifest.as_bytes())
                .is_ok_and(|manifest| manifest.require_complete().is_ok())
            && self
                .previous_source
                .files
                .get("payloads/manifest.json")
                .is_some_and(|file| {
                    use sha2::{Digest, Sha256};
                    file.identity.sha256
                        == format!("{:x}", Sha256::digest(self.previous_manifest.as_bytes()))
                });
        old_manifest_valid
            && native_command
            && work::valid_boot(&self.boot_id)
            && reference.boot_id == self.boot_id
            && (reference.directory_device, reference.directory_inode) == self.previous_source.root
            && entry.record.op == self.authority.operation
            && entry.native_intent.as_ref() == Some(&self.authority.intent)
            && entry.record.settled == Some(PhaseOutcome::Confirmed)
            && entry
                .record
                .phases
                .iter()
                .any(|p| p.phase == ConfigPhase::Native && p.outcome == PhaseOutcome::Confirmed)
            && !entry.superseded_phases.contains(&ConfigPhase::Native)
            && !entry.finalized_unknown
            && entry.candidate.as_ref().is_some_and(|config| {
                has_user_native(config)
                    && crate::config_file::parse_status(&crate::config_file::to_json_string(config))
                        .is_ok()
            })
            && vpnhide_protocol::parse_config(self.authority.intent.wire.as_bytes()).is_some_and(
                |wire| {
                    wire.default_mask != 0 || wire.targets.iter().any(|target| target.hookmask != 0)
                },
            )
            && self
                .previous_source
                .files
                .keys()
                .any(|name| name.starts_with("zygisk/"))
            && self
                .previous_source
                .files
                .get("payloads/manifest.json")
                .is_some_and(|manifest| {
                    self.previous_source.valid_source(&manifest.identity.sha256)
                })
            && !self
                .incoming
                .files
                .keys()
                .any(|name| name.starts_with("zygisk/"))
            && self.descriptor.validate().is_ok()
            && self.descriptor.operation == self.authority.operation.0
            && self
                .incoming
                .valid_source(&self.descriptor.manifest_file_sha256)
            && self.manager.valid()
    }
    pub(crate) fn digest(&self) -> String {
        use sha2::{Digest, Sha256};
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(self).expect("install intent"))
        )
    }
    /// Independent later user intent wins. A prior generation is never reused
    /// as this boot's authorization; this is only a bounded continuity source.
    pub(crate) fn accepts(
        &self,
        journal: &Journal,
        config: &vpnhide_core_rpc::CanonicalConfig,
    ) -> bool {
        self.valid()
            && (has_user_native(config)
                || journal.backend_choice.as_ref().is_some_and(|choice| {
                    choice.request.choice
                        == (BackendChoice::Fixed {
                            backend: NativeBackend::Zygisk,
                        })
                }))
            && !journal.has_unknown()
            && journal.backend_choice.as_ref().is_none_or(|choice| {
                choice.request.choice
                    == (BackendChoice::Fixed {
                        backend: NativeBackend::Zygisk,
                    })
            })
            && journal
                .replacement
                .as_ref()
                .is_none_or(|work| !work.retained())
            && journal.zygisk_authority.as_ref().is_some_and(|authority| {
                if journal
                    .operations
                    .iter()
                    .filter(|entry| {
                        entry
                            .record
                            .phases
                            .iter()
                            .any(|phase| phase.phase == ConfigPhase::Native)
                    })
                    .max_by_key(|entry| entry.record.op.0)
                    .is_none_or(|entry| entry.record.op != authority.operation)
                {
                    return false;
                }
                authority.reference().is_some_and(|reference| {
                    (reference.directory_device, reference.directory_inode)
                        == self.previous_source.root
                        && authority.intent.zygisk_module == self.authority.intent.zygisk_module
                        && journal.get(authority.operation).is_some_and(|entry| {
                            entry.native_intent.as_ref() == Some(&authority.intent)
                                && entry.candidate.as_ref() == Some(config)
                                && entry.record.settled == Some(PhaseOutcome::Confirmed)
                                && !entry.superseded_phases.contains(&ConfigPhase::Native)
                                && entry.record.phases.iter().any(|p| {
                                    p.phase == ConfigPhase::Native
                                        && p.outcome == PhaseOutcome::Confirmed
                                })
                        })
                })
            })
    }
}

/// The immutable installer intent and optional completed publication readback
/// remain historical even when independent new-boot readback permits adoption.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallContinuity {
    pub intent: InstallIntent,
    pub published: Option<ProjectionImage>,
    pub adopted_in_boot: String,
}
impl InstallContinuity {
    pub(crate) fn valid(&self, work: &work::ProjectionWork) -> bool {
        self.intent.valid()
            && work::valid_boot(&self.adopted_in_boot)
            && self.adopted_in_boot != self.intent.boot_id
            && self.intent.boot_id == work.boot_id
            && self.intent.descriptor == work.descriptor
            && self.intent.incoming.same_content(&work.source)
            && self.intent.manager == work.manager
            && work.operation.0 > self.intent.authority.operation.0
            && self.published.as_ref().is_none_or(|image| image.valid_for(&work.descriptor))
            && work.legacy_adoption.is_none() && work.previous_withdrawal.is_none()
            && work.request.acknowledged_warning.is_none()
            && work.dispatch_intent && work.outcome == PhaseOutcome::Confirmed && work.failure.is_none()
            && work.staging.as_ref().is_some_and(|stage| {
                stage.staged.is_none() && matches!(&stage.active, Some(ActiveProjection::Complete { image }) if image.valid_for(&work.descriptor))
            })
    }
}

fn private(paths: &ProjectionPaths, uid: u32) -> io::Result<File> {
    let parent = directory(&paths.persistent)?;
    let meta = parent.metadata()?;
    if meta.uid() != uid || meta.mode() & 0o077 != 0 {
        return Err(invalid());
    }
    let store = super::super::directory(&parent, STORE, true)?;
    let meta = store.metadata()?;
    if meta.uid() != uid || meta.mode() & 0o077 != 0 {
        return Err(invalid());
    }
    Ok(store)
}
fn write_once(root: &File, name: &str, value: &impl Serialize) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    if bytes.len() as u64 > MAX_INTENT {
        return Err(invalid());
    }
    let path = PathBuf::from(format!("/proc/self/fd/{}", root.as_raw_fd())).join(name);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    root.sync_all()
}
fn read<T: serde::de::DeserializeOwned>(root: &File, name: &str, uid: u32) -> io::Result<T> {
    let (data, _) = bytes(root, name, MAX_INTENT, uid, Instant::now() + BUDGET)?;
    serde_json::from_slice(&data).map_err(io::Error::other)
}

/// One fixed managed slot, no nested manager or execution of an installed
/// script. No replacement/cleanup of an occupied slot, even on interrupted retry.
/// The outer root manager owns Core installation; this is its Rust-only,
/// root-owned inert companion publication step, not a second policy writer.
fn publish(
    paths: &ProjectionPaths,
    store: &File,
    intent: &InstallIntent,
    files: &BTreeMap<String, Vec<u8>>,
    uid: u32,
) -> io::Result<()> {
    absent(&paths.active())?;
    absent(&paths.staged())?;
    let parent = directory(paths.staged().parent().ok_or_else(invalid)?)?;
    let meta = parent.metadata()?;
    if meta.uid() != uid || meta.mode() & 0o022 != 0 {
        return Err(invalid());
    }
    let temp = tempfile::Builder::new()
        .prefix(".pair-")
        .tempdir_in(format!("/proc/self/fd/{}", store.as_raw_fd()))?;
    for (name, data) in files {
        let path = temp.path().join(name);
        fs::create_dir_all(path.parent().ok_or_else(invalid)?)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o644)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)?;
        file.write_all(data)?;
        companion_metadata(&file, false)?;
        file.sync_all()?;
    }
    for path in [temp.path().join("zygisk"), temp.path().to_path_buf()] {
        let dir = directory(&path)?;
        companion_metadata(&dir, true)?;
        dir.sync_all()?;
    }
    let prepared = ProjectionImage::inspect_owned(temp.path(), &intent.descriptor, uid)?;
    if !prepared.valid_for(&intent.descriptor) {
        return Err(invalid());
    }
    intent
        .previous_source
        .require_unchanged_owned(&paths.source(), uid)?;
    super::super::rename_no_replace(
        store,
        temp.path()
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(invalid)?,
        &parent,
        MANAGED_MODULE_ID,
    )?;
    parent.sync_all()?;
    store.sync_all()?;
    // The old private temp name is gone; TempDir's cleanup cannot remove the
    // renamed module. The durable intent predates this publication boundary.
    Ok(())
}

pub(crate) fn prepare_owned(
    paths: &ProjectionPaths,
    incoming: &Path,
    running: &File,
    journal: &Journal,
    uid: u32,
) -> io::Result<InstallIntent> {
    if incoming != paths.root.join("data/adb/modules_update/vpnhide_core") {
        return Err(invalid());
    }
    for path in [paths.source(), incoming.to_path_buf()] {
        for marker in ["disable", "remove", ".ksu_hot_install"] {
            absent(&path.join(marker))?;
        }
    }
    let authority = journal.zygisk_authority.clone().ok_or_else(invalid)?;
    if authority.intent.zygisk_module.as_ref() != Some(&paths.source()) {
        return Err(invalid());
    }
    let native = journal
        .get(authority.operation)
        .cloned()
        .ok_or_else(invalid)?;
    let previous_source =
        ImageFence::inspect(&paths.source(), None, crate::payloads::Abi::Arm64, uid)?;
    passive_layout(incoming, uid)?;
    absent(&incoming.join("zygisk"))?;
    let (prop, _) = bytes(
        &directory(incoming)?,
        "module.prop",
        vpnhide_protocol::zygisk_activation::MAX_MODULE_PROP_BYTES as u64,
        uid,
        Instant::now() + BUDGET,
    )?;
    if recipient_from_manifest(&prop) != Some(Recipient::Authorized) {
        return Err(invalid());
    }
    let incoming_image = ImageFence::inspect(incoming, None, crate::payloads::Abi::Arm64, uid)?;
    incoming_image.require_running_owned(running, uid)?;
    let catalog =
        Catalog::open_pinned_before(&directory(incoming)?, Some(Instant::now() + BUDGET))?;
    catalog.manifest.require_complete()?;
    let manifest = fs::read(incoming.join("payloads/manifest.json"))?;
    let intent = InstallIntent {
        boot_id: paths.boot()?,
        previous_source,
        previous_manifest: fs::read_to_string(paths.source().join("payloads/manifest.json"))?,
        native: Box::new(native),
        authority,
        incoming: incoming_image,
        descriptor: Descriptor::bind(
            &manifest,
            journal
                .zygisk_authority
                .as_ref()
                .ok_or_else(invalid)?
                .operation
                .0,
        )?,
        manager: Manager::discover_owned(&paths.root, uid)?.identity,
    };
    if !intent.valid()
        || !intent.accepts(
            journal,
            intent.native.candidate.as_ref().ok_or_else(invalid)?,
        )
    {
        return Err(invalid());
    }
    let target =
        crate::zygisk::ZygiskTarget::for_revocation(&paths.source()).map_err(io::Error::other)?;
    target
        .require_authorization(&intent.authority)
        .map_err(io::Error::other)?;
    intent
        .previous_source
        .require_unchanged_owned(&paths.source(), uid)?;
    Ok(intent)
}

pub(crate) fn dispatch_owned(
    paths: &ProjectionPaths,
    incoming: &Path,
    running: &File,
    previous_running: &File,
    journal_path: &Path,
    uid: u32,
    mut checkpoint: impl FnMut(&str) -> io::Result<()>,
) -> io::Result<()> {
    let before = crate::journal::read(journal_path)?;
    let config = crate::config_file::read(&paths.root.join("data/system/vpnhide_config.json"))?
        .ok_or_else(invalid)?;
    let (source, _source_leases) = ImageFence::lease_owned(&paths.source(), uid)?;
    let (target, _target_leases) = ImageFence::lease_owned(incoming, uid)?;
    let intent = prepare_owned(paths, incoming, running, &before, uid)?;
    source.require_running_owned(previous_running, uid)?;
    if source != intent.previous_source
        || target != intent.incoming
        || !intent.accepts(&before, &config)
    {
        return Err(invalid());
    }
    let source_directory = directory(&paths.source())?;
    let _grant_lease = open_beneath(
        &source_directory,
        vpnhide_protocol::zygisk_activation::FILE_NAME,
    )?;
    read_file(
        &_grant_lease,
        uid,
        vpnhide_protocol::zygisk_activation::MAX_BYTES as u64,
        Instant::now() + BUDGET,
    )?;
    if identity(&source_directory)? != intent.previous_source.root {
        return Err(invalid());
    }
    let store = private(paths, uid)?;
    let _lock = crate::instance::InstanceGuard::acquire([paths
        .persistent
        .join(STORE)
        .join("installer.lock")])?;
    // Never replay an ambiguous publication. An exact existing intent may be
    // completed by readback only; changed source/package or absent stage refuses.
    match open_beneath(&store, "intent.json") {
        Ok(_) => {
            let old: InstallIntent = read(&store, "intent.json", uid)?;
            if !old.valid()
                || !old.incoming.same_content(&intent.incoming)
                || old.previous_source != intent.previous_source
                || !old.accepts(&before, &config)
            {
                return Err(invalid());
            }
            match ProjectionImage::inspect_owned(&paths.staged(), &old.descriptor, uid) {
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    ProjectionImage::inspect_owned(&paths.active(), &old.descriptor, uid)?;
                }
                Err(error) => return Err(error),
            }
            return Ok(());
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let root = directory(incoming)?;
    let mut libraries = Vec::new();
    for payload in &intent.descriptor.payloads {
        let (data, _) = bytes(
            &root,
            &payload.implementation.relative_path()?,
            64 * 1024 * 1024,
            uid,
            Instant::now() + BUDGET,
        )?;
        libraries.push(data);
    }
    let files = intent.descriptor.files(&libraries[0], &libraries[1])?;
    if crate::journal::read(journal_path)? != before
        || crate::config_file::read(&paths.root.join("data/system/vpnhide_config.json"))?.as_ref()
            != Some(&config)
    {
        return Err(invalid());
    }
    intent.incoming.require_unchanged_owned(incoming, uid)?;
    passive_layout(incoming, uid)?;
    let parent = directory(paths.staged().parent().ok_or_else(invalid)?)?;
    if parent.metadata()?.dev() != store.metadata()?.dev() {
        return Err(io::Error::other(
            "installer handoff requires same-filesystem atomic staging",
        ));
    }
    write_once(&store, "intent.json", &intent)?;
    checkpoint("intent")?;
    if crate::journal::read(journal_path)? != before
        || crate::config_file::read(&paths.root.join("data/system/vpnhide_config.json"))?.as_ref()
            != Some(&config)
    {
        return Err(invalid());
    }
    crate::zygisk::ZygiskTarget::for_revocation(&paths.source())
        .map_err(io::Error::other)?
        .require_authorization(&intent.authority)
        .map_err(io::Error::other)?;
    source.require_running_owned(previous_running, uid)?;
    intent.incoming.require_unchanged_owned(incoming, uid)?;
    passive_layout(incoming, uid)?;
    publish(paths, &store, &intent, &files, uid)?;
    checkpoint("published")?;
    intent.incoming.require_unchanged_owned(incoming, uid)?;
    passive_layout(incoming, uid)?;
    intent
        .previous_source
        .require_unchanged_owned(&paths.source(), uid)?;
    let image = ProjectionImage::inspect_owned(&paths.staged(), &intent.descriptor, uid)?;
    write_once(&store, "published.json", &image)?;
    checkpoint("receipt")
}

pub fn run(incoming: &Path) -> io::Result<()> {
    if unsafe { libc::geteuid() } != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "installer requires root",
        ));
    }
    let paths = ProjectionPaths {
        root: "/".into(),
        persistent: crate::service::PERSIST_DIR.into(),
    };
    // Ordinary passive/fresh installs have no continuity effects or private
    // directory creation. Root autoload alone is not proof of prior use.
    if fs::symlink_metadata(paths.source().join("zygisk"))
        .is_err_and(|e| e.kind() == io::ErrorKind::NotFound)
    {
        if fs::symlink_metadata(paths.active()).is_ok()
            || fs::symlink_metadata(paths.staged()).is_ok()
        {
            let journal = crate::journal::read(&paths.persistent.join("operations.json"))?;
            let source =
                ImageFence::inspect(&paths.source(), None, crate::payloads::Abi::Arm64, 0)?;
            let incoming_image =
                ImageFence::inspect(incoming, None, crate::payloads::Abi::Arm64, 0)?;
            if source.same_content(&incoming_image) {
                return Ok(());
            }
            let replacement = journal.replacement.as_ref().ok_or_else(|| {
                io::Error::other(
                    "Managed Zygisk requires the paired Core updater; existing runtime retained",
                )
            })?;
            if !replacement.dispatch_intent
                || (replacement.projection.is_none() && replacement.withdrawn_source.is_none())
                || replacement.before != source
                || replacement.boot_id != paths.boot()?
            {
                return Err(invalid());
            }
            let verified = vpnhide_package_trust::TrustStore::compiled()
                .and_then(|trust| trust.authenticate(&replacement.request.descriptor))
                .map_err(io::Error::other)?;
            ImageFence::inspect_replacement(incoming, &verified, 0)?;
        }
        return Ok(());
    }
    let journal = crate::journal::read(&paths.persistent.join("operations.json"))?;
    if unused_root_owned(&paths, &journal)? {
        return Ok(());
    }
    let running = File::open("/proc/self/exe")?;
    let previous_running = previous_executable(&paths)?;
    dispatch_owned(
        &paths,
        incoming,
        &running,
        &previous_running,
        &paths.persistent.join("operations.json"),
        0,
        |_| Ok(()),
    )
}

pub(crate) fn load_owned(
    paths: &ProjectionPaths,
    uid: u32,
) -> io::Result<Option<(InstallIntent, Option<ProjectionImage>)>> {
    let path = paths.persistent.join(STORE);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
        Ok(_) => {}
    }
    let store = private(paths, uid)?;
    let intent: InstallIntent = read(&store, "intent.json", uid)?;
    if !intent.valid() {
        return Err(invalid());
    }
    let published = match read(&store, "published.json", uid) {
        Ok(image) => Some(image),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    Ok(Some((intent, published)))
}

pub(crate) struct InstallReadback {
    pub origin: InstallContinuity,
    pub source: ImageFence,
    pub image: ProjectionImage,
    source_directory: File,
    recipient_directory: File,
}
impl InstallReadback {
    pub(crate) fn prepare_owned(
        paths: &ProjectionPaths,
        running: &File,
        uid: u32,
    ) -> io::Result<Option<Self>> {
        let Some((intent, published)) = load_owned(paths, uid)? else {
            return Ok(None);
        };
        let boot = paths.boot()?;
        if boot == intent.boot_id {
            return Err(io::Error::other("install continuity requires a new boot"));
        }
        paths.require_source_enabled(uid)?;
        passive_layout(&paths.source(), uid)?;
        absent(&paths.staged())?;
        let source_directory = directory(&paths.source())?;
        let recipient_directory = directory(&paths.active())?;
        let source = ImageFence::inspect(&paths.source(), None, crate::payloads::Abi::Arm64, uid)?;
        source.require_running_owned(running, uid)?;
        if source.root != identity(&source_directory)? || !source.same_content(&intent.incoming) {
            return Err(invalid());
        }
        let image = ProjectionImage::inspect_owned(&paths.active(), &intent.descriptor, uid)?;
        if image.directory != identity(&recipient_directory)?
            || published.as_ref().is_some_and(|old| {
                old.files.len() != image.files.len()
                    || old.files.iter().any(|(name, file)| {
                        image.files.get(name).is_none_or(|actual| {
                            actual.identity != file.identity || actual.mode != file.mode
                        })
                    })
            })
        {
            return Err(invalid());
        }
        let readback = Self {
            origin: InstallContinuity {
                intent,
                published,
                adopted_in_boot: boot,
            },
            source,
            image,
            source_directory,
            recipient_directory,
        };
        readback.require_owned(paths, running, uid)?;
        Ok(Some(readback))
    }
    pub(crate) fn require_owned(
        &self,
        paths: &ProjectionPaths,
        running: &File,
        uid: u32,
    ) -> io::Result<()> {
        if paths.boot()? != self.origin.adopted_in_boot
            || identity(&self.source_directory)? != self.source.root
            || identity(&self.recipient_directory)? != self.image.directory
        {
            return Err(invalid());
        }
        paths.require_source_enabled(uid)?;
        passive_layout(&paths.source(), uid)?;
        absent(&paths.staged())?;
        self.source.require_unchanged_owned(&paths.source(), uid)?;
        self.source.require_running_owned(running, uid)?;
        if ProjectionImage::inspect_owned(&paths.active(), &self.origin.intent.descriptor, uid)?
            != self.image
        {
            return Err(invalid());
        }
        Ok(())
    }
}

fn companion_metadata(file: &File, is_directory: bool) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    #[cfg(target_os = "android")]
    {
        if unsafe { libc::fchown(file.as_raw_fd(), 0, 0) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let label = c"u:object_r:system_file:s0";
        if unsafe {
            libc::fsetxattr(
                file.as_raw_fd(),
                c"security.selinux".as_ptr(),
                label.as_ptr().cast(),
                label.to_bytes_with_nul().len(),
                0,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    file.set_permissions(fs::Permissions::from_mode(if is_directory {
        0o755
    } else {
        0o644
    }))
}

pub(crate) fn valid_receipt(entry: &crate::journal::JournalEntry) -> bool {
    let Some(crate::journal::CommandIntent::AdoptInstallHandoff { intent_sha256 }) = &entry.command
    else {
        return true;
    };
    intent_sha256.len() == 64
        && intent_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && entry.edit == vpnhide_core_rpc::ConfigEdit::RepairActivation
        && entry.admission.as_ref().is_some_and(|a| {
            a.from_revision == entry.base.revision
                && a.commit_revision == entry.base.revision
                && a.writes.is_empty()
        })
        && entry.candidate.is_none()
        && entry.native_intent.is_none()
        && entry.maintenance.is_none()
        && entry.maintenance_report.is_none()
        && !entry.finalized_unknown
        && entry.superseded_by.is_none()
        && entry.superseded_phases.is_empty()
        && entry.record.settled == Some(PhaseOutcome::Confirmed)
        && entry.record.phases.len() == 1
        && entry.record.phases[0].phase == ConfigPhase::Maintenance
        && entry.record.phases[0].outcome == PhaseOutcome::Confirmed
}
#[cfg(test)]
pub(crate) mod tests;

/// Peer identity is provided by the kernel; no old RPC schema or executable is
/// invoked. Connecting then closing without a request cannot start root work.
fn previous_executable(paths: &ProjectionPaths) -> io::Result<File> {
    let socket = paths
        .root
        .join("data/user_de/0/dev.okhsunrog.vpnhide/vpnhide-core.sock");
    tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?
        .block_on(async {
            let peer = tokio::time::timeout(
                Duration::from_secs(1),
                tokio::net::UnixStream::connect(socket),
            )
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "old Core peer unavailable"))??;
            let credentials = crate::auth::peer_cred(&peer)?;
            if credentials.uid != 0 || credentials.pid == 0 || credentials.pid > i32::MAX as u32 {
                return Err(invalid());
            }
            File::open(paths.root.join(format!("proc/{}/exe", credentials.pid)))
        })
}

/// A diagnostic self target is not user intent to introduce injection.
pub(crate) fn has_user_native(config: &vpnhide_core_rpc::CanonicalConfig) -> bool {
    config.apps.iter().any(|(package, app)| {
        package != vpnhide_migration_policy::APP_PACKAGE && app.native.enabled
    })
}
fn unused_root_owned(paths: &ProjectionPaths, journal: &Journal) -> io::Result<bool> {
    if journal.has_unknown()
        || journal.zygisk_projection.is_some()
        || journal.backend_choice.as_ref().is_some_and(|choice| {
            choice.request.choice
                == (BackendChoice::Fixed {
                    backend: NativeBackend::Zygisk,
                })
        })
    {
        return Ok(false);
    }
    let config = crate::config_file::read(&paths.root.join("data/system/vpnhide_config.json"))?;
    let latest = journal
        .operations
        .iter()
        .filter(|entry| {
            entry
                .record
                .phases
                .iter()
                .any(|phase| phase.phase == ConfigPhase::Native)
        })
        .max_by_key(|entry| entry.record.op.0);
    let confirmed = latest.is_some_and(|entry| {
        entry.record.settled == Some(PhaseOutcome::Confirmed)
            && entry.candidate == config
            && entry.record.phases.iter().any(|phase| {
                phase.phase == ConfigPhase::Native && phase.outcome == PhaseOutcome::Confirmed
            })
    });
    let non_zygisk = confirmed
        && latest
            .and_then(|entry| entry.native_intent.as_ref())
            .is_some_and(|intent| intent.backend != NativeBackend::Zygisk);
    let no_user_native = config
        .as_ref()
        .is_none_or(|config| !has_user_native(config));
    if !non_zygisk && !(no_user_native && (confirmed || latest.is_none())) {
        return Ok(false);
    }
    let target =
        crate::zygisk::ZygiskTarget::for_revocation(&paths.source()).map_err(io::Error::other)?;
    if target.authorization_absent().map_err(io::Error::other)? {
        return Ok(true);
    }
    // Keep an exact owned inactive/diagnostic grant unchanged. This does not
    // assert process exit; only the subsequent real boot removes old autoload.
    let authority = journal.retained_zygisk_authorities().find(|authority| {
        authority.intent.zygisk_module.as_ref() == Some(&paths.source())
            && target.require_authorization(authority).is_ok()
    });
    Ok(authority.is_some_and(|authority| {
        no_user_native
            || authority
                .reference()
                .is_some_and(|reference| reference.boot_id != paths.boot().unwrap_or_default())
    }))
}

fn passive_layout(path: &Path, uid: u32) -> io::Result<()> {
    let root = directory(path)?;
    let catalog = Catalog::open_pinned_before(&root, Some(Instant::now() + BUDGET))?;
    let allowed = vpnhide_artifact_model::core_layout::CoreLayout::Installed(&catalog.manifest)
        .allowed_files()?;
    let mut large = catalog
        .manifest
        .payloads
        .iter()
        .map(|payload| payload.implementation.relative_path())
        .collect::<io::Result<BTreeSet<_>>>()?;
    large.insert("vpnhide-core".into());
    require_layout(&root, &allowed, &large, uid, Instant::now() + BUDGET)
}
