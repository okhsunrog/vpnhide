//! One Core-owned bounded run. No network/VPN prerequisite, app-supplied root
//! path, shell command, per-open restart, or inference from worker-owned PASS.
use super::boot_time::{BootDeadline, BootInstant};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use vpnhide_diagnostic_model::*;
use vpnhide_loader_ipc::image::{self, ProcessIdentity};

use super::framework_run::FrameworkRun;
use super::native_fixture::TunFixture;
use crate::{CoreState, auth::PeerCred};
use vpnhide_diagnostic_model::framework::{FrameworkWorkerRequest, FrameworkWorkerTask};

mod native_worker;

const WORKER: &str = vpnhide_loader_ipc::WORKER_PROCESS;
const DEADLINE: Duration = Duration::from_secs(20);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WorkerIdentity {
    process: ProcessIdentity,
    uid: u32,
    zygote: ProcessIdentity,
    network_namespace: (u64, u64),
}
struct Active {
    session: u64,
    deadline: BootDeadline,
    task: Option<DiagnosticWorkerTask>,
    worker: Option<WorkerIdentity>,
    report: Option<NativeMeasurement>,
    native: Option<super::native_exchange::NativeExchange>,
    finished: bool,
    minimum_birth_ticks: Option<u64>,
    framework: Option<Arc<Mutex<FrameworkRun>>>,
}
#[derive(Default)]
struct Inner {
    app_dir: Option<PathBuf>,
    next: u64,
    active: Option<Active>,
    last: Option<DiagnosticRun>,
    cancelled: bool,
    last_completed: Option<DiagnosticRun>,
    attempts: u8,
    retry_at: Option<BootDeadline>,
    pending_cleanup: Option<NativeFixtureSpec>,
    unidentified_cleanup: bool,
    pending_framework: Option<Arc<Mutex<FrameworkRun>>>,
}
type Spawn = fn(Box<dyn FnOnce() + Send>) -> std::io::Result<()>;
pub struct Coordinator {
    inner: Mutex<Inner>,
    changed: Condvar,
    monitoring: std::sync::atomic::AtomicBool,
    spawn: Spawn,
}

impl Default for Coordinator {
    fn default() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            changed: Condvar::new(),
            monitoring: std::sync::atomic::AtomicBool::new(false),
            spawn: |job| {
                std::thread::Builder::new()
                    .name("vpnhide-diagnostics".into())
                    .spawn(job)
                    .map(|_| ())
            },
        }
    }
}

fn boot_ticks() -> Result<u64, String> {
    let frequency = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    let frequency = u64::try_from(frequency).map_err(|_| "worker birth clock unavailable")?;
    BootInstant::now()
        .and_then(|now| now.ticks(frequency))
        .map_err(|error| error.to_string())
}

fn hash(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn platform_identity() -> Option<String> {
    // APEX updates change the mounted manifest even if Android build fingerprint
    // is unchanged. Missing/unreadable platform evidence never supports proof reuse.
    let mut material = image::bounded_text(Path::new("/system/build.prop"), 128 * 1024)
        .ok()?
        .into_bytes();
    let apex = fs::File::open("/apex/com.android.tethering/apex_manifest.pb").ok()?;
    use std::io::Read;
    let mut manifest = Vec::new();
    apex.take(64 * 1024 + 1).read_to_end(&mut manifest).ok()?;
    if manifest.is_empty() || manifest.len() > 64 * 1024 {
        return None;
    }
    material.extend_from_slice(&manifest);
    Some(hash(material))
}
fn context_digests(snapshot: &vpnhide_core_rpc::StateSnapshot) -> (String, String) {
    (
        hash(serde_json::to_vec(&snapshot.config).expect("config codec")),
        hash(
            serde_json::to_vec(&(
                &snapshot.backend,
                &snapshot.apply_evidence,
                &snapshot.zygisk_revocation,
                &snapshot.held_resources,
                snapshot.mutation_held,
                &snapshot.startup_error,
                &snapshot.storage_fault,
            ))
            .expect("runtime codec"),
        ),
    )
}
/// Runs inside the same publication mutation as a policy/runtime change.
/// Read-only consumers cannot see a later Save with earlier evidence still current.
pub(crate) fn invalidate_published(snapshot: &mut vpnhide_core_rpc::StateSnapshot, epoch: u64) {
    if snapshot.autonomous_diagnostics.is_none() {
        snapshot.autonomous_assessment = None;
        return;
    }
    let (policy, runtime) = context_digests(snapshot);
    if let Some(run) = snapshot.autonomous_diagnostics.as_mut() {
        run.stale |= run.fingerprint.core_epoch != epoch
            || run.fingerprint.policy_sha256 != policy
            || run.fingerprint.runtime_sha256 != runtime;
        snapshot.autonomous_assessment = Some(run.assessment());
    }
}

fn fingerprint(
    state: &CoreState,
    snapshot: &vpnhide_core_rpc::StateSnapshot,
) -> DiagnosticFingerprint {
    let (policy_sha256, runtime_sha256) = context_digests(snapshot);
    let boot_id = image::bounded_text(Path::new(crate::native_boot::BOOT_ID_PATH), 128)
        .unwrap_or_default()
        .trim()
        .to_owned();
    DiagnosticFingerprint {
        boot_id,
        core_epoch: state.publisher.epoch(),
        policy_sha256,
        runtime_sha256,
        platform_sha256: platform_identity(),
        // The registry is proof of generations actually consumed by live processes.
        // Absence of the loader is not an observed complete process inventory.
        loaded_generations_sha256: state.loader().generation_fingerprint(),
        loaded_generations_complete: false,
    }
}
fn plan(snapshot: &vpnhide_core_rpc::StateSnapshot) -> DiagnosticPlan {
    let config = snapshot.config.clone().unwrap_or_default();
    DiagnosticPlan::supported(
        config
            .settings
            .optional_features
            .contains("filesystem_iface_paths"),
        config.apps.values().any(|a| a.app_hiding),
        config.apps.values().any(|a| a.ports),
    )
}
fn namespace_identity(path: &Path) -> Result<(u64, u64), DiagnosticError> {
    let metadata = fs::metadata(path).map_err(|_| DiagnosticError::WrongProcess)?;
    Ok((metadata.dev(), metadata.ino()))
}
fn shared_network_namespace(root: &Path, pid: u32) -> Result<(u64, u64), DiagnosticError> {
    let ours = namespace_identity(&root.join("self/ns/net"))?;
    let worker = namespace_identity(&root.join(pid.to_string()).join("ns/net"))?;
    if ours != worker {
        return Err(DiagnosticError::WrongProcess);
    }
    Ok(ours)
}

fn worker_identity(
    root: &Path,
    dir: &Path,
    peer: PeerCred,
) -> Result<WorkerIdentity, DiagnosticError> {
    // Re-resolve owner on each call: reinstall and data replacement cannot reuse
    // an earlier admitted UID. SO_PEERCRED is retained by each connection.
    if peer.uid < 10_000 || fs::metadata(dir).map(|m| m.uid()).ok() != Some(peer.uid) {
        return Err(DiagnosticError::WrongProcess);
    }
    let child = root.join(peer.pid.to_string());
    if fs::metadata(&child).map(|m| m.uid()).ok() != Some(peer.uid)
        || !image::bounded_text(&child.join("cmdline"), 512)
            .is_ok_and(|v| v.split('\0').next() == Some(WORKER))
    {
        return Err(DiagnosticError::WrongProcess);
    }
    let process =
        image::process_identity(root, peer.pid).map_err(|_| DiagnosticError::WrongProcess)?;
    let zygote = crate::zygisk_loader::parent_zygote(root, process)
        .map_err(|_| DiagnosticError::WrongProcess)?;
    Ok(WorkerIdentity {
        process,
        uid: peer.uid,
        zygote,
        network_namespace: shared_network_namespace(root, peer.pid)?,
    })
}
struct NativeEvidence {
    filesystem: vpnhide_diagnostic_model::ScopedFilesystemResult,
    sys_interfaces: vpnhide_diagnostic_model::ScopedSysInterfaceResult,
    proc_interfaces: ScopedProcInterfaceResult,
    addresses: ScopedAddressResult,
    native: ScopedNativeResult,
    binding: ScopedBindResult,
}
struct MeasurementFailure {
    filesystem: Option<Box<vpnhide_diagnostic_model::ScopedFilesystemResult>>,
    sys_interfaces: Option<Box<vpnhide_diagnostic_model::ScopedSysInterfaceResult>>,
    proc_interfaces: Option<Box<ScopedProcInterfaceResult>>,
    addresses: Option<Box<ScopedAddressResult>>,
    binding: Option<Box<ScopedBindResult>>,
    detail: String,
    native: Option<ScopedNativeResult>,
    cleanup_confirmed: Option<bool>,
}

impl Coordinator {
    pub fn configure(&self, app_dir: PathBuf) {
        self.inner.lock().expect("diagnostic coordinator").app_dir = Some(app_dir);
    }
    pub(crate) fn configured(&self) -> bool {
        self.inner
            .lock()
            .expect("diagnostic coordinator")
            .app_dir
            .is_some()
    }
    pub fn cancel(&self) {
        self.inner.lock().expect("diagnostic coordinator").cancelled = true;
        self.changed.notify_all();
    }
    pub fn request(
        self: &Arc<Self>,
        state: Arc<CoreState>,
        force: bool,
    ) -> Result<DiagnosticRun, DiagnosticError> {
        self.reconcile_cleanup()?;
        let snapshot = state.snapshot();
        if snapshot.startup_error.is_some()
            || snapshot.storage_fault.is_some()
            || snapshot.mutation_held
            || !snapshot.operations_in_flight.is_empty()
            || snapshot.config.is_none()
            || snapshot.config.as_ref().is_some_and(|config| {
                vpnhide_migration_policy::with_self_target(
                    config.clone(),
                    vpnhide_migration_policy::APP_PACKAGE,
                ) != *config
            })
            || snapshot.backend.applied.is_none()
            || snapshot.backend.applied != snapshot.backend.selected
        {
            return Err(DiagnosticError::Busy);
        }
        #[cfg(target_os = "android")]
        if self
            .inner
            .lock()
            .expect("diagnostic coordinator")
            .active
            .is_none()
        {
            FrameworkRun::recover_orphan(state.publisher.epoch())?;
        }
        let fingerprint = fingerprint(&state, &snapshot);
        let plan = plan(&snapshot);
        let mut inner = self.inner.lock().expect("diagnostic coordinator");
        if inner.app_dir.is_none() || inner.cancelled {
            return Err(DiagnosticError::Disabled);
        }
        if let Some(last) = inner.last.as_ref() {
            if inner.active.is_some() {
                return if last.fingerprint == fingerprint {
                    Ok(last.clone())
                } else {
                    Err(DiagnosticError::Busy)
                };
            }
            if !force && last.fingerprint == fingerprint && last.plan == plan {
                // Retain a bounded partial/infrastructure receipt even without
                // complete generation evidence. Cache eligibility is distinct from
                // actual condition changes; neither turns a partial run into PASS.
                return Ok(last.clone());
            }
        }
        if inner
            .last
            .as_ref()
            .is_none_or(|last| last.fingerprint != fingerprint)
        {
            inner.attempts = 0;
        }
        let deadline = BootDeadline::from_now(DEADLINE).map_err(|_| DiagnosticError::Disabled)?;
        inner.attempts = inner.attempts.saturating_add(1);
        inner.retry_at = None;
        inner.next = inner.next.checked_add(1).ok_or(DiagnosticError::Expired)?;
        let run = DiagnosticRun {
            native_raw: None,
            filesystem: None,
            sys_interfaces: None,
            proc_interfaces: None,
            addresses: None,
            binding: None,
            session: inner.next,
            fingerprint,
            plan,
            outcome: DiagnosticOutcome::Running,
            framework: None,
            stale: false,
        };
        inner.active = Some(Active {
            session: run.session,
            deadline,
            task: None,
            worker: None,
            report: None,
            native: None,
            finished: false,
            minimum_birth_ticks: None,
            framework: None,
        });
        inner.last = Some(run.clone());
        state
            .publisher
            .update(|s| s.autonomous_diagnostics = Some(run.clone()));
        let coordinator = self.clone();
        let session = run.session;
        let worker_state = state.clone();
        (self.spawn)(Box::new(move || coordinator.execute(worker_state, session))).map_err(
            |_| {
                inner.active = None;
                inner.last.as_mut().expect("run reserved").outcome =
                    DiagnosticOutcome::Infrastructure {
                        detail: "diagnostic thread unavailable".into(),
                        native: None,
                        cleanup_confirmed: None,
                    };
                state
                    .publisher
                    .update(|s| s.autonomous_diagnostics = inner.last.clone());
                DiagnosticError::Disabled
            },
        )?;
        Ok(run)
    }
    fn execute(&self, state: Arc<CoreState>, session: u64) {
        // Keep the physical worker lease through BOTH fixture cleanup paths.
        let worker_lease = super::android_worker::lease();
        let result = match &worker_lease {
            Ok(_) => self.measure(&state, session),
            Err(detail) => Err(MeasurementFailure {
                filesystem: None,
                sys_interfaces: None,
                proc_interfaces: None,
                addresses: None,
                binding: None,
                detail: detail.clone(),
                native: None,
                cleanup_confirmed: Some(true),
            }),
        };
        let framework = self
            .inner
            .lock()
            .expect("diagnostic coordinator")
            .active
            .as_ref()
            .and_then(|a| a.framework.clone());
        let pending_framework = framework.clone();
        let framework = framework.map(|run| run.lock().expect("framework run").cleanup());
        if framework.as_ref().is_some_and(|(_, clean)| !clean) {
            self.inner
                .lock()
                .expect("diagnostic coordinator")
                .pending_framework = pending_framework;
        }
        let current = fingerprint(&state, &state.snapshot());
        let mut inner = self.inner.lock().expect("diagnostic coordinator");
        if inner.active.as_ref().is_none_or(|a| a.session != session) {
            return;
        }
        let raw_receipt = inner
            .active
            .as_ref()
            .and_then(|a| a.native.as_ref())
            .map(|n| n.receipt());
        inner.active = None;
        let retryable = matches!(&result, Err(failure) if failure.cleanup_confirmed == Some(true))
            || framework
                .as_ref()
                .is_some_and(|(result, clean)| *clean && !result.controls.is_empty());
        if retryable && inner.attempts < 3 {
            inner.retry_at =
                BootDeadline::from_now(Duration::from_secs(30 * u64::from(inner.attempts))).ok();
        }
        let Some(run) = inner.last.as_mut() else {
            return;
        };
        run.native_raw = raw_receipt;
        match result {
            Ok(evidence) => {
                run.record_filesystem(evidence.filesystem);
                run.record_sys_interfaces(evidence.sys_interfaces);
                run.record_proc_interfaces(evidence.proc_interfaces);
                run.record_addresses(evidence.addresses);
                run.record_native(evidence.native);
                run.record_binding(evidence.binding);
            }
            Err(failure) => {
                run.filesystem = failure.filesystem.map(|v| *v);
                run.sys_interfaces = failure.sys_interfaces.map(|v| *v);
                run.proc_interfaces = failure.proc_interfaces.map(|v| *v);
                run.addresses = failure.addresses.map(|v| *v);
                run.binding = failure.binding.map(|v| *v);
                run.outcome = DiagnosticOutcome::Infrastructure {
                    detail: failure.detail,
                    native: failure.native,
                    cleanup_confirmed: failure.cleanup_confirmed,
                }
            }
        }
        if let Some((framework, _)) = framework {
            run.record_framework(framework);
        }
        // Do not reattribute a run if policy, loaded images or platform changed.
        // Retain its evidence as stale rather than rewriting its fingerprint.
        if run.fingerprint != current {
            run.stale = true;
        }
        let run = run.clone();
        if matches!(run.outcome, DiagnosticOutcome::Incomplete { .. }) {
            inner.last_completed = Some(run.clone());
        }
        state
            .publisher
            .update(|s| s.autonomous_diagnostics = Some(run));
        self.changed.notify_all();
    }
    fn measure(
        &self,
        state: &CoreState,
        session: u64,
    ) -> Result<NativeEvidence, MeasurementFailure> {
        let fixture = TunFixture::create_detailed(session).map_err(|failure| {
            let cleaned = failure.cleanup.as_ref().map(|(_, cleaned)| *cleaned);
            let mut inner = self.inner.lock().expect("diagnostic coordinator");
            if failure.resource_created && cleaned != Some(true) {
                if let Some((spec, _)) = failure.cleanup {
                    inner.pending_cleanup = Some(spec);
                } else {
                    inner.unidentified_cleanup = true;
                }
            }
            MeasurementFailure {
                filesystem: None,
                sys_interfaces: None,
                proc_interfaces: None,
                addresses: None,
                binding: None,
                detail: format!("native fixture unavailable: {}", failure.error),
                native: None,
                cleanup_confirmed: cleaned,
            }
        })?;
        let report = (|| -> Result<NativeMeasurement, String> {
            let app_dir = self
                .inner
                .lock()
                .expect("diagnostic coordinator")
                .app_dir
                .clone()
                .ok_or("diagnostic adapter disabled")?;
            super::android_worker::prepare_fresh(&app_dir)?;
            let minimum_birth_ticks = boot_ticks()?
                .checked_add(1)
                .ok_or("worker birth clock overflow")?;
            // Proc starttime is quantized. Dispatch in the next tick so an older
            // root pre-specialization child cannot claim this fresh run.
            while boot_ticks()? < minimum_birth_ticks {
                std::thread::sleep(Duration::from_millis(2));
            }
            {
                let mut inner = self.inner.lock().expect("diagnostic coordinator");
                if inner.cancelled {
                    return Err("diagnostic run cancelled".into());
                }
                let active = inner
                    .active
                    .as_mut()
                    .filter(|a| a.session == session)
                    .ok_or("diagnostic session expired")?;
                if active
                    .deadline
                    .expired(BootInstant::now().map_err(|e| e.to_string())?)
                {
                    return Err(
                        "Android diagnostic worker deadline exceeded before dispatch".into(),
                    );
                }
                active.minimum_birth_ticks = Some(minimum_birth_ticks);
                active.task = Some(DiagnosticWorkerTask {
                    session,
                    core_epoch: state.publisher.epoch(),
                    fixture: fixture.spec.clone(),
                });
            }
            super::android_worker::launch()?;
            let mut inner = self.inner.lock().expect("diagnostic coordinator");
            loop {
                if inner.cancelled {
                    return Err("diagnostic run cancelled".into());
                }
                let active = inner
                    .active
                    .as_mut()
                    .filter(|a| a.session == session)
                    .ok_or("diagnostic session expired")?;
                if active.finished {
                    return active
                        .report
                        .clone()
                        .ok_or("native worker finished without observation".into());
                }
                let Some(remaining) = active
                    .deadline
                    .wait_slice(BootInstant::now().map_err(|e| e.to_string())?)
                else {
                    return Err("Android diagnostic worker deadline exceeded".into());
                };
                inner = self
                    .changed
                    .wait_timeout(inner, remaining)
                    .expect("diagnostic wait")
                    .0;
            }
        })();
        let unavailable = NativeMeasurement::unavailable(ProbeError::Timeout);
        // All paths close AND read back cleanup before releasing the run slot.
        // Synthetic unavailable samples cannot establish hiding success.
        let spec = fixture.spec.clone();
        let raw_receipt = self
            .inner
            .lock()
            .expect("diagnostic coordinator")
            .active
            .as_ref()
            .filter(|a| a.session == session)
            .and_then(|a| a.native.as_ref())
            .map(|n| n.receipt());
        let app_raw = raw_receipt.as_ref().and_then(|r| r.qualified_sample());
        let early_report = self
            .inner
            .lock()
            .expect("diagnostic coordinator")
            .active
            .as_ref()
            .filter(|a| a.session == session)
            .and_then(|a| a.report.clone());
        let protected = report
            .as_ref()
            .ok()
            .or(early_report.as_ref())
            .unwrap_or(&unavailable);
        let completion = fixture.finish_paired(protected, app_raw);
        let cleaned = cleanup_confirmed(&spec, &completion.cleanup);
        if !cleaned {
            self.inner
                .lock()
                .expect("diagnostic coordinator")
                .pending_cleanup = Some(spec.clone());
        }
        match report {
            Ok(_) if cleaned => Ok(NativeEvidence {
                filesystem: completion.filesystem,
                sys_interfaces: completion.sys_interfaces,
                proc_interfaces: completion.proc_interfaces,
                addresses: completion.addresses,
                native: completion.result,
                binding: completion.binding,
            }),
            Ok(_) => Err(MeasurementFailure {
                filesystem: Some(Box::new(completion.filesystem)),
                sys_interfaces: Some(Box::new(completion.sys_interfaces)),
                proc_interfaces: Some(Box::new(completion.proc_interfaces)),
                addresses: Some(Box::new(completion.addresses)),
                binding: Some(Box::new(completion.binding)),
                detail: "native fixture cleanup unconfirmed; diagnostic admission held".into(),
                native: Some(completion.result),
                cleanup_confirmed: Some(false),
            }),
            Err(detail) => Err(MeasurementFailure {
                filesystem: Some(Box::new(completion.filesystem)),
                sys_interfaces: Some(Box::new(completion.sys_interfaces)),
                proc_interfaces: Some(Box::new(completion.proc_interfaces)),
                addresses: Some(Box::new(completion.addresses)),
                binding: Some(Box::new(completion.binding)),
                detail,
                native: Some(completion.result),
                cleanup_confirmed: Some(cleaned),
            }),
        }
    }
    fn reconcile_cleanup(&self) -> Result<(), DiagnosticError> {
        let pending = self
            .inner
            .lock()
            .expect("diagnostic coordinator")
            .pending_framework
            .clone();
        if let Some(run) = pending {
            let (_, clean) = run.try_lock().map_err(|_| DiagnosticError::Busy)?.cleanup();
            if !clean {
                return Err(DiagnosticError::Busy);
            }
            let mut inner = self.inner.lock().expect("diagnostic coordinator");
            if inner
                .pending_framework
                .as_ref()
                .is_some_and(|r| Arc::ptr_eq(r, &run))
            {
                inner.pending_framework = None;
            }
        }
        let pending = {
            let inner = self.inner.lock().expect("diagnostic coordinator");
            if inner.unidentified_cleanup {
                return Err(DiagnosticError::Busy);
            }
            inner.pending_cleanup.clone()
        };
        if let Some(spec) = pending {
            if !cleanup_confirmed(
                &spec,
                &vpnhide_checks::fixture::observe(&spec.interface.name),
            ) {
                return Err(DiagnosticError::Busy);
            }
            let mut inner = self.inner.lock().expect("diagnostic coordinator");
            if inner.pending_cleanup.as_ref() == Some(&spec) {
                inner.pending_cleanup = None;
            }
        }
        Ok(())
    }
    /// Core polling observes effective-state changes; opening an APK screen never
    /// starts this supervisor. Infrastructure retries are bounded per fingerprint.
    pub fn monitor(self: &Arc<Self>, state: &Arc<CoreState>) {
        use std::sync::atomic::Ordering;
        if self.monitoring.swap(true, Ordering::SeqCst) {
            return;
        }
        let coordinator = self.clone();
        let weak = Arc::downgrade(state);
        let started = std::thread::Builder::new()
            .name("vpnhide-diagnostic-watch".into())
            .spawn(move || {
                loop {
                    let mut inner = coordinator.inner.lock().expect("diagnostic coordinator");
                    let until = Instant::now() + Duration::from_secs(5);
                    while !inner.cancelled && Instant::now() < until {
                        inner = coordinator
                            .changed
                            .wait_timeout(inner, until.saturating_duration_since(Instant::now()))
                            .expect("diagnostic monitor wait")
                            .0;
                    }
                    if inner.cancelled {
                        return;
                    }
                    let force = BootInstant::now()
                        .is_ok_and(|now| inner.retry_at.is_some_and(|t| t.expired(now)));
                    drop(inner);
                    let Some(state) = weak.upgrade() else {
                        return;
                    };
                    let _ = coordinator.request(state, force);
                }
            });
        if let Err(error) = started {
            self.monitoring.store(false, Ordering::SeqCst);
            crate::alog::info(&format!("diagnostic monitor could not start: {error}"));
        }
    }
    pub fn join(&self, peer: PeerCred) -> Result<Option<DiagnosticWorkerTask>, DiagnosticError> {
        let mut inner = self.inner.lock().expect("diagnostic coordinator");
        let app_dir = inner.app_dir.as_ref().ok_or(DiagnosticError::Disabled)?;
        let identity = worker_identity(Path::new("/proc"), app_dir, peer)?;
        if inner.cancelled {
            return Err(DiagnosticError::Expired);
        }
        let Some(active) = inner.active.as_mut() else {
            return Ok(None);
        };
        active.claim(
            identity,
            BootInstant::now().map_err(|_| DiagnosticError::Expired)?,
        )
    }

    pub fn framework_step(
        &self,
        peer: PeerCred,
        request: FrameworkWorkerRequest,
    ) -> Result<Option<FrameworkWorkerTask>, DiagnosticError> {
        let framework = {
            let mut inner = self.inner.lock().expect("diagnostic coordinator");
            let identity = worker_identity(
                Path::new("/proc"),
                inner.app_dir.as_ref().ok_or(DiagnosticError::Disabled)?,
                peer,
            )?;
            if inner.cancelled {
                return Err(DiagnosticError::Expired);
            }
            let active = inner.active.as_mut().ok_or(DiagnosticError::StaleSession)?;
            let task = active.task.as_ref().ok_or(DiagnosticError::StaleSession)?;
            if active
                .deadline
                .expired(BootInstant::now().map_err(|_| DiagnosticError::Expired)?)
            {
                return Err(DiagnosticError::Expired);
            }
            if active.worker != Some(identity) {
                return Err(DiagnosticError::WrongProcess);
            }
            if task.session != request.session
                || task.core_epoch != request.core_epoch
                || active.finished
                || active.report.is_none()
            {
                return Err(DiagnosticError::StaleSession);
            }
            if active.framework.is_none() {
                if request.phase
                    != vpnhide_diagnostic_model::framework::FrameworkWorkerPhase::Prepared
                {
                    return Err(DiagnosticError::StaleSession);
                }
                active.framework = Some(Arc::new(Mutex::new(FrameworkRun::new(
                    task,
                    (
                        identity.uid,
                        identity.process.pid,
                        identity.process.start_ticks,
                    ),
                    request.hook_sha256.clone(),
                    active.deadline,
                )?)));
            }
            active
                .framework
                .as_ref()
                .expect("framework reserved")
                .clone()
        };
        let mut framework = framework.try_lock().map_err(|_| DiagnosticError::Busy)?;
        framework.step(request)
    }

    pub fn report(
        &self,
        peer: PeerCred,
        report: DiagnosticWorkerReport,
    ) -> Result<(), DiagnosticError> {
        let mut inner = self.inner.lock().expect("diagnostic coordinator");
        let app_dir = inner.app_dir.as_ref().ok_or(DiagnosticError::Disabled)?;
        let identity = worker_identity(Path::new("/proc"), app_dir, peer)?;
        let epoch = inner
            .last
            .as_ref()
            .ok_or(DiagnosticError::StaleSession)?
            .fingerprint
            .core_epoch;
        let active = inner.active.as_mut().ok_or(DiagnosticError::StaleSession)?;
        let exchange = active
            .native
            .as_ref()
            .filter(|n| n.protected_ready())
            .ok_or(DiagnosticError::StaleSession)?;
        let thread =
            native_worker::thread_identity(Path::new("/proc"), identity, exchange.identity().tid)?;
        if thread != exchange.identity() {
            return Err(DiagnosticError::WrongProcess);
        }
        active.accept(
            identity,
            epoch,
            report,
            BootInstant::now().map_err(|_| DiagnosticError::Expired)?,
        )?;
        self.changed.notify_all();
        Ok(())
    }
}

impl Active {
    fn finish(
        &mut self,
        worker: WorkerIdentity,
        session: u64,
        epoch: u64,
        now: BootInstant,
        authenticate_thread: impl FnOnce(DiagnosticThreadIdentity) -> Result<(), DiagnosticError>,
    ) -> Result<(), DiagnosticError> {
        if self.deadline.expired(now) {
            return Err(DiagnosticError::Expired);
        }
        if self.worker != Some(worker) {
            return Err(DiagnosticError::WrongProcess);
        }
        let task = self.task.as_ref().ok_or(DiagnosticError::StaleSession)?;
        if task.session != session
            || task.core_epoch != epoch
            || self.finished
            || self.report.is_none()
        {
            return Err(DiagnosticError::StaleSession);
        }
        let native = self
            .native
            .as_ref()
            .filter(|n| n.protected_ready())
            .ok_or(DiagnosticError::StaleSession)?;
        authenticate_thread(native.identity())?;
        self.finished = true;
        Ok(())
    }
    fn claim(
        &mut self,
        identity: WorkerIdentity,
        now: BootInstant,
    ) -> Result<Option<DiagnosticWorkerTask>, DiagnosticError> {
        if self.deadline.expired(now) {
            return Err(DiagnosticError::Expired);
        }
        if self
            .minimum_birth_ticks
            .is_some_and(|ticks| identity.process.start_ticks < ticks)
        {
            return Err(DiagnosticError::WrongProcess);
        }
        if self.worker.is_some_and(|previous| previous != identity) {
            return Err(DiagnosticError::WrongProcess);
        }
        if self.task.is_some() {
            self.worker = Some(identity);
        }
        Ok(self.task.clone())
    }
    fn accept(
        &mut self,
        identity: WorkerIdentity,
        epoch: u64,
        report: DiagnosticWorkerReport,
        now: BootInstant,
    ) -> Result<(), DiagnosticError> {
        if self.deadline.expired(now) {
            return Err(DiagnosticError::Expired);
        }
        if self.session != report.session || epoch != report.core_epoch {
            return Err(DiagnosticError::StaleSession);
        }
        if self.worker != Some(identity) {
            return Err(DiagnosticError::WrongProcess);
        }
        if self.report.is_some() {
            return Err(DiagnosticError::StaleSession);
        }
        if !self.native.as_ref().is_some_and(|n| n.protected_ready()) {
            return Err(DiagnosticError::StaleSession);
        }
        if !report.native.bounded() {
            return Err(DiagnosticError::Malformed);
        }
        // Delivery is intentionally separate from worker completion. A later
        // Java failure/death must retain this already accepted measurement.
        self.report = Some(report.native);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publisher::Publisher;
    use vpnhide_core_rpc::{CanonicalConfig, NativeBackend, NativeBackendStatus, StateSnapshot};

    fn task() -> DiagnosticWorkerTask {
        DiagnosticWorkerTask {
            session: 7,
            core_epoch: 9,
            fixture: NativeFixtureSpec {
                session: 7,
                interface: InterfaceId {
                    name: "tun000000000007".into(),
                    index: 15,
                },
                flags: 4240,
                mtu: 1543,
                ordinary: OrdinaryControl {
                    interface: InterfaceId {
                        name: "lo".into(),
                        index: 1,
                    },
                    flags: 73,
                    mtu: 65536,
                },
            },
        }
    }
    fn identity(ticks: u64) -> WorkerIdentity {
        WorkerIdentity {
            network_namespace: (1, 2),
            process: ProcessIdentity {
                pid: 456,
                start_ticks: ticks,
            },
            uid: 10123,
            zygote: ProcessIdentity {
                pid: 2,
                start_ticks: 20,
            },
        }
    }
    fn report() -> DiagnosticWorkerReport {
        DiagnosticWorkerReport {
            session: 7,
            core_epoch: 9,
            native: vpnhide_checks::fixture::observe("tun000000000007"),
        }
    }
    #[test]
    fn claim_deadline_epoch_pid_reuse_and_report_replay_are_fenced() {
        let now = BootInstant::at(100);
        let mut active = Active {
            session: 7,
            deadline: now.after(DEADLINE).unwrap(),
            task: None,
            worker: None,
            report: None,
            native: None,
            finished: false,
            minimum_birth_ticks: Some(25),
            framework: None,
        };
        // No claim before fresh-process preparation has completed.
        assert_eq!(active.claim(identity(30), now), Ok(None));
        assert_eq!(
            active.accept(identity(30), 9, report(), now),
            Err(DiagnosticError::WrongProcess)
        );
        active.task = Some(task());
        assert_eq!(
            active.claim(identity(24), now),
            Err(DiagnosticError::WrongProcess)
        );
        assert_eq!(active.claim(identity(30), now), Ok(Some(task())));
        assert_eq!(
            active.claim(identity(31), now),
            Err(DiagnosticError::WrongProcess)
        );
        let mut old_epoch = report();
        old_epoch.core_epoch = 8;
        assert_eq!(
            active.accept(identity(30), 9, old_epoch, now),
            Err(DiagnosticError::StaleSession)
        );
        assert_eq!(
            active.accept(identity(31), 9, report(), now),
            Err(DiagnosticError::WrongProcess)
        );
        assert_eq!(
            active.accept(identity(30), 9, report(), BootInstant::at(120)),
            Err(DiagnosticError::Expired)
        );
        assert!(active.report.is_none());
        assert_eq!(
            active.accept(identity(30), 9, report(), now),
            Err(DiagnosticError::StaleSession)
        );
        active.native = Some(super::super::native_exchange::NativeExchange::unsupported(
            DiagnosticThreadIdentity {
                uid: 10123,
                pid: 456,
                process_start_ticks: 30,
                tid: 457,
                thread_start_ticks: 31,
            },
            NativeRawReceipt::UnsupportedBackend {
                identity: DiagnosticThreadIdentity {
                    uid: 10123,
                    pid: 456,
                    process_start_ticks: 30,
                    tid: 457,
                    thread_start_ticks: 31,
                },
            },
        ));
        assert_eq!(active.accept(identity(30), 9, report(), now), Ok(()));
        assert_eq!(
            active.accept(identity(30), 9, report(), now),
            Err(DiagnosticError::StaleSession)
        );
    }
    #[test]
    fn accepted_native_observation_survives_later_worker_death_and_cannot_finish_without_revalidation()
     {
        let now = BootInstant::at(100);
        let thread = DiagnosticThreadIdentity {
            uid: 10123,
            pid: 456,
            process_start_ticks: 30,
            tid: 457,
            thread_start_ticks: 31,
        };
        let mut active = Active {
            session: 7,
            deadline: now.after(DEADLINE).unwrap(),
            task: Some(task()),
            worker: Some(identity(30)),
            report: None,
            native: Some(super::super::native_exchange::NativeExchange::unsupported(
                thread,
                NativeRawReceipt::UnsupportedBackend { identity: thread },
            )),
            finished: false,
            minimum_birth_ticks: Some(25),
            framework: None,
        };
        assert_eq!(
            active.finish(identity(30), 7, 9, now, |_| Ok(())),
            Err(DiagnosticError::StaleSession)
        );
        let observation = report();
        let expected = observation.native.clone();
        active.accept(identity(30), 9, observation, now).unwrap();
        assert!(!active.finished);
        assert_eq!(active.report, Some(expected.clone()));
        // Java collection may still be running or the process may have died.
        assert_eq!(
            active.finish(identity(30), 7, 9, now, |_| Err(
                DiagnosticError::WrongProcess
            )),
            Err(DiagnosticError::WrongProcess)
        );
        assert!(!active.finished);
        assert_eq!(
            active.finish(identity(30), 7, 9, BootInstant::at(120), |_| Ok(())),
            Err(DiagnosticError::Expired)
        );
        assert_eq!(active.report, Some(expected.clone()));
        // The Android process can suspend without CLOCK_MONOTONIC advancing.
        // A later boot-time sample expires every admission while retaining the
        // observation accepted before suspension for cleanup/attribution.
        let resumed = BootInstant::at(300);
        assert_eq!(
            active.claim(identity(30), resumed),
            Err(DiagnosticError::Expired)
        );
        assert_eq!(
            active.accept(identity(30), 9, report(), resumed),
            Err(DiagnosticError::Expired)
        );
        assert_eq!(
            active.finish(identity(30), 7, 9, resumed, |_| panic!(
                "expired authority must not authenticate/finish"
            )),
            Err(DiagnosticError::Expired)
        );
        assert!(!active.finished);
        assert_eq!(active.report, Some(expected));
        active
            .finish(identity(30), 7, 9, now, |actual| {
                assert_eq!(actual, thread);
                Ok(())
            })
            .unwrap();
        assert!(active.finished);
        assert_eq!(
            active.finish(identity(30), 7, 9, now, |_| Ok(())),
            Err(DiagnosticError::StaleSession)
        );
    }
    fn state() -> Arc<CoreState> {
        CoreState::new(Publisher::new(
            9,
            StateSnapshot {
                config: Some(vpnhide_migration_policy::with_self_target(
                    CanonicalConfig::default(),
                    vpnhide_migration_policy::APP_PACKAGE,
                )),
                backend: NativeBackendStatus {
                    selected: Some(NativeBackend::Builtin),
                    applied: Some(NativeBackend::Builtin),
                    ..Default::default()
                },
                ..Default::default()
            },
        ))
    }
    #[test]
    fn unprepared_managed_target_cannot_admit_a_false_clean_run() {
        let state = state();
        state
            .publisher
            .update(|snapshot| snapshot.config = Some(CanonicalConfig::default()));
        let coordinator = Arc::new(Coordinator::default());
        coordinator.configure(PathBuf::from("/nonexistent-app"));
        assert_eq!(coordinator.request(state, true), Err(DiagnosticError::Busy));
        assert!(coordinator.inner.lock().unwrap().active.is_none());
    }
    #[test]
    fn incomplete_identity_retains_only_stale_evidence_and_reads_do_not_restart_work() {
        let state = state();
        let coordinator = Arc::new(Coordinator::default());
        coordinator.configure(PathBuf::from("/nonexistent-app"));
        let snapshot = state.snapshot();
        let last = DiagnosticRun {
            native_raw: None,
            filesystem: None,
            sys_interfaces: None,
            proc_interfaces: None,
            addresses: None,
            binding: None,
            session: 3,
            framework: None,
            fingerprint: fingerprint(&state, &snapshot),
            plan: plan(&snapshot),
            stale: false,
            outcome: DiagnosticOutcome::Infrastructure {
                detail: "unavailable".into(),
                native: None,
                cleanup_confirmed: None,
            },
        };
        assert!(!last.fingerprint.reusable());
        coordinator.inner.lock().unwrap().last = Some(last.clone());
        let retained = coordinator.request(state.clone(), false).unwrap();
        assert_eq!(retained.session, 3);
        assert!(!retained.stale);
        assert!(coordinator.inner.lock().unwrap().active.is_none());
        // A concurrent mutation/unknown effect prohibits even explicit force.
        state.publisher.update(|s| s.mutation_held = true);
        assert_eq!(coordinator.request(state, true), Err(DiagnosticError::Busy));
    }
    #[test]
    fn publication_invalidates_evidence_atomically_with_policy_and_delivery_changes() {
        let state = state();
        let snapshot = state.snapshot();
        let last = DiagnosticRun {
            native_raw: None,
            filesystem: None,
            sys_interfaces: None,
            proc_interfaces: None,
            addresses: None,
            binding: None,
            session: 3,
            framework: None,
            fingerprint: fingerprint(&state, &snapshot),
            plan: plan(&snapshot),
            stale: false,
            outcome: DiagnosticOutcome::Running,
        };
        state
            .publisher
            .update(|s| s.autonomous_diagnostics = Some(last));
        assert!(!state.snapshot().autonomous_diagnostics.unwrap().stale);
        state
            .publisher
            .update(|s| s.config.as_mut().unwrap().debug = true);
        assert!(state.snapshot().autonomous_diagnostics.unwrap().stale);
        state.publisher.update(|s| {
            let (policy, runtime) = context_digests(s);
            let run = s.autonomous_diagnostics.as_mut().unwrap();
            run.fingerprint.policy_sha256 = policy;
            run.fingerprint.runtime_sha256 = runtime;
            run.stale = false;
        });
        assert!(!state.snapshot().autonomous_diagnostics.unwrap().stale);
        state.publisher.update(|s| s.backend.applied = None);
        assert!(state.snapshot().autonomous_diagnostics.unwrap().stale);
    }
    #[test]
    fn assessment_is_core_owned_and_changes_atomically_with_its_source_run() {
        use vpnhide_diagnostic_model::ReuseBlocker;
        let state = state();
        let snapshot = state.snapshot();
        let run = DiagnosticRun {
            native_raw: None,
            filesystem: None,
            sys_interfaces: None,
            proc_interfaces: None,
            addresses: None,
            binding: None,
            session: 9,
            fingerprint: fingerprint(&state, &snapshot),
            plan: plan(&snapshot),
            outcome: DiagnosticOutcome::Running,
            framework: None,
            stale: false,
        };
        state
            .publisher
            .update(|s| s.autonomous_diagnostics = Some(run.clone()));
        let published = state.snapshot();
        assert_eq!(published.autonomous_assessment, Some(run.assessment()));
        assert!(
            !published
                .autonomous_assessment
                .unwrap()
                .reuse_blockers
                .contains(&ReuseBlocker::ConditionsChanged)
        );
        state
            .publisher
            .update(|s| s.config.as_mut().unwrap().debug = true);
        let published = state.snapshot();
        assert!(published.autonomous_diagnostics.as_ref().unwrap().stale);
        assert_eq!(
            published.autonomous_assessment,
            Some(
                published
                    .autonomous_diagnostics
                    .as_ref()
                    .unwrap()
                    .assessment()
            )
        );
        state.publisher.update(|s| s.autonomous_diagnostics = None);
        assert!(state.snapshot().autonomous_assessment.is_none());
    }

    #[test]
    fn live_process_consumption_churn_does_not_restart_the_heavy_suite() {
        let state = state();
        let snapshot = state.snapshot();
        let before = context_digests(&snapshot);
        state.publisher.update(|s| {
            s.zygisk_lifecycle = Some(vpnhide_core_rpc::ZygiskLifecycle::pending(
                vpnhide_core_rpc::ZygiskAuthorizationRef {
                    operation: 8,
                    boot_id: "boot".into(),
                    generation: 8,
                    policy_sha256: "a".repeat(64),
                    directory_device: 1,
                    directory_inode: 2,
                },
                7,
            ))
        });
        state.publisher.update(|s| {
            s.zygisk_lifecycle.as_mut().unwrap().processes.push(
                vpnhide_core_rpc::ZygiskProcessConsumption {
                    pid: 5,
                    start_ticks: 50,
                    uid: 10123,
                    process: "game".into(),
                    abi: "arm64".into(),
                    generation: Some(7),
                    policy_sha256: Some("b".repeat(64)),
                    consumption: vpnhide_core_rpc::ZygiskConsumption::Stale,
                },
            )
        });
        assert_eq!(context_digests(&state.snapshot()), before);
    }
    #[test]
    fn uncertain_cleanup_holds_force_and_changed_context_admission() {
        let state = state();
        let coordinator = Arc::new(Coordinator::default());
        coordinator.configure(PathBuf::from("/nonexistent-app"));
        coordinator.inner.lock().unwrap().unidentified_cleanup = true;
        assert_eq!(
            coordinator.request(state.clone(), true),
            Err(DiagnosticError::Busy)
        );
        state
            .publisher
            .update(|s| s.config.as_mut().unwrap().debug = true);
        assert_eq!(
            coordinator.request(state, false),
            Err(DiagnosticError::Busy)
        );
        assert!(coordinator.inner.lock().unwrap().active.is_none());
    }

    #[test]
    fn thread_creation_failure_settles_the_published_attempt() {
        let state = state();
        let coordinator = Arc::new(Coordinator {
            spawn: |_| Err(std::io::Error::other("injected thread exhaustion")),
            ..Default::default()
        });
        coordinator.configure(PathBuf::from("/nonexistent-app"));
        assert_eq!(
            coordinator.request(state.clone(), true),
            Err(DiagnosticError::Disabled)
        );
        let published = state
            .snapshot()
            .autonomous_diagnostics
            .expect("attempt published");
        assert!(matches!(
            published.outcome,
            DiagnosticOutcome::Infrastructure {
                native: None,
                cleanup_confirmed: None,
                ..
            }
        ));
        assert!(coordinator.inner.lock().unwrap().active.is_none());
    }
    #[test]
    fn root_or_ordinary_app_process_cannot_join_the_dedicated_worker_session() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(
            worker_identity(
                Path::new("/proc"),
                directory.path(),
                PeerCred {
                    uid: 0,
                    gid: 0,
                    pid: std::process::id()
                }
            ),
            Err(DiagnosticError::WrongProcess)
        );
        assert_eq!(
            worker_identity(
                Path::new("/proc"),
                directory.path(),
                PeerCred {
                    uid: 10123,
                    gid: 10123,
                    pid: std::process::id()
                }
            ),
            Err(DiagnosticError::WrongProcess)
        );
    }
    #[test]
    fn native_worker_must_share_fixture_network_namespace_and_unknown_is_not_equal() {
        let root = tempfile::tempdir().unwrap();
        for dir in ["self/ns", "123/ns"] {
            fs::create_dir_all(root.path().join(dir)).unwrap();
        }
        assert!(shared_network_namespace(root.path(), 123).is_err());
        fs::write(root.path().join("self/ns/net"), "namespace-a").unwrap();
        fs::write(root.path().join("123/ns/net"), "namespace-b").unwrap();
        assert!(shared_network_namespace(root.path(), 123).is_err());
        fs::remove_file(root.path().join("123/ns/net")).unwrap();
        fs::hard_link(
            root.path().join("self/ns/net"),
            root.path().join("123/ns/net"),
        )
        .unwrap();
        assert!(shared_network_namespace(root.path(), 123).is_ok());
    }
}
