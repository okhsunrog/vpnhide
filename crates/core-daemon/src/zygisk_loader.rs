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
    pub companion: ProcessIdentity,
    pub companion_anchor: u64,
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

fn parent_zygote(proc_root: &Path, child: ProcessIdentity) -> io::Result<ProcessIdentity> {
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
    if fs::metadata(&path)?.uid() != 0 {
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
        companion,
        companion_anchor: report.companion_anchor,
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
            records.remove(0);
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
        self.reconcile(Path::new("/proc"));
        if self.live_worker_at(&context, Path::new("/proc")) {
            return Ok(());
        }
        let start = Instant::now();
        let previous: Vec<_> = self
            .observations
            .lock()
            .expect("loader observations")
            .iter()
            .map(|o| o.process)
            .collect();
        let run = |arguments: &[&str]| -> Result<(), String> {
            let mut command = std::process::Command::new("/system/bin/cmd");
            command.args(arguments);
            let output =
                vpnhide_activator::output_with_timeout(&mut command, Duration::from_secs(2))
                    .map_err(|e| e.to_string())?;
            if !output.status.success() {
                return Err(format!(
                    "Android worker launch refused: {}",
                    String::from_utf8_lossy(&output.stderr)
                        .chars()
                        .take(256)
                        .collect::<String>()
                ));
            }
            Ok(())
        };
        run(&[
            "deviceidle",
            "tempwhitelist",
            "-u",
            "0",
            "-d",
            "5000",
            "dev.okhsunrog.vpnhide",
        ])?;
        run(&[
            "activity",
            "start-service",
            "--user",
            "0",
            "-n",
            "dev.okhsunrog.vpnhide/.diagnostics.LoaderProbeService",
        ])?;
        while start.elapsed() < Duration::from_secs(5) {
            self.reconcile(Path::new("/proc"));
            if self
                .observations
                .lock()
                .expect("loader observations")
                .iter()
                .any(|o| {
                    o.specialized
                        && o.process_name == WORKER_PROCESS
                        && o.boot_id == context.boot_id
                        && !previous.contains(&o.process)
                })
            {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        Err("fresh Android loader worker did not produce verified specialization evidence".into())
    }

    fn live_worker_at(&self, context: &LoaderContext, proc_root: &Path) -> bool {
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
            o.specialized
                && o.process_name == WORKER_PROCESS
                && o.boot_id == context.boot_id
                && o.epoch == context.epoch
                && o.directory_identity == (directory.dev(), directory.ino())
                && o.manifest_sha256 == catalog.sha256
                && image::process_identity(proc_root, o.process.pid).ok() == Some(o.process)
                && image::bounded_text(
                    &proc_root.join(o.process.pid.to_string()).join("cmdline"),
                    512,
                )
                .is_ok_and(|s| s.split('\0').next() == Some(WORKER_PROCESS))
                && image::process_identity(proc_root, o.companion.pid).ok() == Some(o.companion)
                && image::process_identity(proc_root, o.zygote.pid).ok() == Some(o.zygote)
                && image::mapping(proc_root, o.companion.pid, o.companion_anchor)
                    .is_ok_and(|m| m == o.mapping)
                && image::inspect_before(&o.image, Some(deadline)).is_ok_and(|identity| {
                    identity
                        .sha256
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>()
                        == o.payload_sha256
                })
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
        let script =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/test_payload_bundle.py");
        assert!(
            std::process::Command::new("python3")
                .arg(script)
                .arg("--fixture")
                .arg(root)
                .status()
                .unwrap()
                .success()
        );
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
            companion: ProcessIdentity {
                pid: 3,
                start_ticks: 30,
            },
            companion_anchor: address as usize as u64,
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
