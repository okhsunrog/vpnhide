//! One Core-owned bounded run. No network/VPN prerequisite, app-supplied root
//! path, shell command, per-open restart, or inference from worker-owned PASS.
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use vpnhide_diagnostic_model::*;
use vpnhide_loader_ipc::image::{self, ProcessIdentity};

use super::native_fixture::TunFixture;
use crate::{CoreState, auth::PeerCred};

const WORKER: &str = vpnhide_loader_ipc::WORKER_PROCESS;
const DEADLINE: Duration = Duration::from_secs(20);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WorkerIdentity {
    process: ProcessIdentity,
    uid: u32,
    zygote: ProcessIdentity,
}
struct Active {
    session: u64,
    deadline: Instant,
    task: Option<DiagnosticWorkerTask>,
    worker: Option<WorkerIdentity>,
    report: Option<NativeMeasurement>,
    minimum_birth_ticks: Option<u64>,
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
    retry_at: Option<Instant>,
    pending_cleanup: Option<NativeFixtureSpec>,
    unidentified_cleanup: bool,
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
    let mut time: libc::timespec = unsafe { std::mem::zeroed() };
    let frequency = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if frequency <= 0 || unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut time) } != 0 {
        return Err("worker birth clock unavailable".into());
    }
    let seconds = u64::try_from(time.tv_sec).map_err(|_| "invalid boot clock")?;
    let nanos = u64::try_from(time.tv_nsec).map_err(|_| "invalid boot clock")?;
    let frequency = frequency as u64;
    seconds
        .checked_mul(frequency)
        .and_then(|v| v.checked_add(nanos * frequency / 1_000_000_000))
        .ok_or("boot clock overflow".into())
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
pub(crate) fn invalidate_published(snapshot: &mut vpnhide_core_rpc::StateSnapshot) {
    if snapshot.autonomous_diagnostics.is_none() {
        return;
    }
    let (policy, runtime) = context_digests(snapshot);
    if let Some(run) = snapshot.autonomous_diagnostics.as_mut() {
        run.stale |=
            run.fingerprint.policy_sha256 != policy || run.fingerprint.runtime_sha256 != runtime;
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
    })
}
struct MeasurementFailure {
    detail: String,
    native: Option<ScopedNativeResult>,
    cleanup_confirmed: Option<bool>,
}

impl Coordinator {
    pub fn configure(&self, app_dir: PathBuf) {
        self.inner.lock().expect("diagnostic coordinator").app_dir = Some(app_dir);
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
            || snapshot.backend.applied.is_none()
            || snapshot.backend.applied != snapshot.backend.selected
        {
            return Err(DiagnosticError::Busy);
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
                // complete generation evidence. It is stale evidence, never reusable
                // current proof; a read does not restart heavy work indefinitely.
                let mut retained = last.clone();
                retained.stale |= !retained.fingerprint.reusable();
                return Ok(retained);
            }
        }
        if inner
            .last
            .as_ref()
            .is_none_or(|last| last.fingerprint != fingerprint)
        {
            inner.attempts = 0;
        }
        inner.attempts = inner.attempts.saturating_add(1);
        inner.retry_at = None;
        inner.next = inner.next.checked_add(1).ok_or(DiagnosticError::Expired)?;
        let run = DiagnosticRun {
            session: inner.next,
            fingerprint,
            plan,
            outcome: DiagnosticOutcome::Running,
            stale: false,
        };
        inner.active = Some(Active {
            session: run.session,
            deadline: Instant::now() + DEADLINE,
            task: None,
            worker: None,
            report: None,
            minimum_birth_ticks: None,
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
        let result = self.measure(&state, session);
        let current = fingerprint(&state, &state.snapshot());
        let mut inner = self.inner.lock().expect("diagnostic coordinator");
        if inner.active.as_ref().is_none_or(|a| a.session != session) {
            return;
        }
        inner.active = None;
        let retryable = matches!(&result, Err(failure) if failure.cleanup_confirmed == Some(true));
        if retryable && inner.attempts < 3 {
            inner.retry_at =
                Some(Instant::now() + Duration::from_secs(30 * u64::from(inner.attempts)));
        }
        let Some(run) = inner.last.as_mut() else {
            return;
        };
        match result {
            Ok(native) => run.record_native(native),
            Err(failure) => {
                run.outcome = DiagnosticOutcome::Infrastructure {
                    detail: failure.detail,
                    native: failure.native,
                    cleanup_confirmed: failure.cleanup_confirmed,
                }
            }
        }
        // Do not reattribute a run if policy, loaded images or platform changed.
        // Retain its evidence as stale rather than rewriting its fingerprint.
        if run.fingerprint != current || !run.fingerprint.reusable() {
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
    ) -> Result<ScopedNativeResult, MeasurementFailure> {
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
                if let Some(report) = active.report.take() {
                    return Ok(report);
                }
                let Some(remaining) = active.deadline.checked_duration_since(Instant::now()) else {
                    return Err("Android diagnostic worker deadline exceeded".into());
                };
                inner = self
                    .changed
                    .wait_timeout(inner, remaining)
                    .expect("diagnostic wait")
                    .0;
            }
        })();
        let unavailable = NativeMeasurement {
            flags: Sample::Error(ProbeError::Timeout),
            mtu: Sample::Error(ProbeError::Timeout),
            ifaddrs: Sample::Error(ProbeError::Timeout),
            links: Sample::Error(ProbeError::Timeout),
            ordinary_flags: Sample::Error(ProbeError::Timeout),
            ordinary_mtu: Sample::Error(ProbeError::Timeout),
        };
        // All paths close AND read back cleanup before releasing the run slot.
        // Synthetic unavailable samples cannot establish hiding success.
        let spec = fixture.spec.clone();
        let completion = fixture.finish(report.as_ref().unwrap_or(&unavailable));
        let cleaned = cleanup_confirmed(&spec, &completion.cleanup);
        if !cleaned {
            self.inner
                .lock()
                .expect("diagnostic coordinator")
                .pending_cleanup = Some(spec.clone());
        }
        match report {
            Ok(_) if cleaned => Ok(completion.result),
            Ok(_) => Err(MeasurementFailure {
                detail: "native fixture cleanup unconfirmed; diagnostic admission held".into(),
                native: Some(completion.result),
                cleanup_confirmed: Some(false),
            }),
            Err(detail) => Err(MeasurementFailure {
                detail,
                native: Some(completion.result),
                cleanup_confirmed: Some(cleaned),
            }),
        }
    }
    fn reconcile_cleanup(&self) -> Result<(), DiagnosticError> {
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
                    let force = inner.retry_at.is_some_and(|t| t <= Instant::now());
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
        active.claim(identity, Instant::now())
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
        active.accept(identity, epoch, report, Instant::now())?;
        self.changed.notify_all();
        Ok(())
    }
}

impl Active {
    fn claim(
        &mut self,
        identity: WorkerIdentity,
        now: Instant,
    ) -> Result<Option<DiagnosticWorkerTask>, DiagnosticError> {
        if self.deadline <= now {
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
        now: Instant,
    ) -> Result<(), DiagnosticError> {
        if self.deadline <= now {
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
        if !report.native.bounded() {
            return Err(DiagnosticError::Malformed);
        }
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
        let now = Instant::now();
        let mut active = Active {
            session: 7,
            deadline: now + DEADLINE,
            task: None,
            worker: None,
            report: None,
            minimum_birth_ticks: Some(25),
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
            active.accept(identity(30), 9, report(), now + DEADLINE),
            Err(DiagnosticError::Expired)
        );
        assert!(active.report.is_none());
        assert_eq!(active.accept(identity(30), 9, report(), now), Ok(()));
        assert_eq!(
            active.accept(identity(30), 9, report(), now),
            Err(DiagnosticError::StaleSession)
        );
    }
    fn state() -> Arc<CoreState> {
        CoreState::new(Publisher::new(
            9,
            StateSnapshot {
                config: Some(CanonicalConfig::default()),
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
    fn incomplete_identity_retains_only_stale_evidence_and_reads_do_not_restart_work() {
        let state = state();
        let coordinator = Arc::new(Coordinator::default());
        coordinator.configure(PathBuf::from("/nonexistent-app"));
        let snapshot = state.snapshot();
        let last = DiagnosticRun {
            session: 3,
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
        assert!(retained.stale);
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
            session: 3,
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
}
