//! Root companion observations are separate from authorization and hook health.
//! No writable heartbeat, installed module name or current pathname proves which
//! library image a process loaded. Unknown evidence never promotes availability.

use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use vpnhide_loader_ipc::{
    Acknowledgement, CORE_SOCKET, Challenge, CompanionReport, Credentials, VERSION, WORKER_PROCESS,
    image::{self, ImageAbi, ProcessIdentity},
    transport,
};

use crate::payloads::{Abi, Catalog, MAX_MANIFEST_BYTES, Manifest, PayloadKind};

#[derive(Debug, Clone)]
pub struct LoaderObservation {
    pub process: ProcessIdentity,
    pub zygote: ProcessIdentity,
    zygote_owner: u32,
    pub companion: ProcessIdentity,
    pub companion_anchor: u64,
    child_anchor: u64,
    child_mapping: image::Mapping,
    image: Arc<File>,
    // Retain the verified inode through process churn; prevent directory ABA.
    _recipient_directory: Arc<File>,
    mapping: image::Mapping,
    pub target_uid: u32,
    pub process_name: String,
    pub abi: ImageAbi,
    pub payload_sha256: String,
    pub payload_bytes: u64,
    pub manifest_sha256: String,
    pub boot_id: String,
    pub epoch: u64,
    pub directory_identity: (u64, u64),
    pub catalog_directory_identity: (u64, u64),
    pub generation: Option<u64>,
    pub policy_sha256: Option<[u8; 32]>,
    pub specialized: bool,
}

#[derive(Debug, Default)]
pub struct LoaderRegistry {
    context: Mutex<Option<LoaderContext>>,
    observations: Mutex<Vec<LoaderObservation>>,
    probe: Mutex<()>,
}

#[derive(Debug, Clone)]
struct LoaderContext {
    /// Trusted source of the manifest and payload identities.
    root: PathBuf,
    /// Provider module directory, independently pinned from the source catalog.
    recipient_root: PathBuf,
    boot_id: String,
    epoch: u64,
    catalog: Arc<Mutex<Option<CatalogGeneration>>>,
    binding: Option<BindingExpectation>,
}

#[derive(Debug, Clone)]
struct BindingExpectation {
    owner_uid: u32,
    source: (u64, u64),
    recipient: (u64, u64),
    descriptor: vpnhide_artifact_model::zygisk_projection::Descriptor,
    // Keep the recipient generation alive across cache/context churn.
    _recipient: Arc<File>,
}

#[derive(Debug, Clone)]
struct CatalogGeneration {
    directory: (u64, u64),
    // This lease pins the inode for the entire cached generation.
    _directory: Arc<File>,
    manifest: Manifest,
    sha256: String,
    file_sha256: String,
}

fn pinned_generation(
    context: &LoaderContext,
    directory: &File,
    deadline: Instant,
) -> io::Result<CatalogGeneration> {
    if Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "loader catalog deadline",
        ));
    }
    let metadata = directory.metadata()?;
    let identity = (metadata.dev(), metadata.ino());
    if context
        .binding
        .as_ref()
        .is_some_and(|binding| metadata.uid() != binding.owner_uid || metadata.mode() & 0o022 != 0)
    {
        return Err(invalid("managed source ownership changed"));
    }
    if !metadata.is_dir() || directory_identity(&open_directory(&context.root)?)? != identity {
        return Err(invalid(
            "catalog descriptor differs from authoritative source",
        ));
    }
    let mut cache = context
        .catalog
        .try_lock()
        .map_err(|_| io::Error::new(io::ErrorKind::WouldBlock, "catalog verification busy"))?;
    let generation = if let Some(generation) = cache.as_ref() {
        if generation.directory != identity {
            return Err(invalid("loader catalog directory generation changed"));
        }
        generation.clone()
    } else {
        let catalog = Catalog::open_pinned_before(directory, Some(deadline))?;
        catalog.manifest.require_complete()?;
        CatalogGeneration {
            directory: identity,
            _directory: Arc::new(directory.try_clone()?),
            sha256: manifest_sha256(&catalog),
            file_sha256: catalog.manifest_sha256,
            manifest: catalog.manifest,
        }
    };
    let mut bytes = Vec::new();
    crate::payloads::open_beneath(directory, "payloads/manifest.json")?
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if format!("{:x}", Sha256::digest(&bytes)) != generation.file_sha256
        || Manifest::parse(&bytes)? != generation.manifest
    {
        return Err(invalid("loader manifest changed after verification"));
    }
    if Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "loader catalog deadline",
        ));
    }
    if context.binding.as_ref().is_some_and(|binding| {
        generation.directory != binding.source
            || generation.sha256 != binding.descriptor.manifest_sha256
            || generation.file_sha256 != binding.descriptor.manifest_file_sha256
            || generation.manifest.bundle_commit != binding.descriptor.source_commit
    }) {
        return Err(invalid(
            "managed catalog differs from its owned projection binding",
        ));
    }
    *cache = Some(generation.clone());
    Ok(generation)
}

fn open_directory(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
}
fn directory_identity(directory: &File) -> io::Result<(u64, u64)> {
    let metadata = directory.metadata()?;
    if !metadata.is_dir() {
        return Err(invalid("expected directory"));
    }
    Ok((metadata.dev(), metadata.ino()))
}

enum RecipientAccess {
    Live,
    // Disabling/removing future discovery does not unload existing hooks.
    Retained,
}

struct LoaderSources {
    catalog_directory: File,
    recipient: crate::zygisk::ZygiskTarget,
    recipient_identity: (u64, u64),
    generation: CatalogGeneration,
}
impl LoaderSources {
    fn open(
        context: &LoaderContext,
        received: &File,
        deadline: Instant,
        access: RecipientAccess,
    ) -> io::Result<Self> {
        let recipient_identity = directory_identity(received)?;
        let recipient = match access {
            RecipientAccess::Live => crate::zygisk::ZygiskTarget::open(&context.recipient_root),
            RecipientAccess::Retained => {
                crate::zygisk::ZygiskTarget::for_revocation(&context.recipient_root)
            }
        }
        .map_err(io::Error::other)?;
        if recipient.identity().map_err(io::Error::other)? != recipient_identity {
            return Err(invalid(
                "callback recipient differs from expected recipient",
            ));
        }
        if context.binding.as_ref().is_some_and(|binding| {
            binding.recipient != recipient_identity
                || !received
                    .metadata()
                    .is_ok_and(|m| m.uid() == binding.owner_uid && m.mode() & 0o022 == 0)
        }) {
            return Err(invalid(
                "recipient differs from the journal-qualified projection",
            ));
        }
        // The companion supplies only its recipient. Open the authoritative
        // source ourselves, never a peer-selected embedded catalog.
        let catalog_directory = open_directory(&context.root)?;
        let generation = pinned_generation(context, &catalog_directory, deadline)?;
        let sources = Self {
            catalog_directory,
            recipient,
            recipient_identity,
            generation,
        };
        sources.revalidate(context, deadline)?;
        Ok(sources)
    }
    fn revalidate(&self, context: &LoaderContext, deadline: Instant) -> io::Result<()> {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "loader source deadline",
            ));
        }
        if self
            .recipient
            .revalidate_identity()
            .map_err(io::Error::other)?
            != self.recipient_identity
        {
            return Err(invalid("recipient changed during source inspection"));
        }
        if context.binding.as_ref().is_some_and(|binding| {
            !binding
                ._recipient
                .metadata()
                .is_ok_and(|m| m.uid() == binding.owner_uid && m.mode() & 0o022 == 0)
        }) {
            return Err(invalid("managed recipient ownership changed"));
        }
        let generation = pinned_generation(context, &self.catalog_directory, deadline)?;
        if generation.directory != self.generation.directory
            || generation.file_sha256 != self.generation.file_sha256
        {
            return Err(invalid("authoritative source changed during inspection"));
        }
        Ok(())
    }
}

struct Verification<'a> {
    context: &'a LoaderContext,
    challenge: &'a Challenge,
    proc_root: &'a Path,
    deadline: Instant,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub fn manifest_sha256(catalog: &Catalog) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&catalog.manifest).expect("manifest serialization"))
    )
}

pub(crate) fn parent_zygote(
    proc_root: &Path,
    child: ProcessIdentity,
) -> io::Result<ProcessIdentity> {
    parent_zygote_owned(proc_root, child, 0)
}
fn parent_zygote_owned(
    proc_root: &Path,
    child: ProcessIdentity,
    owner: u32,
) -> io::Result<ProcessIdentity> {
    let stat = image::bounded_text(&proc_root.join(child.pid.to_string()).join("stat"), 8192)?;
    let (_, fields) = stat
        .rsplit_once(") ")
        .ok_or_else(|| invalid("invalid child stat"))?;
    let parent: u32 = fields
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| invalid("missing zygote parent"))?
        .parse()
        .map_err(|_| invalid("invalid zygote parent"))?;
    let path = proc_root.join(parent.to_string());
    if fs::metadata(&path)?.uid() != owner {
        return Err(invalid("parent is not root zygote"));
    }
    let cmdline = image::bounded_text(&path.join("cmdline"), 256)?;
    if !matches!(
        cmdline.split('\0').next(),
        Some("zygote" | "zygote32" | "zygote64")
    ) {
        return Err(invalid("parent is not a zygote"));
    }
    image::process_identity(proc_root, parent)
}

fn verify_report(
    verification: &Verification<'_>,
    peer: Credentials,
    report: &CompanionReport,
    image_file: &File,
    directory: &File,
) -> io::Result<LoaderObservation> {
    let proc_root = verification.proc_root;
    let challenge = verification.challenge;
    if peer.uid != 0
        || report.child.uid != 0
        || peer.pid == report.child.pid
        || !challenge.valid()
        || !report.callback.valid()
        || report.callback.nonce != challenge.nonce
    {
        return Err(invalid("untrusted loader observation or stale challenge"));
    }
    let child = image::process_identity(proc_root, report.child.pid)?;
    let companion = image::process_identity(proc_root, peer.pid)?;
    let zygote = parent_zygote(proc_root, child)?;
    let child_map = image::mapping(proc_root, child.pid, report.callback.anchor)?;
    let companion_map = image::mapping(proc_root, companion.pid, report.companion_anchor)?;
    if !image::matches(image_file, child_map)? || !image::matches(image_file, companion_map)? {
        return Err(invalid(
            "callback/companion do not map the retained library image",
        ));
    }
    let sources = LoaderSources::open(
        verification.context,
        directory,
        verification.deadline,
        RecipientAccess::Live,
    )?;
    let generation = &sources.generation;
    let identity = image::inspect_before(image_file, Some(verification.deadline))?;
    let digest = identity
        .sha256
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let abi = match identity.abi {
        ImageAbi::Arm64 => Abi::Arm64,
        ImageAbi::Arm32 => Abi::Arm32,
    };
    if !generation.manifest.payloads.iter().any(|p| {
        p.implementation == PayloadKind::Zygisk { abi }
            && p.sha256 == digest
            && p.bytes == identity.bytes
    }) {
        return Err(invalid(
            "actual loaded image is not the active catalog payload",
        ));
    }
    let callback = &report.callback;
    if let Some(generation) = callback.generation {
        let mut bytes = Vec::new();
        crate::payloads::open_beneath(directory, "zygisk-activation.txt")?
            .take(vpnhide_protocol::zygisk_activation::MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        let authorization =
            vpnhide_protocol::zygisk_activation::Authorization::decode(&bytes, &challenge.boot_id)
                .ok_or_else(|| invalid("invalid current authorization"))?;
        let expected: [u8; 32] = Sha256::digest(authorization.wire.as_bytes()).into();
        if authorization.generation != generation || callback.policy_sha256 != Some(expected) {
            return Err(invalid(
                "consumed generation/policy differs from current authorization",
            ));
        }
    } else {
        let owner = fs::metadata(crate::service::APP_DATA_DIR)?.uid();
        if callback.process != WORKER_PROCESS || callback.target_uid != owner {
            return Err(invalid("inert loader probe is not the fixed app worker"));
        }
    }
    if child != image::process_identity(proc_root, child.pid)?
        || companion != image::process_identity(proc_root, companion.pid)?
        || zygote != parent_zygote(proc_root, child)?
    {
        return Err(invalid("loader process identity changed during inspection"));
    }
    sources.revalidate(verification.context, verification.deadline)?;
    Ok(LoaderObservation {
        process: child,
        zygote,
        zygote_owner: 0,
        companion,
        companion_anchor: report.companion_anchor,
        child_anchor: callback.anchor,
        child_mapping: child_map,
        image: Arc::new(image_file.try_clone()?),
        _recipient_directory: Arc::new(directory.try_clone()?),
        mapping: companion_map,
        target_uid: callback.target_uid,
        process_name: callback.process.clone(),
        abi: identity.abi,
        payload_sha256: digest,
        payload_bytes: identity.bytes,
        manifest_sha256: generation.sha256.clone(),
        boot_id: challenge.boot_id.clone(),
        epoch: challenge.epoch,
        directory_identity: sources.recipient_identity,
        catalog_directory_identity: generation.directory,
        generation: callback.generation,
        policy_sha256: callback.policy_sha256,
        specialized: false,
    })
}

fn live_identity(o: &LoaderObservation, root: &Path, child_mapping: bool) -> bool {
    let identity = |p: ProcessIdentity| image::process_identity(root, p.pid).ok() == Some(p);
    identity(o.process) && identity(o.zygote) && identity(o.companion)
        && fs::metadata(root.join(o.process.pid.to_string())).is_ok_and(|m|m.uid()==o.target_uid)
        && fs::metadata(root.join(o.zygote.pid.to_string())).is_ok_and(|m|m.uid()==o.zygote_owner)
        && fs::metadata(root.join(o.companion.pid.to_string())).is_ok_and(|m|m.uid()==o.zygote_owner)
        && image::bounded_text(&root.join(o.process.pid.to_string()).join("cmdline"),512)
            .is_ok_and(|s|s.split('\0').next()==Some(o.process_name.as_str()))
        && parent_zygote_owned(root,o.process,o.zygote_owner).ok()==Some(o.zygote)
        && image::mapping(root,o.companion.pid,o.companion_anchor).is_ok_and(|m|m==o.mapping)
        && (!child_mapping || image::mapping(root,o.process.pid,o.child_anchor).is_ok_and(|m|m==o.child_mapping))
        // Proc paths can be reused while the preceding fields are read.
        && identity(o.process) && identity(o.zygote) && identity(o.companion)
}

impl LoaderRegistry {
    #[cfg(test)]
    pub(crate) fn configure_test_context(&self, root: PathBuf, boot_id: String, epoch: u64) {
        *self.context.lock().expect("loader context") = Some(LoaderContext {
            recipient_root: root.clone(),
            root,
            boot_id,
            epoch,
            catalog: Arc::new(Mutex::new(None)),
            binding: None,
        });
    }
    pub(crate) fn bind_projection_owned(
        &self,
        source: &Path,
        recipient: &Path,
        readback: &crate::maintenance::zygisk_projection::work::ActiveProjectionReadback,
        uid: u32,
        boot: &str,
        epoch: u64,
    ) -> io::Result<()> {
        let previous = self
            .context
            .lock()
            .expect("loader context")
            .clone()
            .ok_or_else(|| invalid("loader listener unavailable"))?;
        if previous.boot_id != boot
            || previous.epoch != epoch
            || source != previous.root
            || source
                .parent()
                .map(|p| p.join(vpnhide_protocol::zygisk_activation::MANAGED_MODULE_ID))
                .as_deref()
                != Some(recipient)
        {
            return Err(invalid(
                "projection is outside the configured source/module namespace",
            ));
        }
        let image = &readback.image;
        if directory_identity(&readback.source_directory)? != readback.source.root
            || directory_identity(&readback.recipient_directory)? != image.directory
        {
            return Err(invalid(
                "readback directory leases differ from its evidence",
            ));
        }
        image.descriptor.validate()?;
        if !image.valid_for(&image.descriptor) {
            return Err(invalid("projection image evidence is incomplete"));
        }
        let directory = open_directory(recipient)?;
        let metadata = directory.metadata()?;
        if metadata.uid() != uid
            || metadata.mode() & 0o022 != 0
            || directory_identity(&directory)? != image.directory
        {
            return Err(invalid("owned projection directory generation changed"));
        }
        let target = crate::zygisk::ZygiskTarget::open(recipient).map_err(io::Error::other)?;
        if !target.is_managed().map_err(io::Error::other)? {
            return Err(invalid("projection recipient contract changed"));
        }
        let source_directory = open_directory(source)?;
        if directory_identity(&source_directory)? != readback.source.root {
            return Err(invalid("source directory changed after readback"));
        }
        let next = LoaderContext {
            root: previous.root.clone(),
            recipient_root: recipient.into(),
            boot_id: previous.boot_id.clone(),
            epoch: previous.epoch,
            catalog: Arc::new(Mutex::new(None)),
            binding: Some(BindingExpectation {
                owner_uid: uid,
                source: readback.source.root,
                recipient: image.directory,
                descriptor: image.descriptor.clone(),
                _recipient: Arc::new(directory),
            }),
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        pinned_generation(&next, &source_directory, deadline)?;
        if target.revalidate_identity().map_err(io::Error::other)? != image.directory {
            return Err(invalid("projection changed during binding"));
        }
        let mut current = self.context.lock().expect("loader context");
        if current
            .as_ref()
            .is_none_or(|current| !Arc::ptr_eq(&current.catalog, &previous.catalog))
        {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "loader binding changed during qualification",
            ));
        }
        *current = Some(next);
        Ok(())
    }
    fn accept_current(
        &self,
        context: &LoaderContext,
        observation: LoaderObservation,
    ) -> io::Result<()> {
        let current = self.context.lock().expect("loader context");
        if current
            .as_ref()
            .is_none_or(|current| !Arc::ptr_eq(&current.catalog, &context.catalog))
        {
            return Err(invalid("loader binding changed during the callback"));
        }
        if observation.boot_id != context.boot_id
            || observation.epoch != context.epoch
            || context.binding.as_ref().is_some_and(|binding| {
                observation.directory_identity != binding.recipient
                    || observation.catalog_directory_identity != binding.source
                    || observation.manifest_sha256 != binding.descriptor.manifest_sha256
            })
        {
            return Err(invalid("callback is outside the bound generation"));
        }
        self.accept(observation);
        Ok(())
    }
    /// Ownership of this exact readback namespace only. This is not loader,
    /// authorization, installed-hook or behavior evidence.
    pub(crate) fn owns_projection(&self, modules: &Path) -> bool {
        let context = self.context.lock().expect("loader context");
        let Some(context) = context.as_ref() else {
            return false;
        };
        let Some(binding) = context.binding.as_ref() else {
            return false;
        };
        if context.root != modules.join("vpnhide_core")
            || context.recipient_root
                != modules.join(vpnhide_protocol::zygisk_activation::MANAGED_MODULE_ID)
        {
            return false;
        }
        let matches = |path: &Path, expected| {
            open_directory(path).is_ok_and(|file| {
                file.metadata().is_ok_and(|metadata| {
                    metadata.uid() == binding.owner_uid && metadata.mode() & 0o022 == 0
                }) && directory_identity(&file).ok() == Some(expected)
            })
        };
        matches(&context.root, binding.source)
            && matches(&context.recipient_root, binding.recipient)
    }
    pub fn enabled(&self) -> bool {
        self.context.lock().expect("loader context").is_some()
    }
    /// Promote only a process that actually became the stated app, with the same
    /// kernel starttime and live zygote parent. This is not hook effectiveness.
    fn reconcile(&self, proc_root: &Path) {
        let mut observations = self.observations.lock().expect("loader observations");
        for observation in observations.iter_mut() {
            if observation.specialized {
                continue;
            }
            let path = proc_root.join(observation.process.pid.to_string());
            let matching = image::process_identity(proc_root, observation.process.pid).ok()
                == Some(observation.process)
                && fs::metadata(&path).is_ok_and(|m| m.uid() == observation.target_uid)
                && image::bounded_text(&path.join("cmdline"), 512)
                    .is_ok_and(|s| s.split('\0').next() == Some(observation.process_name.as_str()))
                && image::process_identity(proc_root, observation.zygote.pid).ok()
                    == Some(observation.zygote);
            if matching {
                observation.specialized = true;
            }
        }
    }

    /// Loaded code identity in the live companion/zygote generations. Child
    /// PID membership belongs to policy coverage, not the heavy-run cache key:
    /// opening the UI or starting a game must not restart the fixture suite.
    pub fn generation_fingerprint(&self) -> Option<String> {
        self.generation_fingerprint_at(Path::new("/proc"))
    }
    fn generation_fingerprint_at(&self, proc_root: &Path) -> Option<String> {
        let context = self.context.lock().expect("loader context").clone()?;
        let observations = self.observations.lock().expect("loader observations");
        let material: std::collections::BTreeSet<_> = observations
            .iter()
            .filter(|o| {
                o.boot_id == context.boot_id
                    && o.epoch == context.epoch
                    && image::process_identity(proc_root, o.zygote.pid).ok() == Some(o.zygote)
                    && image::process_identity(proc_root, o.companion.pid).ok() == Some(o.companion)
                    && image::mapping(proc_root, o.companion.pid, o.companion_anchor)
                        .is_ok_and(|m| m == o.mapping)
            })
            .map(|o| {
                (
                    o.zygote.pid,
                    o.zygote.start_ticks,
                    o.companion.pid,
                    o.companion.start_ticks,
                    o.payload_sha256.clone(),
                    o.manifest_sha256.clone(),
                )
            })
            .collect();
        if material.is_empty() {
            return None;
        }
        Some(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&material).ok()?)
        ))
    }

    /// Readback confirms only future-process authority. Samples prove exactly
    /// which policy live authenticated processes consumed, never installed hooks.
    pub fn lifecycle(
        &self,
        record: &crate::journal::ZygiskAuthorityRecord,
        epoch: u64,
    ) -> Option<vpnhide_core_rpc::ZygiskLifecycle> {
        use vpnhide_core_rpc::ZygiskPublication as P;
        let expected = record.reference()?;
        let mut result = vpnhide_core_rpc::ZygiskLifecycle::pending(expected.clone(), epoch);
        let mut time: libc::timespec = unsafe { std::mem::zeroed() };
        if unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut time) } == 0 && time.tv_sec >= 0
        {
            result.observed_at_boot_ms =
                Some((time.tv_sec as u64).saturating_mul(1000) + (time.tv_nsec as u64) / 1_000_000);
        }
        let boot = crate::native_boot::read_boot_id(Path::new(crate::native_boot::BOOT_ID_PATH));
        if boot != expected.boot_id {
            result.publication = P::PriorBoot;
            return Some(result);
        }
        let module = record.intent.zygisk_module.as_ref()?;
        result.publication = (|| {
            let target = crate::zygisk::ZygiskTarget::for_revocation(module).ok()?;
            if target.identity().ok()? != (expected.directory_device, expected.directory_inode) {
                return None;
            }
            match target.read_authorized(&boot).ok()? {
                None => Some(P::Absent),
                Some(a) if a.matches(expected.generation, &record.intent.wire) => Some(P::Matching),
                Some(_) => Some(P::Different),
            }
        })()
        .unwrap_or(P::Unknown);
        result.processes = self.consumption_at(&expected, epoch, Path::new("/proc"));
        Some(result)
    }

    fn consumption_at(
        &self,
        expected: &vpnhide_core_rpc::ZygiskAuthorizationRef,
        epoch: u64,
        proc_root: &Path,
    ) -> Vec<vpnhide_core_rpc::ZygiskProcessConsumption> {
        self.consumption_at_injected(expected, epoch, proc_root, |_| {})
    }
    fn consumption_at_injected(
        &self,
        expected: &vpnhide_core_rpc::ZygiskAuthorizationRef,
        epoch: u64,
        proc_root: &Path,
        mut before_finish: impl FnMut(&LoaderObservation),
    ) -> Vec<vpnhide_core_rpc::ZygiskProcessConsumption> {
        use vpnhide_core_rpc::{ZygiskConsumption as C, ZygiskProcessConsumption};
        self.reconcile(proc_root);
        let Some(context) = self.context.lock().expect("loader context").clone() else {
            return vec![];
        };
        if context.boot_id != expected.boot_id || context.epoch != epoch {
            return vec![];
        }
        let Ok(directory) = open_directory(&context.recipient_root) else {
            return vec![];
        };
        let deadline = Instant::now() + Duration::from_millis(200);
        let Ok(sources) =
            LoaderSources::open(&context, &directory, deadline, RecipientAccess::Retained)
        else {
            return vec![];
        };
        if sources.recipient_identity != (expected.directory_device, expected.directory_inode) {
            return vec![];
        }
        let catalog = &sources.generation;
        let records = self.observations.lock().expect("loader observations");
        let mut verified = std::collections::BTreeMap::new();
        let mut result = vec![];
        for o in records.iter().take(64) {
            if !o.specialized
                || o.boot_id != context.boot_id
                || o.epoch != epoch
                || o.directory_identity != sources.recipient_identity
                || o.catalog_directory_identity != catalog.directory
                || o.manifest_sha256 != catalog.sha256
                || !live_identity(o, proc_root, true)
            {
                continue;
            }
            let key = (
                o.mapping.device_major,
                o.mapping.device_minor,
                o.mapping.inode,
                o.payload_sha256.clone(),
            );
            if !*verified.entry(key).or_insert_with(|| {
                image::inspect_before(&o.image, Some(deadline)).is_ok_and(|id| {
                    id.abi == o.abi
                        && id.bytes == o.payload_bytes
                        && id
                            .sha256
                            .iter()
                            .map(|b| format!("{b:02x}"))
                            .collect::<String>()
                            == o.payload_sha256
                })
            }) {
                continue;
            }
            before_finish(o);
            if !live_identity(o, proc_root, true) {
                continue;
            }
            let policy = o
                .policy_sha256
                .map(|hash| hash.iter().map(|b| format!("{b:02x}")).collect::<String>());
            let consumption = match (o.generation, policy.as_ref()) {
                (Some(generation), Some(hash))
                    if generation == expected.generation && *hash == expected.policy_sha256 =>
                {
                    C::Current
                }
                (Some(_), Some(_)) => C::Stale,
                _ => C::Unattributed,
            };
            result.push(ZygiskProcessConsumption {
                pid: o.process.pid,
                start_ticks: o.process.start_ticks,
                uid: o.target_uid,
                process: o.process_name.clone(),
                abi: match o.abi {
                    ImageAbi::Arm64 => "arm64",
                    ImageAbi::Arm32 => "arm32",
                }
                .into(),
                generation: o.generation,
                policy_sha256: policy,
                consumption,
            });
        }
        if sources.revalidate(&context, deadline).is_err() {
            return vec![];
        }
        result.sort_by_key(|p| (p.pid, p.start_ticks));
        result
    }

    pub fn available(&self, catalog: &Catalog, abi: Abi, needs_arm32: bool) -> bool {
        self.available_at(catalog, abi, needs_arm32, Path::new("/proc"))
    }

    fn available_at(
        &self,
        catalog: &Catalog,
        abi: Abi,
        needs_arm32: bool,
        proc_root: &Path,
    ) -> bool {
        self.reconcile(proc_root);
        let context = self.context.lock().expect("loader context");
        let Some(context) = context.as_ref() else {
            return false;
        };
        let Ok(directory) = open_directory(&context.recipient_root) else {
            return false;
        };
        let deadline = Instant::now() + Duration::from_millis(200);
        let Ok(sources) = LoaderSources::open(context, &directory, deadline, RecipientAccess::Live)
        else {
            return false;
        };
        let required_authorization = match sources.recipient.is_managed() {
            Ok(true) => match sources.recipient.read_authorized(&context.boot_id) {
                Ok(Some(authority)) => Some(authority),
                _ => return false,
            },
            Ok(false) => None,
            Err(_) => return false,
        };
        let manifest = manifest_sha256(catalog);
        if sources.generation.directory != catalog.root_identity
            || sources.generation.sha256 != manifest
            || sources.generation.file_sha256 != catalog.manifest_sha256
        {
            return false;
        }
        let observations = self.observations.lock().expect("loader observations");
        let mut inspected = std::collections::BTreeSet::new();
        let mut proven = |abi| {
            observations.iter().any(|o| {
                o.abi == abi
                    && o.specialized
                    && o.boot_id == context.boot_id
                    && o.epoch == context.epoch
                    && o.directory_identity == sources.recipient_identity
                    && o.catalog_directory_identity == sources.generation.directory
                    && o.manifest_sha256 == manifest
                    && required_authorization.as_ref().is_none_or(|a| {
                        o.generation == Some(a.generation)
                            && o.policy_sha256 == Some(Sha256::digest(a.wire.as_bytes()).into())
                    })
                    && image::process_identity(proc_root, o.zygote.pid).ok() == Some(o.zygote)
                    && image::process_identity(proc_root, o.companion.pid).ok() == Some(o.companion)
                    && image::mapping(proc_root, o.companion.pid, o.companion_anchor)
                        .is_ok_and(|m| m == o.mapping)
                    && inspected.insert((
                        o.mapping.device_major,
                        o.mapping.device_minor,
                        o.mapping.inode,
                    ))
                    && image::inspect_before(&o.image, Some(deadline)).is_ok_and(|identity| {
                        identity.abi == o.abi
                            && identity.bytes == o.payload_bytes
                            && identity
                                .sha256
                                .iter()
                                .map(|b| format!("{b:02x}"))
                                .collect::<String>()
                                == o.payload_sha256
                    })
            })
        };
        let proven = match abi {
            Abi::Arm64 => proven(ImageAbi::Arm64) && (!needs_arm32 || proven(ImageAbi::Arm32)),
            Abi::Arm32 => proven(ImageAbi::Arm32),
        };
        proven && sources.revalidate(context, deadline).is_ok()
    }

    fn accept(&self, mut observation: LoaderObservation) {
        let mut records = self.observations.lock().expect("loader observations");
        records.retain(|o| {
            o.process != observation.process
                && image::process_identity(Path::new("/proc"), o.companion.pid).ok()
                    == Some(o.companion)
                && image::process_identity(Path::new("/proc"), o.zygote.pid).ok() == Some(o.zygote)
        });
        if let Some(previous) = records.iter().find(|o| {
            o.mapping.device_major == observation.mapping.device_major
                && o.mapping.device_minor == observation.mapping.device_minor
                && o.mapping.inode == observation.mapping.inode
                && o.payload_sha256 == observation.payload_sha256
        }) {
            observation.image = previous.image.clone();
        }
        if let Some(previous) = records
            .iter()
            .find(|o| o.directory_identity == observation.directory_identity)
        {
            observation._recipient_directory = previous._recipient_directory.clone();
        }
        // Distinct retained backing images share the bundle's byte budget;
        // repeated processes referencing one image do not allocate more images.
        loop {
            let mut images = std::collections::BTreeMap::new();
            for record in records.iter().chain(std::iter::once(&observation)) {
                images
                    .entry((
                        record.mapping.device_major,
                        record.mapping.device_minor,
                        record.mapping.inode,
                    ))
                    .or_insert(record.payload_bytes);
            }
            if images.values().copied().sum::<u64>() <= crate::payloads::MAX_BUNDLE_BYTES
                || records.is_empty()
            {
                break;
            }
            records.remove(0);
        }
        if records.len() >= 64 {
            // Retain one authenticated record per loaded image generation even
            // after the short-lived worker exits. Normal child churn must not
            // evict the sole other ABI's code identity and trigger a heavy run.
            let duplicate = records
                .iter()
                .enumerate()
                .position(|(index, record)| {
                    records.iter().enumerate().any(|(other, candidate)| {
                        index != other
                            && candidate.zygote == record.zygote
                            && candidate.companion == record.companion
                            && candidate.payload_sha256 == record.payload_sha256
                            && candidate.manifest_sha256 == record.manifest_sha256
                    })
                })
                .unwrap_or(0);
            records.remove(duplicate);
        }
        records.push(observation);
    }

    /// Off-writer adapter: fixed component, no caller-chosen commands/paths. A
    /// short allowlist may permit the background start on AOSP; ROM acceptance
    /// remains required. Its expiry is left intact for concurrent owners.
    pub fn probe_worker(&self) -> Result<(), String> {
        let _probe = self
            .probe
            .try_lock()
            .map_err(|_| "loader probe already running")?;
        let context = self
            .context
            .lock()
            .expect("loader context")
            .clone()
            .ok_or("loader listener unavailable")?;
        let _worker_lease = crate::autonomous::android_worker::lease()?;
        let target = crate::zygisk::ZygiskTarget::open(&context.recipient_root)?;
        let expected = target.read_authorized(&context.boot_id)?;
        if target.is_managed()? && expected.is_none() {
            return Err("managed projection requires journaled bootstrap authority".into());
        }
        self.reconcile(Path::new("/proc"));
        if self.live_worker_for_at(&context, Path::new("/proc"), expected.as_ref()) {
            return Ok(());
        }
        let previous: Vec<_> = self
            .observations
            .lock()
            .expect("loader observations")
            .iter()
            .map(|o| o.process)
            .collect();
        crate::autonomous::android_worker::prepare_fresh(Path::new(crate::service::APP_DATA_DIR))?;
        crate::autonomous::android_worker::launch()?;
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            self.reconcile(Path::new("/proc"));
            // The selected generation may change while Android dispatches. Do not
            // accept a stale callback merely because a new process appeared.
            if target.read_authorized(&context.boot_id)? != expected {
                return Err("authorization changed during loader probe".into());
            }
            if self.live_worker_for_at_filtered(
                &context,
                Path::new("/proc"),
                expected.as_ref(),
                &previous,
            ) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        Err("fresh Android loader worker did not produce verified specialization evidence".into())
    }

    #[cfg(test)]
    fn live_worker_at(&self, context: &LoaderContext, proc_root: &Path) -> bool {
        self.live_worker_for_at(context, proc_root, None)
    }
    fn live_worker_for_at(
        &self,
        context: &LoaderContext,
        proc_root: &Path,
        expected: Option<&vpnhide_protocol::zygisk_activation::Authorization>,
    ) -> bool {
        self.live_worker_for_at_filtered(context, proc_root, expected, &[])
    }
    fn live_worker_for_at_filtered(
        &self,
        context: &LoaderContext,
        proc_root: &Path,
        expected: Option<&vpnhide_protocol::zygisk_activation::Authorization>,
        excluded: &[ProcessIdentity],
    ) -> bool {
        let Ok(directory) = open_directory(&context.recipient_root) else {
            return false;
        };
        let deadline = Instant::now() + Duration::from_millis(200);
        let Ok(sources) = LoaderSources::open(context, &directory, deadline, RecipientAccess::Live)
        else {
            return false;
        };
        let catalog = &sources.generation;
        if sources.recipient.is_managed().unwrap_or(true) && expected.is_none() {
            return false;
        }
        let policy = match expected {
            Some(a) => match a.policy(vpnhide_protocol::hook_ids::ZYGISK_HOOK_MASK) {
                Some(policy) => Some(policy),
                None => return false,
            },
            None => None,
        };
        let records = self.observations.lock().expect("loader observations");
        records.iter().any(|o| {
            // A fixed worker may prove an explicitly empty bootstrap grant and
            // then dlclose. A protected worker must still map its hook image.
            let mapping_required = policy.as_ref().is_some_and(|p| {
                p.targets
                    .iter()
                    .any(|t| t.uid == o.target_uid && t.hookmask != 0)
            });
            !excluded.contains(&o.process)
                && o.specialized
                && match expected {
                    Some(a) => {
                        (!mapping_required
                            || image::mapping(proc_root, o.process.pid, o.child_anchor)
                                .is_ok_and(|m| m == o.child_mapping))
                            && o.generation == Some(a.generation)
                            && o.policy_sha256 == Some(Sha256::digest(a.wire.as_bytes()).into())
                    }
                    None => o.generation.is_none() && o.policy_sha256.is_none(),
                }
                && o.process_name == WORKER_PROCESS
                && o.boot_id == context.boot_id
                && o.epoch == context.epoch
                && o.directory_identity == sources.recipient_identity
                && o.catalog_directory_identity == catalog.directory
                && o.manifest_sha256 == catalog.sha256
                && live_identity(o, proc_root, mapping_required)
                && image::inspect_before(&o.image, Some(deadline)).is_ok_and(|identity| {
                    identity
                        .sha256
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>()
                        == o.payload_sha256
                })
                && live_identity(o, proc_root, mapping_required)
                && sources.revalidate(context, deadline).is_ok()
        })
    }
}

pub fn start(
    registry: Arc<LoaderRegistry>,
    root: PathBuf,
    recipient_root: PathBuf,
    boot_id: String,
    epoch: u64,
) -> io::Result<crate::ServerHandle> {
    let context = LoaderContext {
        root,
        recipient_root,
        boot_id,
        epoch,
        catalog: Arc::new(Mutex::new(None)),
        binding: None,
    };
    if context.boot_id.is_empty() || context.boot_id.len() > 128 {
        return Err(invalid("loader listener requires boot identity"));
    }
    let accepted_context = context.clone();
    let configured_registry = registry.clone();
    let budget = Arc::new(tokio::sync::Semaphore::new(2));
    let handle = crate::listener::listen_prepared(
        Path::new(CORE_SOCKET),
        |path| fs::set_permissions(path, fs::Permissions::from_mode(0o600)),
        move |stream, cancel| {
            let Ok(permit) = budget.clone().try_acquire_owned() else {
                return;
            };
            let registry = registry.clone();
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                if cancel.is_cancelled() {
                    return;
                }
                let Ok(stream) = stream.into_std() else {
                    return;
                };
                let Some(context) = registry.context.lock().expect("loader context").clone() else {
                    return;
                };
                let _ = receive_observation(stream, &context, &registry);
            });
        },
    )?;
    *configured_registry.context.lock().expect("loader context") = Some(accepted_context);
    Ok(handle)
}

fn receive_observation(
    mut stream: UnixStream,
    context: &LoaderContext,
    registry: &LoaderRegistry,
) -> io::Result<()> {
    stream.set_nonblocking(false)?;
    let peer = transport::peer_credentials(&stream)?;
    if peer.uid != 0 {
        return Err(invalid("loader observer requires root companion"));
    }
    let mut nonce = [0; 32];
    File::open("/dev/urandom")?.read_exact(&mut nonce)?;
    let challenge = Challenge {
        version: VERSION,
        nonce,
        boot_id: context.boot_id.clone(),
        epoch: context.epoch,
    };
    transport::send(&mut stream, &challenge, &[])?;
    let received = transport::receive::<CompanionReport>(&mut stream, 2, false)?;
    let mut descriptors = received.descriptors.into_iter();
    let image_file = File::from(descriptors.next().expect("transport checked image fd"));
    let directory = File::from(descriptors.next().expect("transport checked directory fd"));
    let verification = Verification {
        context,
        challenge: &challenge,
        proc_root: Path::new("/proc"),
        deadline: Instant::now() + Duration::from_secs(2),
    };
    let observation = verify_report(
        &verification,
        peer,
        &received.value,
        &image_file,
        &directory,
    )?;
    if Instant::now() >= verification.deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "loader observation expired",
        ));
    }
    registry.accept_current(context, observation)?;
    transport::send(
        &mut stream,
        &Acknowledgement {
            accepted: true,
            nonce,
        },
        &[],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::fd::FromRawFd;

    struct Mapped(*mut libc::c_void);
    impl Drop for Mapped {
        fn drop(&mut self) {
            unsafe {
                libc::munmap(self.0, 4096);
            }
        }
    }
    struct Fixture {
        _arena: tempfile::TempDir,
        _mapped: Mapped,
        context: LoaderContext,
        proc_root: PathBuf,
        catalog: Catalog,
        observation: LoaderObservation,
    }

    fn bundle(root: &Path) {
        vpnhide_artifact_model::fixtures::write_bundle_tree(root).unwrap();
    }

    fn process(root: &Path, pid: u32, ticks: u64, name: &str) {
        let path = root.join(pid.to_string());
        fs::create_dir_all(&path).unwrap();
        let mut fields = vec!["0".to_owned(); 20];
        fields[0] = "S".into();
        fields[1] = "2".into();
        fields[19] = ticks.to_string();
        fs::write(
            path.join("stat"),
            format!("{pid} (process) {}", fields.join(" ")),
        )
        .unwrap();
        fs::write(path.join("cmdline"), format!("{name}\0")).unwrap();
    }
    fn fixture() -> Fixture {
        let arena = tempfile::tempdir().unwrap();
        let root = arena.path().join("core");
        bundle(&root);
        let catalog = Catalog::open(&root).unwrap();
        let payload = catalog
            .payloads
            .iter()
            .find(|p| p.metadata.implementation == PayloadKind::Zygisk { abi: Abi::Arm64 })
            .unwrap();
        let mut bytes = Vec::new();
        payload
            .file
            .try_clone()
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        let raw = unsafe {
            libc::memfd_create(
                c"loader-registry-test".as_ptr(),
                libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
            )
        };
        assert!(raw >= 0);
        let mut image_file = unsafe { File::from_raw_fd(raw) };
        image_file.write_all(&bytes).unwrap();
        assert_eq!(
            unsafe {
                libc::fcntl(
                    raw,
                    libc::F_ADD_SEALS,
                    libc::F_SEAL_WRITE
                        | libc::F_SEAL_GROW
                        | libc::F_SEAL_SHRINK
                        | libc::F_SEAL_SEAL,
                )
            },
            0
        );
        let address = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                4096,
                libc::PROT_READ | libc::PROT_EXEC,
                libc::MAP_PRIVATE,
                raw,
                0,
            )
        };
        assert_ne!(address, libc::MAP_FAILED);
        let mapping = image::mapping(
            Path::new("/proc"),
            std::process::id(),
            address as usize as u64,
        )
        .unwrap();
        let proc_root = arena.path().join("proc");
        process(&proc_root, 2, 20, "zygote64");
        process(&proc_root, 3, 30, "companion");
        process(&proc_root, 4, 40, "not-yet-specialized");
        fs::write(
            proc_root.join("3/maps"),
            format!(
                "{:x}-{:x} r-xp 0000 {:02x}:{:02x} {} /manager-image\n",
                mapping.start,
                mapping.end,
                mapping.device_major,
                mapping.device_minor,
                mapping.inode
            ),
        )
        .unwrap();
        let directory = fs::metadata(&root).unwrap();
        let context = LoaderContext {
            recipient_root: root.clone(),
            root,
            boot_id: "boot".into(),
            epoch: 7,
            catalog: Arc::new(Mutex::new(None)),
            binding: None,
        };
        let observation = LoaderObservation {
            process: ProcessIdentity {
                pid: 4,
                start_ticks: 40,
            },
            zygote: ProcessIdentity {
                pid: 2,
                start_ticks: 20,
            },
            zygote_owner: unsafe { libc::geteuid() },
            companion: ProcessIdentity {
                pid: 3,
                start_ticks: 30,
            },
            companion_anchor: address as usize as u64,
            child_anchor: address as usize as u64,
            child_mapping: mapping,
            image: Arc::new(image_file),
            _recipient_directory: Arc::new(File::open(&context.recipient_root).unwrap()),
            mapping,
            target_uid: unsafe { libc::geteuid() },
            process_name: WORKER_PROCESS.into(),
            abi: ImageAbi::Arm64,
            payload_sha256: payload.metadata.sha256.clone(),
            payload_bytes: payload.metadata.bytes,
            manifest_sha256: manifest_sha256(&catalog),
            boot_id: "boot".into(),
            epoch: 7,
            directory_identity: (directory.dev(), directory.ino()),
            catalog_directory_identity: (directory.dev(), directory.ino()),
            generation: None,
            policy_sha256: None,
            specialized: false,
        };
        Fixture {
            _arena: arena,
            _mapped: Mapped(address),
            context,
            proc_root,
            catalog,
            observation,
        }
    }

    #[test]
    fn consumption_is_exact_live_process_evidence_not_a_fresh_worker_health_claim() {
        use vpnhide_core_rpc::ZygiskConsumption as C;
        let f = fixture();
        let registry = LoaderRegistry::default();
        *registry.context.lock().unwrap() = Some(f.context.clone());
        process(&f.proc_root, 4, 40, WORKER_PROCESS);
        let maps = fs::read(f.proc_root.join("3/maps")).unwrap();
        fs::write(f.proc_root.join("4/maps"), &maps).unwrap();
        let mut current = f.observation.clone();
        current.generation = Some(8);
        current.policy_sha256 = Some([3; 32]);
        let mut old = current.clone();
        old.process = ProcessIdentity {
            pid: 5,
            start_ticks: 50,
        };
        old.process_name = "game.old".into();
        old.generation = Some(7);
        old.policy_sha256 = Some([2; 32]);
        process(&f.proc_root, 5, 50, "game.old");
        fs::write(f.proc_root.join("5/maps"), &maps).unwrap();
        registry.observations.lock().unwrap().extend([current, old]);
        let expected = vpnhide_core_rpc::ZygiskAuthorizationRef {
            operation: 8,
            boot_id: "boot".into(),
            generation: 8,
            policy_sha256: "03".repeat(32),
            directory_device: f.observation.directory_identity.0,
            directory_inode: f.observation.directory_identity.1,
        };
        let observed = registry.consumption_at(&expected, 7, &f.proc_root);
        assert_eq!(
            observed.iter().map(|p| p.consumption).collect::<Vec<_>>(),
            vec![C::Current, C::Stale]
        );
        let mut wrong = expected.clone();
        wrong.boot_id = "other boot".into();
        assert!(registry.consumption_at(&wrong, 7, &f.proc_root).is_empty());
        wrong = expected.clone();
        wrong.directory_inode += 1;
        assert!(registry.consumption_at(&wrong, 7, &f.proc_root).is_empty());
        assert!(
            registry
                .consumption_at(&expected, 8, &f.proc_root)
                .is_empty()
        );
        fs::remove_file(f.proc_root.join("4/maps")).unwrap();
        assert_eq!(
            registry.consumption_at(&expected, 7, &f.proc_root).len(),
            1,
            "an unmapped child is not current coverage"
        );
        fs::write(f.proc_root.join("4/maps"), &maps).unwrap();
        process(&f.proc_root, 4, 41, WORKER_PROCESS);
        assert_eq!(
            registry.consumption_at(&expected, 7, &f.proc_root).len(),
            1,
            "PID reuse cannot retain current coverage"
        );
        process(&f.proc_root, 3, 31, "companion");
        assert!(
            registry
                .consumption_at(&expected, 7, &f.proc_root)
                .is_empty()
        );
    }

    #[test]
    fn a_pid_reused_during_image_validation_cannot_inherit_the_previous_generation() {
        let f = fixture();
        let registry = LoaderRegistry::default();
        *registry.context.lock().unwrap() = Some(f.context.clone());
        process(&f.proc_root, 4, 40, WORKER_PROCESS);
        fs::write(
            f.proc_root.join("4/maps"),
            fs::read(f.proc_root.join("3/maps")).unwrap(),
        )
        .unwrap();
        let mut record = f.observation.clone();
        record.generation = Some(8);
        record.policy_sha256 = Some([3; 32]);
        registry.observations.lock().unwrap().push(record);
        let expected = vpnhide_core_rpc::ZygiskAuthorizationRef {
            operation: 8,
            boot_id: "boot".into(),
            generation: 8,
            policy_sha256: "03".repeat(32),
            directory_device: f.observation.directory_identity.0,
            directory_inode: f.observation.directory_identity.1,
        };
        assert_eq!(registry.consumption_at(&expected, 7, &f.proc_root).len(), 1);
        let result = registry.consumption_at_injected(&expected, 7, &f.proc_root, |_| {
            process(&f.proc_root, 4, 41, WORKER_PROCESS)
        });
        assert!(result.is_empty());
    }

    #[test]
    fn live_worker_join_requires_exact_consumed_generation_and_policy() {
        use vpnhide_protocol::zygisk_activation::Authorization;
        let f = fixture();
        let registry = LoaderRegistry::default();
        process(&f.proc_root, 4, 40, WORKER_PROCESS);
        fs::write(
            f.proc_root.join("4/maps"),
            fs::read(f.proc_root.join("3/maps")).unwrap(),
        )
        .unwrap();
        let mut record = f.observation.clone();
        record.specialized = true;
        record.generation = Some(7);
        let mut auth = Authorization {
            boot_id: "boot".into(),
            generation: 7,
            wire: "vpnhide 2 config\ndebug 0\nend 0\n".into(),
        };
        record.policy_sha256 = Some(Sha256::digest(auth.wire.as_bytes()).into());
        registry.observations.lock().unwrap().push(record);
        assert!(registry.live_worker_for_at(&f.context, &f.proc_root, Some(&auth)));
        assert!(
            !registry.live_worker_at(&f.context, &f.proc_root),
            "an authorized worker is not an inert loader probe"
        );
        auth.generation = 8;
        assert!(!registry.live_worker_for_at(&f.context, &f.proc_root, Some(&auth)));
        auth.generation = 7;
        auth.wire.push(' ');
        assert!(!registry.live_worker_for_at(&f.context, &f.proc_root, Some(&auth)));
    }

    #[test]
    fn code_fingerprint_ignores_child_churn_but_fences_companion_restarts() {
        let f = fixture();
        let registry = LoaderRegistry::default();
        *registry.context.lock().unwrap() = Some(f.context.clone());
        assert!(registry.generation_fingerprint_at(&f.proc_root).is_none());
        registry
            .observations
            .lock()
            .unwrap()
            .push(f.observation.clone());
        let original = registry.generation_fingerprint_at(&f.proc_root).unwrap();
        let mut game = f.observation.clone();
        game.process_name = "example.game".into();
        game.process = ProcessIdentity {
            pid: 88,
            start_ticks: 900,
        };
        game.generation = Some(4);
        registry.observations.lock().unwrap().push(game);
        process(&f.proc_root, 4, 41, "dev.okhsunrog.vpnhide");
        assert_eq!(
            registry.generation_fingerprint_at(&f.proc_root),
            Some(original.clone())
        );
        fs::remove_dir_all(f.proc_root.join("4")).unwrap();
        assert_eq!(
            registry.generation_fingerprint_at(&f.proc_root),
            Some(original)
        );
        process(&f.proc_root, 3, 31, "companion");
        assert!(registry.generation_fingerprint_at(&f.proc_root).is_none());
    }

    #[test]
    fn loader_proof_requires_real_specialization_live_companion_and_required_abis() {
        let f = fixture();
        let registry = LoaderRegistry::default();
        *registry.context.lock().unwrap() = Some(f.context.clone());
        registry
            .observations
            .lock()
            .unwrap()
            .push(f.observation.clone());
        assert!(!registry.available_at(&f.catalog, Abi::Arm64, false, &f.proc_root));
        process(&f.proc_root, 4, 40, WORKER_PROCESS);
        assert!(registry.available_at(&f.catalog, Abi::Arm64, false, &f.proc_root));
        assert!(!registry.available_at(&f.catalog, Abi::Arm64, true, &f.proc_root));
        process(&f.proc_root, 3, 31, "companion");
        assert!(!registry.available_at(&f.catalog, Abi::Arm64, false, &f.proc_root));
        process(&f.proc_root, 3, 30, "companion");
        assert!(registry.available_at(&f.catalog, Abi::Arm64, false, &f.proc_root));
        fs::write(
            f.proc_root.join("3/maps"),
            "1000-2000 r-xp 0000 00:00 0 /stale\n",
        )
        .unwrap();
        assert!(!registry.available_at(&f.catalog, Abi::Arm64, false, &f.proc_root));
    }

    #[test]
    fn old_epoch_pid_reuse_and_replaced_directory_never_reuse_loader_proof() {
        let f = fixture();
        let registry = LoaderRegistry::default();
        *registry.context.lock().unwrap() = Some(f.context.clone());
        registry
            .observations
            .lock()
            .unwrap()
            .push(f.observation.clone());
        process(&f.proc_root, 4, 41, WORKER_PROCESS);
        assert!(!registry.available_at(&f.catalog, Abi::Arm64, false, &f.proc_root));
        process(&f.proc_root, 4, 40, WORKER_PROCESS);
        assert!(registry.available_at(&f.catalog, Abi::Arm64, false, &f.proc_root));
        registry.context.lock().unwrap().as_mut().unwrap().epoch = 8;
        assert!(!registry.available_at(&f.catalog, Abi::Arm64, false, &f.proc_root));
        registry.context.lock().unwrap().as_mut().unwrap().epoch = 7;
        fs::rename(&f.context.root, f.context.root.with_extension("old")).unwrap();
        bundle(&f.context.root);
        let replacement = Catalog::open(&f.context.root).unwrap();
        assert_eq!(manifest_sha256(&replacement), manifest_sha256(&f.catalog));
        assert!(!registry.available_at(&replacement, Abi::Arm64, false, &f.proc_root));
    }

    #[test]
    fn cached_catalog_is_bound_to_exact_directory_manifest_and_deadline() {
        let f = fixture();
        let directory = File::open(&f.context.root).unwrap();
        assert!(pinned_generation(&f.context, &directory, Instant::now()).is_err());
        assert!(f.context.catalog.lock().unwrap().is_none());
        pinned_generation(
            &f.context,
            &directory,
            Instant::now() + Duration::from_secs(2),
        )
        .unwrap();
        let path = f.context.root.join("payloads/manifest.json");
        let mut manifest = f.catalog.manifest.clone();
        manifest.bundle_commit = "b".repeat(40);
        fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(
            pinned_generation(
                &f.context,
                &directory,
                Instant::now() + Duration::from_secs(2)
            )
            .is_err()
        );
        fs::rename(&f.context.root, f.context.root.with_extension("old")).unwrap();
        bundle(&f.context.root);
        assert!(
            pinned_generation(
                &f.context,
                &File::open(&f.context.root).unwrap(),
                Instant::now() + Duration::from_secs(2)
            )
            .is_err()
        );
    }

    #[test]
    fn repeated_probe_joins_a_live_verified_worker_and_never_a_reused_pid() {
        let f = fixture();
        let registry = LoaderRegistry::default();
        *registry.context.lock().unwrap() = Some(f.context.clone());
        registry
            .observations
            .lock()
            .unwrap()
            .push(f.observation.clone());
        process(&f.proc_root, 4, 40, WORKER_PROCESS);
        registry.reconcile(&f.proc_root);
        assert!(registry.live_worker_at(&f.context, &f.proc_root));
        assert!(registry.live_worker_at(&f.context, &f.proc_root));
        process(&f.proc_root, 4, 41, WORKER_PROCESS);
        assert!(!registry.live_worker_at(&f.context, &f.proc_root));
    }
    fn split_recipient(f: &mut Fixture) {
        let recipient = f._arena.path().join("recipient");
        bundle(&recipient);
        fs::remove_dir_all(f.context.root.join("zygisk")).unwrap();
        let metadata = fs::metadata(&recipient).unwrap();
        f.context.recipient_root = recipient;
        f.observation._recipient_directory =
            Arc::new(File::open(&f.context.recipient_root).unwrap());
        f.observation.directory_identity = (metadata.dev(), metadata.ino());
        assert_ne!(
            f.observation.directory_identity,
            f.observation.catalog_directory_identity
        );
    }

    #[test]
    fn separate_recipient_never_supplies_or_replaces_the_authoritative_catalog() {
        let mut f = fixture();
        split_recipient(&mut f);
        let recipient = File::open(&f.context.recipient_root).unwrap();
        let path = f.context.recipient_root.join("payloads/manifest.json");
        let mut fake = f.catalog.manifest.clone();
        fake.bundle_commit = "f".repeat(40);
        fs::write(path, serde_json::to_vec(&fake).unwrap()).unwrap();
        let sources = LoaderSources::open(
            &f.context,
            &recipient,
            Instant::now() + Duration::from_secs(2),
            RecipientAccess::Live,
        )
        .unwrap();
        assert_eq!(sources.generation.sha256, manifest_sha256(&f.catalog));
        assert_eq!(
            sources.generation.directory,
            f.observation.catalog_directory_identity
        );
        assert_eq!(sources.recipient_identity, f.observation.directory_identity);
        assert!(
            pinned_generation(
                &f.context,
                &recipient,
                Instant::now() + Duration::from_secs(2)
            )
            .is_err()
        );
        assert!(
            LoaderSources::open(
                &f.context,
                &File::open(&f.context.root).unwrap(),
                Instant::now() + Duration::from_secs(2),
                RecipientAccess::Live
            )
            .is_err()
        );
        fs::write(f.context.root.join("payloads/manifest.json"), b"{}").unwrap();
        assert!(
            LoaderSources::open(
                &f.context,
                &recipient,
                Instant::now() + Duration::from_secs(2),
                RecipientAccess::Live
            )
            .is_err()
        );
    }

    #[test]
    fn both_independent_directory_generations_are_required_for_availability() {
        for replace_source in [false, true] {
            let mut f = fixture();
            split_recipient(&mut f);
            let registry = LoaderRegistry::default();
            *registry.context.lock().unwrap() = Some(f.context.clone());
            registry
                .observations
                .lock()
                .unwrap()
                .push(f.observation.clone());
            process(&f.proc_root, 4, 40, WORKER_PROCESS);
            assert!(registry.available_at(&f.catalog, Abi::Arm64, false, &f.proc_root));
            assert!(!registry.available_at(&f.catalog, Abi::Arm64, true, &f.proc_root));
            assert!(registry.live_worker_at(&f.context, &f.proc_root));
            let path = if replace_source {
                &f.context.root
            } else {
                &f.context.recipient_root
            };
            fs::rename(path, path.with_extension("old")).unwrap();
            bundle(path);
            assert!(!registry.available_at(&f.catalog, Abi::Arm64, false, &f.proc_root));
            assert!(!registry.live_worker_at(&f.context, &f.proc_root));
        }
    }

    #[test]
    fn exact_manifest_bytes_are_required_even_if_normalized_metadata_is_unchanged() {
        let mut f = fixture();
        split_recipient(&mut f);
        let registry = LoaderRegistry::default();
        *registry.context.lock().unwrap() = Some(f.context.clone());
        registry
            .observations
            .lock()
            .unwrap()
            .push(f.observation.clone());
        process(&f.proc_root, 4, 40, WORKER_PROCESS);
        assert!(registry.available_at(&f.catalog, Abi::Arm64, false, &f.proc_root));
        fs::write(
            f.context.root.join("payloads/manifest.json"),
            serde_json::to_vec_pretty(&f.catalog.manifest).unwrap(),
        )
        .unwrap();
        assert!(!registry.available_at(&f.catalog, Abi::Arm64, false, &f.proc_root));
        assert!(!registry.live_worker_at(&f.context, &f.proc_root));
    }

    #[test]
    fn disabled_discovery_keeps_qualified_consumption_of_existing_processes() {
        let mut f = fixture();
        split_recipient(&mut f);
        let registry = LoaderRegistry::default();
        *registry.context.lock().unwrap() = Some(f.context.clone());
        process(&f.proc_root, 4, 40, WORKER_PROCESS);
        fs::write(
            f.proc_root.join("4/maps"),
            fs::read(f.proc_root.join("3/maps")).unwrap(),
        )
        .unwrap();
        let mut observed = f.observation.clone();
        observed.generation = Some(8);
        observed.policy_sha256 = Some([3; 32]);
        registry.observations.lock().unwrap().push(observed);
        let expected = vpnhide_core_rpc::ZygiskAuthorizationRef {
            operation: 8,
            boot_id: "boot".into(),
            generation: 8,
            policy_sha256: "03".repeat(32),
            directory_device: f.observation.directory_identity.0,
            directory_inode: f.observation.directory_identity.1,
        };
        fs::write(f.context.recipient_root.join("disable"), b"").unwrap();
        assert!(!registry.available_at(&f.catalog, Abi::Arm64, false, &f.proc_root));
        assert_eq!(
            registry.consumption_at(&expected, 7, &f.proc_root)[0].consumption,
            vpnhide_core_rpc::ZygiskConsumption::Current
        );
        fs::remove_dir_all(f.context.recipient_root.join("zygisk")).unwrap();
        assert_eq!(registry.consumption_at(&expected, 7, &f.proc_root).len(), 1);
    }
    #[test]
    fn managed_availability_requires_current_consumption_and_empty_bootstrap_is_not_a_hook_claim() {
        use vpnhide_protocol::zygisk_activation::Authorization;
        let mut f = fixture();
        split_recipient(&mut f);
        f.context.boot_id = "12345678-1234-1234-1234-123456789abc".into();
        f.observation.boot_id = f.context.boot_id.clone();
        fs::write(
            f.context.recipient_root.join("module.prop"),
            vpnhide_artifact_model::zygisk_projection::Descriptor::module_prop(),
        )
        .unwrap();
        let registry = LoaderRegistry::default();
        *registry.context.lock().unwrap() = Some(f.context.clone());
        registry
            .observations
            .lock()
            .unwrap()
            .push(f.observation.clone());
        process(&f.proc_root, 4, 40, WORKER_PROCESS);
        assert!(!registry.available_at(&f.catalog, Abi::Arm64, false, &f.proc_root));
        let auth = Authorization {
            boot_id: f.context.boot_id.clone(),
            generation: 8,
            wire: "vpnhide 2 config\ndebug 0\ndefault 0\nend 0\n".into(),
        };
        let path = f
            .context
            .recipient_root
            .join(vpnhide_protocol::zygisk_activation::FILE_NAME);
        fs::write(&path, auth.encode().unwrap()).unwrap();
        assert!(!registry.available_at(&f.catalog, Abi::Arm64, false, &f.proc_root));
        let mut records = registry.observations.lock().unwrap();
        records[0].generation = Some(8);
        records[0].policy_sha256 = Some(Sha256::digest(auth.wire.as_bytes()).into());
        drop(records);
        assert!(registry.available_at(&f.catalog, Abi::Arm64, false, &f.proc_root));
        assert!(!registry.live_worker_at(&f.context, &f.proc_root));
        // No child maps file: the empty grant requests no worker hooks.
        assert!(registry.live_worker_for_at(&f.context, &f.proc_root, Some(&auth)));
        let protected = Authorization {
            wire: vpnhide_protocol::format_config(
                false,
                0,
                &[vpnhide_protocol::Target {
                    uid: f.observation.target_uid,
                    hookmask: vpnhide_protocol::hook_ids::ZYGISK_HOOK_MASK,
                }],
            ),
            ..auth.clone()
        };
        assert!(protected.encode().is_some());
        registry.observations.lock().unwrap()[0].policy_sha256 =
            Some(Sha256::digest(protected.wire.as_bytes()).into());
        fs::write(&path, protected.encode().unwrap()).unwrap();
        assert!(!registry.live_worker_for_at(&f.context, &f.proc_root, Some(&protected)));
        fs::write(
            f.proc_root.join("4/maps"),
            fs::read(f.proc_root.join("3/maps")).unwrap(),
        )
        .unwrap();
        assert!(registry.live_worker_for_at(&f.context, &f.proc_root, Some(&protected)));
        fs::write(
            &path,
            Authorization {
                generation: 9,
                ..auth
            }
            .encode()
            .unwrap(),
        )
        .unwrap();
        assert!(!registry.available_at(&f.catalog, Abi::Arm64, false, &f.proc_root));
    }
    fn managed_image(
        f: &Fixture,
    ) -> (
        PathBuf,
        crate::maintenance::zygisk_projection::work::ActiveProjectionReadback,
    ) {
        use vpnhide_artifact_model::zygisk_projection::Descriptor;
        let descriptor = Descriptor::bind(
            &fs::read(f.context.root.join("payloads/manifest.json")).unwrap(),
            8,
        )
        .unwrap();
        let files = descriptor
            .files(
                &fs::read(f.context.root.join("payloads/zygisk/arm64-v8a.so")).unwrap(),
                &fs::read(f.context.root.join("payloads/zygisk/armeabi-v7a.so")).unwrap(),
            )
            .unwrap();
        let recipient = f
            .context
            .root
            .parent()
            .unwrap()
            .join(vpnhide_protocol::zygisk_activation::MANAGED_MODULE_ID);
        for (name, bytes) in files {
            let path = recipient.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        let image = crate::maintenance::zygisk_projection::ProjectionImage::inspect_owned(
            &recipient,
            &descriptor,
            unsafe { libc::geteuid() },
        )
        .unwrap();
        let source_core = f.context.root.join("vpnhide-core");
        fs::copy(f.context.root.join("bin/arm64/vpnhide-core"), &source_core).unwrap();
        fs::set_permissions(&source_core, fs::Permissions::from_mode(0o755)).unwrap();
        let source = crate::maintenance::image::ImageFence::inspect(
            &f.context.root,
            None,
            Abi::Arm64,
            unsafe { libc::geteuid() },
        )
        .unwrap();
        let readback = crate::maintenance::zygisk_projection::work::ActiveProjectionReadback {
            source,
            image,
            source_directory: Arc::new(File::open(&f.context.root).unwrap()),
            recipient_directory: Arc::new(File::open(&recipient).unwrap()),
        };
        (recipient, readback)
    }
    #[test]
    fn a_bound_projection_pins_exact_source_and_recipient_without_promoting_packaging() {
        let f = fixture();
        let (recipient, image) = managed_image(&f);
        let registry = LoaderRegistry::default();
        *registry.context.lock().unwrap() = Some(f.context.clone());
        registry
            .bind_projection_owned(
                &f.context.root,
                &recipient,
                &image,
                unsafe { libc::geteuid() },
                &f.context.boot_id,
                f.context.epoch,
            )
            .unwrap();
        assert!(
            registry
                .bind_projection_owned(
                    &f.context.root,
                    &recipient,
                    &image,
                    unsafe { libc::geteuid() },
                    &f.context.boot_id,
                    f.context.epoch + 1
                )
                .is_err()
        );
        assert!(
            registry
                .bind_projection_owned(
                    &f.context.root,
                    &recipient,
                    &image,
                    unsafe { libc::geteuid() },
                    "other-boot",
                    f.context.epoch
                )
                .is_err()
        );
        let bound = registry.context.lock().unwrap().clone().unwrap();
        assert!(
            LoaderSources::open(
                &bound,
                &File::open(&recipient).unwrap(),
                Instant::now() + Duration::from_secs(1),
                RecipientAccess::Live
            )
            .is_ok()
        );
        assert!(
            LoaderSources::open(
                &bound,
                &File::open(&f.context.root).unwrap(),
                Instant::now() + Duration::from_secs(1),
                RecipientAccess::Live
            )
            .is_err()
        );
        assert!(!registry.available_at(&f.catalog, Abi::Arm64, false, &f.proc_root));
        let mut wrong = bound.clone();
        wrong.binding.as_mut().unwrap().source.1 += 1;
        assert!(
            pinned_generation(
                &wrong,
                &File::open(&f.context.root).unwrap(),
                Instant::now() + Duration::from_secs(1)
            )
            .is_err()
        );
        let mut incomplete = image.clone();
        incomplete.image.files.clear();
        assert!(
            registry
                .bind_projection_owned(
                    &f.context.root,
                    &recipient,
                    &incomplete,
                    unsafe { libc::geteuid() },
                    &f.context.boot_id,
                    f.context.epoch
                )
                .is_err()
        );
        fs::set_permissions(&recipient, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(
            LoaderSources::open(
                &bound,
                &File::open(&recipient).unwrap(),
                Instant::now() + Duration::from_secs(1),
                RecipientAccess::Live
            )
            .is_err()
        );
        fs::set_permissions(&recipient, fs::Permissions::from_mode(0o755)).unwrap();
        let old = recipient.with_file_name("old-recipient");
        fs::rename(&recipient, &old).unwrap();
        for name in [
            "module.prop",
            "vpnhide-projection.json",
            "zygisk/arm64-v8a.so",
            "zygisk/armeabi-v7a.so",
        ] {
            let path = recipient.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::copy(old.join(name), path).unwrap();
        }
        assert_ne!(
            directory_identity(&File::open(&recipient).unwrap()).unwrap(),
            image.image.directory
        );
        assert!(
            LoaderSources::open(
                &bound,
                &File::open(&recipient).unwrap(),
                Instant::now() + Duration::from_secs(1),
                RecipientAccess::Live
            )
            .is_err()
        );
    }
    #[test]
    fn in_flight_callback_cannot_acknowledge_a_previous_binding_generation() {
        let f = fixture();
        let (recipient, image) = managed_image(&f);
        let registry = LoaderRegistry::default();
        *registry.context.lock().unwrap() = Some(f.context.clone());
        let previous = registry.context.lock().unwrap().clone().unwrap();
        registry
            .bind_projection_owned(
                &f.context.root,
                &recipient,
                &image,
                unsafe { libc::geteuid() },
                &f.context.boot_id,
                f.context.epoch,
            )
            .unwrap();
        let bound = registry.context.lock().unwrap().clone().unwrap();
        assert!(
            registry
                .accept_current(&previous, f.observation.clone())
                .is_err()
        );
        assert!(
            registry
                .accept_current(&bound, f.observation.clone())
                .is_err()
        );
        assert!(registry.observations.lock().unwrap().is_empty());
        let mut observation = f.observation.clone();
        observation.directory_identity = image.image.directory;
        observation._recipient_directory = Arc::new(File::open(&recipient).unwrap());
        registry.accept_current(&bound, observation).unwrap();
        assert_eq!(registry.observations.lock().unwrap().len(), 1);
        assert!(!registry.available_at(&f.catalog, Abi::Arm64, false, &f.proc_root));
    }
}
