//! Root companion observations are separate from authorization and hook health.
//! No writable heartbeat, installed module name or current pathname proves which
//! library image a process loaded. Unknown evidence never promotes availability.

use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
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
    root: PathBuf,
    boot_id: String,
    epoch: u64,
    catalog: Arc<Mutex<Option<CatalogGeneration>>>,
}

#[derive(Debug, Clone)]
struct CatalogGeneration {
    directory: (u64, u64),
    manifest: Manifest,
    sha256: String,
}

fn pinned_generation(
    context: &LoaderContext,
    directory: &File,
    deadline: Instant,
) -> io::Result<CatalogGeneration> {
    let metadata = directory.metadata()?;
    let identity = (metadata.dev(), metadata.ino());
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
            sha256: manifest_sha256(&catalog),
            manifest: catalog.manifest,
        }
    };
    let mut bytes = Vec::new();
    crate::payloads::open_beneath(directory, "payloads/manifest.json")?
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if Manifest::parse(&bytes)? != generation.manifest {
        return Err(invalid("loader manifest changed after verification"));
    }
    if Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "loader catalog deadline",
        ));
    }
    *cache = Some(generation.clone());
    Ok(generation)
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
    let root = &verification.context.root;
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
    let received_directory = directory.metadata()?;
    let active_directory = fs::metadata(root)?;
    if !received_directory.is_dir()
        || (received_directory.dev(), received_directory.ino())
            != (active_directory.dev(), active_directory.ino())
    {
        return Err(invalid(
            "Zygisk recipient directory differs from active Core",
        ));
    }
    let recipient = crate::zygisk::ZygiskTarget::open(root).map_err(io::Error::other)?;
    if recipient.identity().map_err(io::Error::other)?
        != (received_directory.dev(), received_directory.ino())
    {
        return Err(invalid("recipient changed before catalog inspection"));
    }
    let generation = pinned_generation(verification.context, directory, verification.deadline)?;
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
    if recipient.revalidate_identity().map_err(io::Error::other)?
        != (received_directory.dev(), received_directory.ino())
    {
        return Err(invalid("recipient changed during catalog inspection"));
    }
    Ok(LoaderObservation {
        process: child,
        zygote,
        zygote_owner: 0,
        companion,
        companion_anchor: report.companion_anchor,
        child_anchor: callback.anchor,
        child_mapping: child_map,
        image: Arc::new(image_file.try_clone()?),
        mapping: companion_map,
        target_uid: callback.target_uid,
        process_name: callback.process.clone(),
        abi: identity.abi,
        payload_sha256: digest,
        payload_bytes: identity.bytes,
        manifest_sha256: generation.sha256,
        boot_id: challenge.boot_id.clone(),
        epoch: challenge.epoch,
        directory_identity: (received_directory.dev(), received_directory.ino()),
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
        let Ok(directory) = File::open(&context.root) else {
            return vec![];
        };
        let Ok(metadata) = directory.metadata() else {
            return vec![];
        };
        if (metadata.dev(), metadata.ino()) != (expected.directory_device, expected.directory_inode)
        {
            return vec![];
        }
        let deadline = Instant::now() + Duration::from_millis(200);
        let Ok(catalog) = pinned_generation(&context, &directory, deadline) else {
            return vec![];
        };
        let records = self.observations.lock().expect("loader observations");
        let mut verified = std::collections::BTreeMap::new();
        let mut result = vec![];
        for o in records.iter().take(64) {
            if !o.specialized
                || o.boot_id != context.boot_id
                || o.epoch != epoch
                || o.directory_identity != (metadata.dev(), metadata.ino())
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
        let Ok(directory) = fs::metadata(&context.root) else {
            return false;
        };
        let manifest = manifest_sha256(catalog);
        let observations = self.observations.lock().expect("loader observations");
        let deadline = Instant::now() + Duration::from_millis(200);
        let mut inspected = std::collections::BTreeSet::new();
        let mut proven = |abi| {
            observations.iter().any(|o| {
                o.abi == abi
                    && o.specialized
                    && o.boot_id == context.boot_id
                    && o.epoch == context.epoch
                    && o.directory_identity == (directory.dev(), directory.ino())
                    && o.manifest_sha256 == manifest
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
        match abi {
            Abi::Arm64 => proven(ImageAbi::Arm64) && (!needs_arm32 || proven(ImageAbi::Arm32)),
            Abi::Arm32 => proven(ImageAbi::Arm32),
        }
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
        let target = crate::zygisk::ZygiskTarget::open(&context.root)?;
        let expected = target.read_authorized(&context.boot_id)?;
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
        let Ok(directory) = fs::metadata(&context.root) else {
            return false;
        };
        let Ok(pinned) = File::open(&context.root) else {
            return false;
        };
        let Ok(catalog) = pinned_generation(
            context,
            &pinned,
            Instant::now() + Duration::from_millis(200),
        ) else {
            return false;
        };
        let records = self.observations.lock().expect("loader observations");
        let deadline = Instant::now() + Duration::from_millis(200);
        records.iter().any(|o| {
            !excluded.contains(&o.process)
                && o.specialized
                && match expected {
                    Some(a) => {
                        image::mapping(proc_root, o.process.pid, o.child_anchor)
                            .is_ok_and(|m| m == o.child_mapping)
                            && o.generation == Some(a.generation)
                            && o.policy_sha256 == Some(Sha256::digest(a.wire.as_bytes()).into())
                    }
                    None => o.generation.is_none() && o.policy_sha256.is_none(),
                }
                && o.process_name == WORKER_PROCESS
                && o.boot_id == context.boot_id
                && o.epoch == context.epoch
                && o.directory_identity == (directory.dev(), directory.ino())
                && o.manifest_sha256 == catalog.sha256
                && live_identity(o, proc_root, expected.is_some())
                && image::inspect_before(&o.image, Some(deadline)).is_ok_and(|identity| {
                    identity
                        .sha256
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>()
                        == o.payload_sha256
                })
                && live_identity(o, proc_root, expected.is_some())
        })
    }
}

pub fn start(
    registry: Arc<LoaderRegistry>,
    root: PathBuf,
    boot_id: String,
    epoch: u64,
) -> io::Result<crate::ServerHandle> {
    let context = LoaderContext {
        root,
        boot_id,
        epoch,
        catalog: Arc::new(Mutex::new(None)),
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
            let context = context.clone();
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                if cancel.is_cancelled() {
                    return;
                }
                let Ok(stream) = stream.into_std() else {
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
    registry.accept(observation);
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
            root,
            boot_id: "boot".into(),
            epoch: 7,
            catalog: Arc::new(Mutex::new(None)),
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
}
