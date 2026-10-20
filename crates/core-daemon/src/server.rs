//! The tarpc plumbing: the [`VpnhideCore`] service impl and the per-connection transport
//! (length-delimited framing + JSON, per core-rpc's codec decision).
//!
//! Kept apart from [`crate::listener`] so the accept-loop lifetime rules stay host-tested
//! while this side owns the tarpc/tokio-serde wiring.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use futures::prelude::*;
use tarpc::context::Context;
use tarpc::server::{BaseChannel, Channel};
use tokio::net::UnixStream;
use tokio_util::codec::length_delimited::LengthDelimitedCodec;
use tokio_util::sync::CancellationToken;
use vpnhide_core_rpc::{
    Accepted, BaseRef, CanonicalConfig, ClientHello, ConfigEdit, ConfigPhase, DiagnosticsReport,
    DiagnosticsRequest, EffectError, Handshake, LogcatRecordingError, LogcatSessionId,
    LogcatSessionStatus, OperationId, OperationRecord, PROTOCOL_VERSION, PhaseOutcome, PhaseRecord,
    Published, RootEffect, SubmitError, VpnhideCore,
};

use crate::config_edit::{self, apply_canonical_edit};
use crate::config_file;
use crate::journal::{self, Journal, JournalEntry};
use crate::operation::{self, PhaseHook, PlanKind, RealPhaseRunner};
use crate::publisher::Publisher;

/// This core build, reported in the handshake for diagnostics only.
pub const CORE_BUILD: &str = env!("CARGO_PKG_VERSION");

/// The authoritative config, its revision, and this instance's operation records.
///
/// This is core's in-memory canonical (§4 tier 1). `config_revision` bumps only on a real
/// config change and is what an edit conflict-checks against. When a journal path is set,
/// the revision and operation records are also durable (§4 tier 3 / §5).
struct ConfigStore {
    config: Option<CanonicalConfig>,
    config_revision: u64,
    operations: HashMap<OperationId, StoredOperation>,
    /// Mirror of the on-disk journal when durability is enabled; kept in lockstep with
    /// `operations` so each phase boundary can sync without rebuilding from the map.
    journal: Journal,
    /// Last revision that committed each field path (also durable via the journal).
    field_revisions: std::collections::BTreeMap<String, u64>,
}

struct StoredOperation {
    base: BaseRef,
    edit: ConfigEdit,
    record: OperationRecord,
}

/// The core's owned state: the published-state store plus the authoritative config.
pub struct CoreState {
    pub publisher: Publisher,
    config: Mutex<ConfigStore>,
    /// Where the durable canonical config lives. `None` = no persistence (host tests); a
    /// path means every accepted change is written there before it is acknowledged (§4).
    config_path: Option<PathBuf>,
    /// Private operation journal (§4 tier 3). `None` = memory-only (host tests that do not
    /// exercise restart fencing). Separate from the LSPosed-facing config file.
    journal_path: Option<PathBuf>,
    /// The kernel control node the `Native` phase delivers to (§4/§9.1). `None` disables
    /// native activation — the safe default so the dev sandbox never writes the real
    /// `/proc/vpnhide_ctl`.
    kmod_ctl_path: Option<PathBuf>,
    /// When true, config changes include a Ports phase (`VPNHIDE_PORTS`).
    ports_enabled: bool,
    /// Dispatch generation: bumped when admitting work and by [`Self::invalidate_dispatch`].
    /// A worker captures the value at admit and must still match before each external
    /// effect, so a delayed predecessor cannot dispatch after newer work (or an explicit
    /// fence) has started (§5).
    dispatch_fence: AtomicU64,
    /// Where root effects act (`VPNHIDE_ROOT_EFFECTS=1` → the real filesystem). `None`
    /// refuses them, so a dev sandbox never deletes or writes real state.
    effect_paths: std::sync::RwLock<Option<crate::effects::EffectPaths>>,
    /// Probe environment for `collect_diagnostics`.
    probe_env: std::sync::RwLock<crate::diagnostics::ProbeEnv>,
    /// This boot's fixed native decision (set once by [`Self::start_native_boot`]).
    native_boot: std::sync::RwLock<Option<operation::NativeBootHandle>>,
    /// The epoch this core run publishes under (recorded on a boot decision).
    epoch: AtomicU64,
    /// The single full-system logcat recording slot (v8).
    logcat: crate::logcat_session::Recorder,
}

/// Internal admission options: boot passes pick their own plan and Ports source.
#[derive(Debug, Clone, Copy)]
pub struct SubmitOptions {
    pub plan: Option<PlanKind>,
    pub ports_source: vpnhide_core_rpc::PortsSource,
    pub netd_ready: Option<bool>,
}

impl Default for SubmitOptions {
    fn default() -> Self {
        Self {
            plan: None,
            ports_source: vpnhide_core_rpc::PortsSource::Edit,
            netd_ready: None,
        }
    }
}

impl CoreState {
    /// A core with no durable file — the config lives only in memory. Used by host tests.
    pub fn new(publisher: Publisher) -> Arc<Self> {
        Arc::new(CoreState {
            publisher,
            config: Mutex::new(ConfigStore {
                config: None,
                config_revision: 0,
                operations: HashMap::new(),
                journal: Journal::default(),
                field_revisions: Default::default(),
            }),
            config_path: None,
            journal_path: None,
            kmod_ctl_path: None,
            ports_enabled: false,
            dispatch_fence: AtomicU64::new(0),
            effect_paths: std::sync::RwLock::new(crate::effects::effect_paths_from_env()),
            probe_env: std::sync::RwLock::new(crate::diagnostics::ProbeEnv::from_env()),
            native_boot: std::sync::RwLock::new(None),
            epoch: AtomicU64::new(0),
            logcat: crate::logcat_session::Recorder::new(
                crate::logcat_session::RecorderConfig::from_env(),
            ),
        })
    }

    /// Invalidate in-flight external dispatch. Delayed workers whose admit ticket no
    /// longer matches will refuse to run further effects (no blind late dispatch).
    pub fn invalidate_dispatch(&self) {
        self.dispatch_fence.fetch_add(1, Ordering::SeqCst);
    }

    /// Load the durable config from `path` and own it going forward (§4 tier-1, restart
    /// recovery). Without a journal the revision is reconstructed as 1 when a config is
    /// present (legacy behaviour). With a journal, the durable revision counter and
    /// operation records are restored and unresolved work is fenced (§5).
    ///
    /// `kmod_ctl_path` is the kernel control node the `Native` phase delivers to; `None`
    /// disables native activation (the dev sandbox default).
    pub fn load(
        publisher: Publisher,
        path: PathBuf,
        kmod_ctl_path: Option<PathBuf>,
    ) -> std::io::Result<Arc<Self>> {
        Self::load_with_journal(publisher, path, None, kmod_ctl_path)
    }

    /// Like [`load`], but also restores the private operation journal from `journal_path`.
    pub fn load_with_journal(
        publisher: Publisher,
        path: PathBuf,
        journal_path: Option<PathBuf>,
        kmod_ctl_path: Option<PathBuf>,
    ) -> std::io::Result<Arc<Self>> {
        let load = if crate::ownership::adopt_on_start_enabled() {
            crate::ownership::adopt_config_on_start(&path)?
        } else {
            config_file::read_status(&path)?
        };
        let file_config = match load {
            config_file::ConfigLoad::Absent => None,
            config_file::ConfigLoad::Present(cfg) => Some(cfg),
            // Refuse to invent defaults over a foreign schema — distinguishable from Absent.
            config_file::ConfigLoad::UnsupportedVersion { version } => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("unsupported config version {version}"),
                ));
            }
        };

        let mut journal = match &journal_path {
            Some(jp) => journal::read(jp)?,
            None => Journal::default(),
        };
        // Fence before exposing operations: Running → Unknown; unfinished ops Held.
        // Effects are never replayed.
        journal.fence_unresolved();

        // Evidence readback may resolve Unknown → Confirmed/FailedKnown without dispatch.
        let zygisk_module = crate::zygisk::module_from_env();
        let sources = crate::recovery::EvidenceSources {
            config_path: Some(path.as_path()),
            kmod_ctl_path: kmod_ctl_path.as_deref(),
            zygisk_module: zygisk_module.as_deref(),
        };
        let mut journal_dirty = false;
        for entry in &mut journal.operations {
            if crate::recovery::reconcile_entry(entry, &sources) {
                journal_dirty = true;
            }
        }
        // Advance durable revision when Persist evidence proves the write landed.
        let mut config_revision_from_journal = journal.config_revision;
        for entry in &journal.operations {
            let next =
                crate::recovery::revision_after_reconcile(entry, config_revision_from_journal);
            if next != config_revision_from_journal {
                config_revision_from_journal = next;
                journal_dirty = true;
            }
        }
        journal.config_revision = config_revision_from_journal;

        if let Some(jp) = &journal_path {
            // Rewrite after fence (and reconcile) so a second boot sees the resolved form.
            let _ = journal_dirty;
            journal::write_atomic(jp, &journal)?;
        }

        let (config, config_revision) = match file_config {
            Some(config) => {
                let rev = if journal_path.is_some() && journal.config_revision > 0 {
                    journal.config_revision
                } else {
                    1
                };
                (Some(config), rev)
            }
            None => (
                None,
                if journal_path.is_some() {
                    journal.config_revision
                } else {
                    0
                },
            ),
        };

        let mut operations = HashMap::new();
        for entry in &journal.operations {
            operations.insert(
                entry.record.op,
                StoredOperation {
                    base: entry.base,
                    edit: entry.edit.clone(),
                    record: entry.record.clone(),
                },
            );
        }

        let field_revisions = journal.field_revisions.clone();
        let state = Arc::new(CoreState {
            publisher,
            config: Mutex::new(ConfigStore {
                config: config.clone(),
                config_revision,
                operations,
                journal,
                field_revisions,
            }),
            config_path: Some(path),
            journal_path,
            kmod_ctl_path,
            ports_enabled: crate::ports::ports_enabled_from_env(),
            dispatch_fence: AtomicU64::new(0),
            effect_paths: std::sync::RwLock::new(crate::effects::effect_paths_from_env()),
            probe_env: std::sync::RwLock::new(crate::diagnostics::ProbeEnv::from_env()),
            native_boot: std::sync::RwLock::new(None),
            epoch: AtomicU64::new(0),
            logcat: crate::logcat_session::Recorder::new(
                crate::logcat_session::RecorderConfig::from_env(),
            ),
        });
        if let Some(config) = config {
            state.publisher.update(|snapshot| {
                snapshot.config = Some(config);
                snapshot.config_revision = config_revision;
            });
        }
        Ok(state)
    }

    /// True when a control node is configured (native management enabled).
    pub fn native_enabled(&self) -> bool {
        self.kmod_ctl_path.is_some()
    }

    /// True when the Ports writer is enabled.
    pub fn ports_enabled(&self) -> bool {
        self.ports_enabled
    }

    /// True once a durable config is loaded/created (for a startup log).
    pub fn has_config(&self) -> bool {
        self.config.lock().expect("config mutex").config.is_some()
    }

    /// Current config revision (boot repair base).
    pub fn config_revision(&self) -> u64 {
        self.config.lock().expect("config mutex").config_revision
    }

    /// True when an Unknown operation still Holds mutation admission.
    pub fn has_unknown_hold(&self) -> bool {
        let store = self.config.lock().expect("config mutex");
        store
            .operations
            .values()
            .any(|s| s.record.settled == Some(PhaseOutcome::Unknown))
            || store.journal.has_unknown()
    }

    /// Admit an edit from boot / internal callers (same path as RPC).
    pub fn submit_edit_public(
        &self,
        op: OperationId,
        base: BaseRef,
        edit: &ConfigEdit,
    ) -> Result<Accepted, SubmitError> {
        self.submit_edit(op, base, edit)
    }

    /// Admit an internal edit with an explicit plan / Ports source (boot passes).
    pub fn submit_edit_with(
        &self,
        op: OperationId,
        base: BaseRef,
        edit: &ConfigEdit,
        options: SubmitOptions,
    ) -> Result<Accepted, SubmitError> {
        self.submit_edit_opts(op, base, edit, options)
    }

    /// Make this boot's native decision (once per run, before serving). Reuses a
    /// decision persisted earlier in the same boot; otherwise scans, loads and retires.
    pub fn start_native_boot(&self, env: crate::native_boot::NativeEnv, epoch: u64) {
        self.epoch.store(epoch, Ordering::SeqCst);
        let filesystem_hiding = self.filesystem_hiding_wanted();
        let boot = crate::native_boot::NativeBoot::start(env, epoch, filesystem_hiding);
        self.install_native_boot(boot);
    }

    /// Install an already-made decision (tests, and [`Self::start_native_boot`]).
    pub fn install_native_boot(&self, boot: crate::native_boot::NativeBoot) {
        let handle = Arc::new(Mutex::new(boot));
        *self.native_boot.write().expect("native boot") = Some(handle);
        self.publish_native_boot(None);
    }

    /// The current per-boot native status, if native management is enabled.
    pub fn native_boot_status(&self) -> Option<vpnhide_core_rpc::NativeBootStatus> {
        let handle = self.native_boot.read().expect("native boot").clone()?;
        let status = handle.lock().expect("native boot").status.clone();
        Some(status)
    }

    /// Save-time capacity gate: project `next` for this boot's selected backend before any
    /// phase runs. A target set the backend cannot hold refuses the whole edit — nothing is
    /// persisted, applied or recorded, so the previous config stays in effect. Any other
    /// projection failure is left to the Native phase to report. Returns the projected wire
    /// so the Native phase can deliver it without resolving packages again.
    fn admit_native_capacity(
        &self,
        next: &CanonicalConfig,
    ) -> Result<Option<(vpnhide_core_rpc::NativeBackend, String)>, SubmitError> {
        if self.kmod_ctl_path.is_none() {
            return Ok(None);
        }
        let Some(handle) = self.native_handle() else {
            return Ok(None);
        };
        let Some(backend) = handle.lock().expect("native boot").selected() else {
            return Ok(None);
        };
        match crate::activation::project_native_wire(next, backend) {
            Ok(wire) => Ok(Some((backend, wire))),
            Err(crate::activation::ProjectionError::Capacity(report)) => {
                crate::alog::info(&format!(
                    "config: refused edit: {} native target uids exceed {backend:?} capacity {}",
                    report.total, report.capacity
                ));
                Err(SubmitError::NativeCapacityExceeded(report))
            }
            Err(crate::activation::ProjectionError::Failed(_)) => Ok(None),
        }
    }

    fn native_handle(&self) -> Option<operation::NativeBootHandle> {
        self.native_boot.read().expect("native boot").clone()
    }

    fn filesystem_hiding_wanted(&self) -> bool {
        self.config
            .lock()
            .expect("config mutex")
            .config
            .as_ref()
            .is_some_and(|c| {
                c.settings
                    .optional_features
                    .contains(vpnhide_activator::OPTIONAL_FEATURE_FILESYSTEM_IFACE_PATHS)
            })
    }

    /// Publish the native boot status into the replica. `applied` is the backend a
    /// just-confirmed Native phase configured, if any.
    fn publish_native_boot(&self, applied: Option<vpnhide_core_rpc::NativeBackend>) {
        let Some(handle) = self.native_handle() else {
            return;
        };
        let boot = handle.lock().expect("native boot");
        let status = boot.status.clone();
        let selected = boot.selected();
        drop(boot);
        self.publisher.update(|snapshot| {
            snapshot.backend.selected = selected;
            if applied.is_some() {
                snapshot.backend.applied = applied;
            }
            snapshot.backend.installed = status.installed.clone();
            // A backend core loaded this boot is live once its load confirmed.
            let loaded = match status.outcome {
                vpnhide_core_rpc::NativeBootOutcome::Selected {
                    backend,
                    mode: vpnhide_core_rpc::SelectionMode::Loaded,
                    ..
                } => Some(backend),
                _ => None,
            };
            snapshot.backend.active = selected.filter(|b| status.live.contains(b)).or(loaded);
            snapshot.backend.boot = Some(status);
        });
    }

    /// After a superkey change: re-decide a boot that was waiting for KernelPatch
    /// authentication, and deliver config when that produced a backend.
    fn retry_native_authentication(&self) {
        let Some(handle) = self.native_handle() else {
            return;
        };
        let filesystem_hiding = self.filesystem_hiding_wanted();
        let epoch = self.epoch.load(Ordering::SeqCst);
        let selected = {
            let mut boot = handle.lock().expect("native boot");
            if !boot.pending_authentication() {
                return;
            }
            crate::alog::info("native: superkey changed; retrying KernelPatch authentication");
            boot.retry_authentication(epoch, filesystem_hiding);
            boot.selected()
        };
        self.publish_native_boot(None);
        if selected.is_some() && self.has_config() && !self.has_unknown_hold() {
            let op = OperationId(
                (1 << 62)
                    | (std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos() as u64)
                        .unwrap_or(1)
                        & ((1 << 62) - 1)),
            );
            let base = BaseRef {
                revision: self.config_revision(),
            };
            let options = SubmitOptions {
                plan: Some(PlanKind::NativeOnly),
                ..SubmitOptions::default()
            };
            if let Err(e) = self.submit_edit_opts(op, base, &ConfigEdit::RepairActivation, options)
            {
                crate::alog::info(&format!("native: post-superkey repair refused: {e:?}"));
            }
        }
    }

    /// Point root effects at `paths` (tests, or a sandbox root). `None` disables them.
    pub fn set_effect_paths(&self, paths: Option<crate::effects::EffectPaths>) {
        *self.effect_paths.write().expect("effect paths") = paths;
    }

    /// Replace the probe environment (tests).
    pub fn set_probe_env(&self, env: crate::diagnostics::ProbeEnv) {
        *self.probe_env.write().expect("probe env") = env;
    }

    /// The logcat recorder (start/stop/status are blocking).
    pub fn logcat(&self) -> &crate::logcat_session::Recorder {
        &self.logcat
    }

    /// Start a core-owned logcat session (blocking).
    pub fn start_logcat_recording(
        &self,
        session: LogcatSessionId,
    ) -> Result<LogcatSessionStatus, LogcatRecordingError> {
        let env = self.probe_env.read().expect("probe env").clone();
        self.logcat.start(session, &env)
    }

    /// Stop a logcat session and hand its file to the app (blocking).
    pub fn stop_logcat_recording(
        &self,
        session: LogcatSessionId,
    ) -> Result<LogcatSessionStatus, LogcatRecordingError> {
        let env = self.probe_env.read().expect("probe env").clone();
        self.logcat.stop(session, &env)
    }

    /// Run a batch of fixed probes (blocking).
    pub fn collect_diagnostics(&self, request: &DiagnosticsRequest) -> DiagnosticsReport {
        let env = self.probe_env.read().expect("probe env").clone();
        crate::diagnostics::collect(&env, request)
    }

    /// Apply a fixed root effect under the mutation lane, so it never interleaves with a
    /// config operation. A full reset also drops the canonical config and publishes that.
    pub fn root_effect(&self, op: OperationId, effect: &RootEffect) -> Result<(), EffectError> {
        let Some(paths) = self.effect_paths.read().expect("effect paths").clone() else {
            return Err(EffectError::Failed("root effects disabled".into()));
        };
        let mut store = self.config.lock().map_err(|_| EffectError::Busy)?;
        eprintln!("core: root effect op={} {}", op.0, effect_name(effect));
        crate::effects::apply(&paths, effect)?;
        if matches!(effect, RootEffect::SetSuperkey { .. }) {
            drop(store);
            self.retry_native_authentication();
            return Ok(());
        }
        if matches!(effect, RootEffect::FullReset) {
            if let Some(path) = &self.config_path {
                match std::fs::remove_file(path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => {
                        return Err(EffectError::Failed(format!("remove config: {}", e.kind())));
                    }
                }
            }
            store.config = None;
            store.config_revision += 1;
            store.journal.config_revision = store.config_revision;
            if let Some(journal_path) = &self.journal_path {
                if let Err(e) = journal::write_atomic(journal_path, &store.journal) {
                    eprintln!("core: journal after reset failed: {e}");
                }
            }
            let revision = store.config_revision;
            self.publisher.update(|snapshot| {
                snapshot.config = None;
                snapshot.config_revision = revision;
            });
        }
        Ok(())
    }

    /// Query a settled/in-flight operation record.
    pub fn get_operation_record(&self, op: OperationId) -> Option<OperationRecord> {
        self.config
            .lock()
            .expect("config mutex")
            .operations
            .get(&op)
            .map(|s| s.record.clone())
    }

    /// Admit one typed edit (§5) and run its phase plan. Coarse optimistic concurrency: the
    /// caller's `base` must match the current config revision, or the edit is a conflict.
    ///
    /// Admission failures (conflict, no config to edit, a reused id) are `Err(SubmitError)`.
    /// An **admitted** edit returns `Ok(Accepted)` and its execution outcome — including a
    /// phase failure — lives in the queryable [`OperationRecord`] (settled), the way the app
    /// accepts an operation and then reports how it settled. `Persist` is the durable commit,
    /// so only a confirmed `Persist` publishes the new config into the replica.
    ///
    /// When a journal is configured, intent is synced before each dispatched effect and the
    /// settled record (plus revision) is synced afterward. Restart recovery fences unresolved
    /// work; it does not replay effects.
    fn submit_edit(
        &self,
        op: OperationId,
        base: BaseRef,
        edit: &ConfigEdit,
    ) -> Result<Accepted, SubmitError> {
        self.submit_edit_opts(op, base, edit, SubmitOptions::default())
    }

    fn submit_edit_opts(
        &self,
        op: OperationId,
        base: BaseRef,
        edit: &ConfigEdit,
        options: SubmitOptions,
    ) -> Result<Accepted, SubmitError> {
        let mut store = self.config.lock().expect("config mutex");
        // Deduplication precedes current-base and uncertainty gates: a lost
        // receipt may be queried using its original base, but never a new payload.
        if let Some(previous) = store.operations.get(&op) {
            return if previous.base == base && previous.edit == *edit {
                Ok(Accepted { op })
            } else {
                Err(SubmitError::IdReused)
            };
        }
        // Retention must not turn a pruned retry into a new mutation.
        if store.journal.is_retired(op) {
            return Err(SubmitError::IdReused);
        }
        // An uncertain dispatch is never repaired by blindly submitting a new edit.
        if store
            .operations
            .values()
            .any(|stored| stored.record.settled == Some(PhaseOutcome::Unknown))
            || store.journal.has_unknown()
        {
            return Err(SubmitError::Unavailable);
        }
        // Field-level conflict: unrelated edits merge against fresh state; same-field
        // (or Replace domain) overlap since base → Conflict (§5).
        if let Some(field) = config_edit::conflict_field(
            base.revision,
            store.config_revision,
            edit,
            &store.field_revisions,
        ) {
            return Err(SubmitError::Conflict { field });
        }
        // Apply against fresh config. With none yet, only a Replace can bootstrap one.
        // RepairActivation requires an existing config (nothing to re-activate otherwise).
        let current = match &store.config {
            Some(config) => config.clone(),
            None => match edit {
                ConfigEdit::Replace { .. } => CanonicalConfig::default(),
                ConfigEdit::RepairActivation => return Err(SubmitError::Unavailable),
                _ => return Err(SubmitError::Unavailable),
            },
        };
        let next = apply_canonical_edit(&current, edit);
        let changed = store.config.as_ref() != Some(&next);
        let repair = matches!(edit, ConfigEdit::RepairActivation);

        // Run the operation's phase plan. The runner performs the durable write (Persist) and,
        // when a control node is configured, delivers the native wire (Native). Repair skips
        // Persist and only activates. Blocking IO under the lock serializes writers; the RPC
        // layer detaches via spawn_blocking so connection loss cannot cancel ownership.
        let native = self.kmod_ctl_path.is_some();
        let ports = self.ports_enabled;
        let kind = if repair {
            match (native, ports) {
                (true, true) => PlanKind::NativeThenPorts,
                (true, false) => PlanKind::NativeOnly,
                (false, true) => PlanKind::PortsOnly,
                (false, false) => PlanKind::PersistOnly,
            }
        } else if changed {
            match (native, ports) {
                (true, true) => PlanKind::PersistThenNativeThenPorts,
                (true, false) => PlanKind::PersistThenNative,
                (false, true) => PlanKind::PersistThenPorts,
                (false, false) => PlanKind::PersistOnly,
            }
        } else {
            PlanKind::PersistOnly
        };
        let kind = options.plan.unwrap_or(kind);
        let plan = operation::plan_kind(kind);
        let native_wire = if changed && !repair && plan.contains(&ConfigPhase::Native) {
            self.admit_native_capacity(&next)?
        } else {
            None
        };

        // Claim a dispatch ticket so a delayed predecessor (or explicit invalidate)
        // cannot run external effects after this admission.
        let ticket = self.dispatch_fence.fetch_add(1, Ordering::SeqCst) + 1;

        let mut runner = RealPhaseRunner::new_with_ports(
            self.config_path.clone(),
            self.kmod_ctl_path.clone(),
            ports,
        );
        runner.native = self.native_handle();
        runner.native_wire = native_wire;
        runner.ports_source = options.ports_source;
        runner.ports_netd_ready = options.netd_ready;

        let outcome = if let Some(journal_path) = self.journal_path.clone() {
            let mut hook = JournalPhaseHook {
                path: journal_path,
                journal: &mut store.journal,
                op,
                base,
                edit,
                candidate: &next,
                plan: plan.clone(),
                completed: Vec::new(),
                ticket,
                fence: &self.dispatch_fence,
            };
            operation::run_plan_hooked(&plan, &next, changed, &runner, Some(&mut hook))
        } else {
            let mut hook = FenceOnlyHook {
                ticket,
                fence: &self.dispatch_fence,
            };
            operation::run_plan_hooked(&plan, &next, changed, &runner, Some(&mut hook))
        };

        if outcome.persisted {
            store.config = Some(next.clone());
            store.config_revision += 1;
            let rev = store.config_revision;
            let writes = config_edit::write_set(edit);
            config_edit::note_writes(&mut store.field_revisions, &writes, rev);
            store.journal.field_revisions = store.field_revisions.clone();
        }
        let ports_report = runner.take_ports_report();
        let record = OperationRecord {
            op,
            phases: outcome.phases.clone(),
            settled: Some(outcome.settled),
            ports: ports_report.clone(),
        };
        if let Some(journal_path) = &self.journal_path {
            let revision_for_journal = store.config_revision;
            if let Err(e) = journal::record_settled(
                journal_path,
                &mut store.journal,
                op,
                base,
                edit,
                &next,
                record.clone(),
                revision_for_journal,
            ) {
                eprintln!("core: journal settle failed: {e}");
                // Evidence of the effect may already exist; surface uncertainty rather than
                // pretending the operation vanished.
                let uncertain = OperationRecord {
                    op,
                    phases: outcome.phases.clone(),
                    settled: Some(PhaseOutcome::Unknown),
                    ports: ports_report.clone(),
                };
                store.operations.insert(
                    op,
                    StoredOperation {
                        base,
                        edit: edit.clone(),
                        record: uncertain,
                    },
                );
                return Err(SubmitError::Unavailable);
            }
        }
        store.operations.insert(
            op,
            StoredOperation {
                base,
                edit: edit.clone(),
                record,
            },
        );

        let revision = store.config_revision;
        // Publish while still owning the mutation lane so a later accepted edit
        // cannot publish first and then be overwritten by this older snapshot.
        if outcome.persisted {
            self.publisher.update(|snapshot| {
                snapshot.config = Some(next);
                snapshot.config_revision = revision;
            });
        }
        let native_ok = outcome
            .phases
            .iter()
            .any(|p| p.phase == ConfigPhase::Native && p.outcome == PhaseOutcome::Confirmed);
        let ran_native_or_ports = outcome.phases.iter().any(|p| {
            matches!(p.phase, ConfigPhase::Native | ConfigPhase::Ports)
                && p.outcome != PhaseOutcome::NotAttempted
        });
        if ran_native_or_ports {
            self.publish_native_boot(runner.selected().filter(|_| native_ok));
        }
        drop(store);
        Ok(Accepted { op })
    }

    fn get_operation(&self, op: OperationId) -> Option<OperationRecord> {
        self.config
            .lock()
            .expect("config mutex")
            .operations
            .get(&op)
            .map(|stored| stored.record.clone())
    }
}

/// Dispatch fence only (no journal) — used by host tests without a journal path.
struct FenceOnlyHook<'a> {
    ticket: u64,
    fence: &'a AtomicU64,
}

impl PhaseHook for FenceOnlyHook<'_> {
    fn before(&mut self, phase: ConfigPhase) -> Result<(), PhaseOutcome> {
        crate::recovery::maybe_delay_for_test(phase_delay_point(phase));
        if self.fence.load(Ordering::SeqCst) != self.ticket {
            eprintln!("core: dispatch fenced before {phase:?} (delayed predecessor)");
            return Err(PhaseOutcome::FailedKnown);
        }
        crate::recovery::maybe_crash_for_test(phase_intent_point(phase));
        Ok(())
    }

    fn after(&mut self, phase: ConfigPhase, _outcome: PhaseOutcome) {
        if phase == ConfigPhase::Persist {
            crate::recovery::maybe_crash_for_test("persist");
        }
        if phase == ConfigPhase::Native {
            crate::recovery::maybe_crash_for_test("native");
        }
    }
}

fn phase_intent_point(phase: ConfigPhase) -> &'static str {
    match phase {
        ConfigPhase::Persist => "persist-intent",
        ConfigPhase::Native => "native-intent",
        ConfigPhase::Ports => "ports-intent",
        ConfigPhase::Secret => "secret-intent",
        ConfigPhase::Cleanup => "cleanup-intent",
    }
}

fn phase_delay_point(phase: ConfigPhase) -> &'static str {
    match phase {
        ConfigPhase::Persist => "persist",
        ConfigPhase::Native => "native",
        ConfigPhase::Ports => "ports",
        ConfigPhase::Secret => "secret",
        ConfigPhase::Cleanup => "cleanup",
    }
}

/// Syncs journal evidence at each phase boundary (durability-before-effect).
struct JournalPhaseHook<'a> {
    path: PathBuf,
    journal: &'a mut Journal,
    op: OperationId,
    base: BaseRef,
    edit: &'a ConfigEdit,
    candidate: &'a CanonicalConfig,
    plan: Vec<ConfigPhase>,
    completed: Vec<(ConfigPhase, PhaseOutcome)>,
    ticket: u64,
    fence: &'a AtomicU64,
}

impl PhaseHook for JournalPhaseHook<'_> {
    fn before(&mut self, phase: ConfigPhase) -> Result<(), PhaseOutcome> {
        journal::record_phase_running(
            &self.path,
            self.journal,
            self.op,
            self.base,
            self.edit,
            self.candidate,
            &self.plan,
            &self.completed,
            phase,
        )
        .map_err(|e| {
            eprintln!("core: journal intent failed before {phase:?}: {e}");
            PhaseOutcome::FailedKnown
        })?;
        crate::recovery::maybe_delay_for_test(phase_delay_point(phase));
        if self.fence.load(Ordering::SeqCst) != self.ticket {
            eprintln!("core: dispatch fenced before {phase:?} (delayed predecessor)");
            return Err(PhaseOutcome::FailedKnown);
        }
        crate::recovery::maybe_crash_for_test(phase_intent_point(phase));
        Ok(())
    }

    fn after(&mut self, phase: ConfigPhase, outcome: PhaseOutcome) {
        self.completed.push((phase, outcome));
        // Mid-plan progress is already covered by the next before() / final settle.
        // Sync after Persist so a crash before Native still sees Persist confirmed.
        if phase == ConfigPhase::Persist {
            let phases: Vec<PhaseRecord> = self
                .plan
                .iter()
                .map(|&p| {
                    let outcome = if let Some((_, o)) = self.completed.iter().find(|(c, _)| *c == p)
                    {
                        *o
                    } else {
                        PhaseOutcome::NotAttempted
                    };
                    PhaseRecord { phase: p, outcome }
                })
                .collect();
            let entry = JournalEntry {
                base: self.base,
                edit: self.edit.clone(),
                candidate: Some(self.candidate.clone()),
                record: OperationRecord {
                    op: self.op,
                    phases,
                    settled: None,
                    ports: None,
                },
            };
            self.journal.upsert(entry);
            if let Err(e) = journal::write_atomic(&self.path, self.journal) {
                eprintln!("core: journal after Persist failed: {e}");
            }
            crate::recovery::maybe_crash_for_test("persist");
        }
        if phase == ConfigPhase::Native {
            crate::recovery::maybe_crash_for_test("native");
        }
    }
}

/// One connection's view of the core. Cloned per request by tarpc (cheap: an `Arc`).
#[derive(Clone)]
pub struct CoreServer {
    state: Arc<CoreState>,
}

impl CoreServer {
    pub fn new(state: Arc<CoreState>) -> Self {
        CoreServer { state }
    }
}

impl VpnhideCore for CoreServer {
    async fn hello(self, _ctx: Context, client: ClientHello) -> Handshake {
        if client.protocol == PROTOCOL_VERSION {
            Handshake::Compatible {
                core_protocol: PROTOCOL_VERSION,
                core_build: CORE_BUILD.to_owned(),
                epoch: self.state.publisher.epoch(),
            }
        } else {
            Handshake::Incompatible {
                core_protocol: PROTOCOL_VERSION,
            }
        }
    }

    async fn state_since(self, _ctx: Context, epoch: u64, revision: u64) -> Published {
        self.state.publisher.state_since(epoch, revision).await
    }

    async fn submit_edit(
        self,
        _ctx: Context,
        op: OperationId,
        base: BaseRef,
        edit: ConfigEdit,
    ) -> Result<Accepted, SubmitError> {
        // Own the mutation independently of this RPC connection (§5): dropping the
        // request future (client cancel / socket loss) must not cancel the work.
        let state = self.state.clone();
        match tokio::task::spawn_blocking(move || state.submit_edit(op, base, &edit)).await {
            Ok(result) => result,
            Err(_) => Err(SubmitError::Unavailable),
        }
    }

    async fn get_operation(self, _ctx: Context, op: OperationId) -> Option<OperationRecord> {
        self.state.get_operation(op)
    }

    async fn collect_diagnostics(
        self,
        _ctx: Context,
        request: DiagnosticsRequest,
    ) -> DiagnosticsReport {
        let state = self.state.clone();
        tokio::task::spawn_blocking(move || state.collect_diagnostics(&request))
            .await
            .unwrap_or_default()
    }

    async fn root_effect(
        self,
        _ctx: Context,
        op: OperationId,
        effect: RootEffect,
    ) -> Result<(), EffectError> {
        // Like submit_edit: owned by core, not cancelled with the request.
        let state = self.state.clone();
        tokio::task::spawn_blocking(move || state.root_effect(op, &effect))
            .await
            .unwrap_or(Err(EffectError::Busy))
    }

    async fn start_logcat_recording(
        self,
        _ctx: Context,
        session: LogcatSessionId,
    ) -> Result<LogcatSessionStatus, LogcatRecordingError> {
        let state = self.state.clone();
        tokio::task::spawn_blocking(move || state.start_logcat_recording(session))
            .await
            .unwrap_or_else(|_| Err(LogcatRecordingError::Failed("core task failed".into())))
    }

    async fn stop_logcat_recording(
        self,
        _ctx: Context,
        session: LogcatSessionId,
    ) -> Result<LogcatSessionStatus, LogcatRecordingError> {
        let state = self.state.clone();
        tokio::task::spawn_blocking(move || state.stop_logcat_recording(session))
            .await
            .unwrap_or_else(|_| Err(LogcatRecordingError::Failed("core task failed".into())))
    }

    async fn logcat_recording_status(self, _ctx: Context) -> Option<LogcatSessionStatus> {
        let state = self.state.clone();
        tokio::task::spawn_blocking(move || state.logcat().status())
            .await
            .unwrap_or(None)
    }
}

fn effect_name(effect: &RootEffect) -> &'static str {
    match effect {
        RootEffect::SetSuperkey { .. } => "set_superkey",
        RootEffect::ClearSuperkey => "clear_superkey",
        RootEffect::DeleteLegacyConfig => "delete_legacy_config",
        RootEffect::FullReset => "full_reset",
    }
}

/// Serve one accepted connection until it closes or its generation is cancelled.
///
/// Framing + codec match core-rpc's contract (length-delimited + JSON, capped at
/// [`vpnhide_core_rpc::MAX_FRAME_BYTES`]). Ends when the peer disconnects or `cancel`
/// fires, so a connection never outlives the generation that accepted it.
pub async fn serve_connection(
    stream: UnixStream,
    state: Arc<CoreState>,
    cancel: CancellationToken,
) {
    let framed = LengthDelimitedCodec::builder()
        .max_frame_length(vpnhide_core_rpc::MAX_FRAME_BYTES)
        .new_framed(stream);
    let transport = tarpc::serde_transport::new(framed, tokio_serde::formats::Json::default());
    let server = CoreServer::new(state);
    let serve = BaseChannel::with_defaults(transport)
        .execute(server.serve())
        .for_each(|response| async move {
            tokio::spawn(response);
        });
    tokio::select! {
        _ = cancel.cancelled() => {}
        _ = serve => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::{Journal, JournalEntry};
    use vpnhide_core_rpc::{
        AppConfig, ConfigToggle, FamilyApplyOutcome, PhaseOutcome, PhaseRecord, StateSnapshot,
    };

    fn server() -> CoreServer {
        CoreServer::new(CoreState::new(Publisher::new(1, StateSnapshot::default())))
    }

    #[tokio::test]
    async fn hello_accepts_the_current_protocol_and_refuses_skew() {
        let hello = |protocol| ClientHello {
            protocol,
            build: "test".to_owned(),
        };
        let ok = server()
            .hello(tarpc::context::current(), hello(PROTOCOL_VERSION))
            .await;
        assert!(matches!(ok, Handshake::Compatible { epoch: 1, .. }));

        for protocol in [1, PROTOCOL_VERSION + 1] {
            let skewed = server()
                .hello(tarpc::context::current(), hello(protocol))
                .await;
            assert_eq!(
                skewed,
                Handshake::Incompatible {
                    core_protocol: PROTOCOL_VERSION
                }
            );
        }
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn submit_bootstraps_applies_conflicts_and_reports_unavailable() {
        // Submits read the Ports env; keep env-mutating tests out.
        let _env = crate::ports::test_env_lock();
        let server = server();
        let ctx = tarpc::context::current;
        let toggle = |f| ConfigEdit::Toggle {
            field: f,
            enabled: true,
        };

        // No config yet: a non-Replace edit is Unavailable.
        let unavailable = server
            .clone()
            .submit_edit(
                ctx(),
                OperationId(1),
                BaseRef { revision: 0 },
                toggle(ConfigToggle::Debug),
            )
            .await;
        assert!(matches!(unavailable, Err(SubmitError::Unavailable)));

        // Replace bootstraps the config against revision 0, and is a queryable settled op.
        let bootstrap = ConfigEdit::Replace {
            config: CanonicalConfig::default(),
        };
        assert_eq!(
            server
                .clone()
                .submit_edit(ctx(), OperationId(2), BaseRef { revision: 0 }, bootstrap)
                .await
                .expect("accepted")
                .op,
            OperationId(2),
        );
        let record = server
            .clone()
            .get_operation(ctx(), OperationId(2))
            .await
            .expect("record");
        assert_eq!(record.settled, Some(PhaseOutcome::Confirmed));

        // A toggle now applies against fresh config; the revision advanced to 1.
        assert!(
            server
                .clone()
                .submit_edit(
                    ctx(),
                    OperationId(3),
                    BaseRef { revision: 1 },
                    toggle(ConfigToggle::Debug)
                )
                .await
                .is_ok()
        );

        // A base from before the Replace conflicts on the broad settings domain.
        let stale = server
            .clone()
            .submit_edit(
                ctx(),
                OperationId(4),
                BaseRef { revision: 0 },
                toggle(ConfigToggle::AutoHideName),
            )
            .await;
        assert!(matches!(stale, Err(SubmitError::Conflict { .. })));
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn field_level_allows_unrelated_edits_and_rejects_same_field() {
        // Submits read the Ports env; keep env-mutating tests out.
        let _env = crate::ports::test_env_lock();
        let server = server();
        let ctx = tarpc::context::current;
        server
            .clone()
            .submit_edit(
                ctx(),
                OperationId(1),
                BaseRef { revision: 0 },
                ConfigEdit::Replace {
                    config: CanonicalConfig::default(),
                },
            )
            .await
            .unwrap();
        // Two clients both at revision 1: unrelated fields merge.
        server
            .clone()
            .submit_edit(
                ctx(),
                OperationId(2),
                BaseRef { revision: 1 },
                ConfigEdit::Toggle {
                    field: ConfigToggle::Debug,
                    enabled: true,
                },
            )
            .await
            .unwrap();
        assert!(
            server
                .clone()
                .submit_edit(
                    ctx(),
                    OperationId(3),
                    BaseRef { revision: 1 },
                    ConfigEdit::Toggle {
                        field: ConfigToggle::AutoHideName,
                        enabled: true,
                    },
                )
                .await
                .is_ok(),
            "unrelated field must merge against fresh state"
        );
        // Same field with a stale base conflicts.
        let conflict = server
            .clone()
            .submit_edit(
                ctx(),
                OperationId(4),
                BaseRef { revision: 1 },
                ConfigEdit::Toggle {
                    field: ConfigToggle::Debug,
                    enabled: false,
                },
            )
            .await;
        assert!(matches!(
            conflict,
            Err(SubmitError::Conflict { field }) if field == "debug"
        ));
        // Replace against a stale base conflicts broadly.
        let replace_conflict = server
            .submit_edit(
                ctx(),
                OperationId(5),
                BaseRef { revision: 1 },
                ConfigEdit::Replace {
                    config: CanonicalConfig::default(),
                },
            )
            .await;
        assert!(matches!(
            replace_conflict,
            Err(SubmitError::Conflict { .. })
        ));
    }

    #[tokio::test]
    async fn root_effects_are_refused_unless_enabled() {
        let result = server()
            .root_effect(
                tarpc::context::current(),
                OperationId(1),
                RootEffect::ClearSuperkey,
            )
            .await;
        assert!(matches!(result, Err(EffectError::Failed(_))));
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn full_reset_drops_the_config_and_publishes_it() {
        // Submits read the Ports env; keep env-mutating tests out.
        let _env = crate::ports::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("vpnhide_config.json");
        let state = CoreState::load(
            Publisher::new(1, StateSnapshot::default()),
            config_path.clone(),
            None,
        )
        .unwrap();
        let fs_root = dir.path().join("root");
        state.set_effect_paths(Some(crate::effects::EffectPaths {
            root: fs_root.clone(),
        }));
        let server = CoreServer::new(state.clone());
        let ctx = tarpc::context::current;
        server
            .clone()
            .submit_edit(
                ctx(),
                OperationId(1),
                BaseRef { revision: 0 },
                ConfigEdit::Replace {
                    config: CanonicalConfig::default(),
                },
            )
            .await
            .unwrap();
        assert!(config_path.exists());
        let before = state.config_revision();

        server
            .clone()
            .root_effect(
                ctx(),
                OperationId(2),
                RootEffect::SetSuperkey {
                    superkey: "k".into(),
                },
            )
            .await
            .unwrap();
        assert!(fs_root.join("data/adb/vpnhide/superkey").exists());

        server
            .clone()
            .root_effect(ctx(), OperationId(3), RootEffect::FullReset)
            .await
            .unwrap();
        assert!(!config_path.exists());
        assert!(!fs_root.join("data/adb/vpnhide/superkey").exists());
        let published = server.clone().state_since(ctx(), 0, 0).await;
        assert!(published.snapshot.config.is_none());
        assert_eq!(published.snapshot.config_revision, before + 1);
        // A Replace against the new revision bootstraps again.
        server
            .clone()
            .submit_edit(
                ctx(),
                OperationId(4),
                BaseRef {
                    revision: before + 1,
                },
                ConfigEdit::Replace {
                    config: CanonicalConfig::default(),
                },
            )
            .await
            .unwrap();
        assert!(config_path.exists());
    }

    #[tokio::test]
    async fn diagnostics_run_fixed_probes_over_rpc() {
        let state = CoreState::new(Publisher::new(1, StateSnapshot::default()));
        state.set_probe_env(crate::diagnostics::ProbeEnv {
            core_exe: PathBuf::from("/nonexistent"),
            app_dir: None,
        });
        let report = CoreServer::new(state)
            .collect_diagnostics(
                tarpc::context::current(),
                DiagnosticsRequest {
                    probes: vec![
                        vpnhide_core_rpc::DiagnosticProbe::HookCounters,
                        vpnhide_core_rpc::DiagnosticProbe::GroundTruthChecks,
                    ],
                    clear_dmesg: false,
                },
            )
            .await;
        assert_eq!(report.outputs.len(), 2);
        assert!(
            report.outputs.iter().all(|o| o.exit == -1),
            "every probe is core's own child; none falls back to a shell"
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn activation_repair_runs_native_without_config_rewrite() {
        // Submits read the Ports env; keep env-mutating tests out.
        let _env = crate::ports::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("cfg.json");
        let ctl = dir.path().join("ctl");
        std::fs::write(
            &ctl,
            "vpnhide 1 status\nbackend 0x4\nkver 0x0\nhooks 0x0\nerror 0x0\n",
        )
        .unwrap();
        let state = CoreState::load(
            Publisher::new(1, StateSnapshot::default()),
            config_path.clone(),
            Some(ctl.clone()),
        )
        .unwrap();
        let server = CoreServer::new(state);
        let ctx = tarpc::context::current;
        let cfg = CanonicalConfig {
            debug: true,
            ..Default::default()
        };
        server
            .clone()
            .submit_edit(
                ctx(),
                OperationId(1),
                BaseRef { revision: 0 },
                ConfigEdit::Replace {
                    config: cfg.clone(),
                },
            )
            .await
            .unwrap();
        let before = std::fs::read_to_string(&config_path).unwrap();
        let published = server.clone().state_since(ctx(), 0, 0).await;
        let rev = published.snapshot.config_revision;
        server
            .clone()
            .submit_edit(
                ctx(),
                OperationId(2),
                BaseRef { revision: rev },
                ConfigEdit::RepairActivation,
            )
            .await
            .unwrap();
        let record = server
            .clone()
            .get_operation(ctx(), OperationId(2))
            .await
            .unwrap();
        assert!(
            record.phases.iter().any(|p| p.phase == ConfigPhase::Native),
            "repair must plan Native"
        );
        assert!(
            !record
                .phases
                .iter()
                .any(|p| p.phase == ConfigPhase::Persist),
            "repair must not rewrite canonical config"
        );
        assert_eq!(std::fs::read_to_string(&config_path).unwrap(), before);
        let after = server.state_since(ctx(), 0, 0).await;
        assert_eq!(after.snapshot.config_revision, rev);
        assert_eq!(after.snapshot.config, Some(cfg));
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn ports_phase_reports_partial_families_on_record() {
        use std::os::unix::fs::PermissionsExt;
        let _guard = crate::ports::test_env_lock();

        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let write_stub = |name: &str, body: &str| {
            let p = bin.join(name);
            std::fs::write(&p, body).unwrap();
            let mut perms = std::fs::metadata(&p).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&p, perms).unwrap();
        };
        // IPv4 ok; IPv6 restore fails — honest partial.
        let tables = r#"#!/bin/sh
case "$*" in
  *"-C OUTPUT"*) exit 1 ;;
  *) exit 0 ;;
esac
"#;
        write_stub("iptables", tables);
        write_stub("ip6tables", tables);
        write_stub("iptables-restore", "#!/bin/sh\ncat >/dev/null; exit 0\n");
        write_stub("ip6tables-restore", "#!/bin/sh\ncat >/dev/null; exit 1\n");

        let marker = dir.path().join("ports_owned");
        let pm_dump = dir.path().join("pm.txt");
        std::fs::write(&pm_dump, "package:com.example.app uid:10123\n").unwrap();
        let old_path = std::env::var_os("PATH");
        let new_path = format!(
            "{}:{}",
            bin.display(),
            old_path
                .as_ref()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        );
        // SAFETY: PATH_LOCK held for this test.
        unsafe {
            std::env::set_var("PATH", &new_path);
            std::env::set_var("VPNHIDE_PORTS", "1");
            std::env::set_var("VPNHIDE_PORTS_OWNER_MARKER", &marker);
            std::env::set_var("VPNHIDE_PORTS_PM_DUMP", &pm_dump);
        }

        let config_path = dir.path().join("cfg.json");
        let state = CoreState::load(
            Publisher::new(1, StateSnapshot::default()),
            config_path,
            None, // no native — Ports-only plan
        )
        .unwrap();
        let server = CoreServer::new(state);
        let ctx = tarpc::context::current;
        let mut cfg = CanonicalConfig::default();
        cfg.apps.insert(
            "com.example.app".into(),
            AppConfig {
                ports: true,
                ..Default::default()
            },
        );
        server
            .clone()
            .submit_edit(
                ctx(),
                OperationId(1),
                BaseRef { revision: 0 },
                ConfigEdit::Replace { config: cfg },
            )
            .await
            .unwrap();
        let record = server
            .clone()
            .get_operation(ctx(), OperationId(1))
            .await
            .unwrap();
        assert!(
            record
                .phases
                .iter()
                .any(|p| p.phase == ConfigPhase::Ports && p.outcome == PhaseOutcome::FailedKnown),
            "Ports phase must run and settle FailedKnown on partial apply: {record:?}"
        );
        let ports = record.ports.expect("ports family report on record");
        assert_eq!(ports.ipv4, FamilyApplyOutcome::Confirmed);
        assert_eq!(ports.ipv6, FamilyApplyOutcome::FailedKnown);
        assert!(
            marker.is_file(),
            "core must claim ownership on partial apply"
        );

        unsafe {
            std::env::remove_var("VPNHIDE_PORTS");
            std::env::remove_var("VPNHIDE_PORTS_OWNER_MARKER");
            std::env::remove_var("VPNHIDE_PORTS_PM_DUMP");
            match old_path {
                Some(v) => std::env::set_var("PATH", v),
                None => std::env::remove_var("PATH"),
            }
        }
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn delayed_worker_is_fenced_before_native_dispatch() {
        // Submits read the Ports env; keep env-mutating tests out.
        let _env = crate::ports::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("cfg.json");
        let ctl = dir.path().join("ctl");
        let ctl_body = "vpnhide 1 status\nbackend 0x4\nkver 0x0\nhooks 0x0\nerror 0x0\n";
        std::fs::write(&ctl, ctl_body).unwrap();
        let state = CoreState::load(
            Publisher::new(1, StateSnapshot::default()),
            config_path,
            Some(ctl.clone()),
        )
        .unwrap();
        // Fence the in-flight Native after Persist intent while the worker delays.
        // SAFETY: single-threaded test setup before the worker starts.
        unsafe {
            std::env::set_var("VPNHIDE_TEST_DELAY_BEFORE", "native");
            std::env::set_var("VPNHIDE_TEST_DELAY_MS", "150");
        }
        let state2 = state.clone();
        let fencer = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(40));
            state2.invalidate_dispatch();
        });
        let server = CoreServer::new(state);
        let ctx = tarpc::context::current;
        let result = server
            .clone()
            .submit_edit(
                ctx(),
                OperationId(1),
                BaseRef { revision: 0 },
                ConfigEdit::Replace {
                    config: CanonicalConfig {
                        debug: true,
                        ..Default::default()
                    },
                },
            )
            .await;
        fencer.join().unwrap();
        unsafe {
            std::env::remove_var("VPNHIDE_TEST_DELAY_BEFORE");
            std::env::remove_var("VPNHIDE_TEST_DELAY_MS");
        }
        result.expect("accepted");
        let record = server.get_operation(ctx(), OperationId(1)).await.unwrap();
        assert_eq!(
            record
                .phases
                .iter()
                .find(|p| p.phase == ConfigPhase::Native)
                .map(|p| p.outcome),
            Some(PhaseOutcome::FailedKnown),
            "fenced Native must not dispatch"
        );
        assert_eq!(std::fs::read_to_string(&ctl).unwrap(), ctl_body);
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn accepted_edit_persists_and_is_recovered_on_reload() {
        // Submits read the Ports env; keep env-mutating tests out.
        let _env = crate::ports::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vpnhide_config.json");
        let ctx = tarpc::context::current;

        // First run: no file yet -> no config, revision 0. A Replace persists it.
        let state = CoreState::load(
            Publisher::new(1, StateSnapshot::default()),
            path.clone(),
            None,
        )
        .unwrap();
        assert!(!state.has_config());
        let server = CoreServer::new(state);
        let cfg = CanonicalConfig {
            debug: true,
            ..Default::default()
        };
        let replace = ConfigEdit::Replace {
            config: cfg.clone(),
        };
        server
            .clone()
            .submit_edit(ctx(), OperationId(1), BaseRef { revision: 0 }, replace)
            .await
            .expect("accepted");
        assert!(
            path.exists(),
            "an accepted edit must be on disk before it is acknowledged"
        );

        // Second run (a restart): a fresh core loads the same file and recovers the config
        // at revision 1, so a follow-up edit conflict-checks against the recovered revision.
        let restarted = CoreState::load(
            Publisher::new(2, StateSnapshot::default()),
            path.clone(),
            None,
        )
        .unwrap();
        assert!(restarted.has_config());
        let server2 = CoreServer::new(restarted);
        let published = server2.clone().state_since(ctx(), 0, 0).await;
        assert_eq!(published.snapshot.config, Some(cfg));
        assert_eq!(published.snapshot.config_revision, 1);
        // An edit against the recovered revision applies; against a stale one it conflicts.
        assert!(
            server2
                .clone()
                .submit_edit(
                    ctx(),
                    OperationId(2),
                    BaseRef { revision: 1 },
                    ConfigEdit::Toggle {
                        field: ConfigToggle::AutoHideName,
                        enabled: true
                    }
                )
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn journal_survives_restart_and_dedupes_same_request() {
        // Submits read the Ports env; keep env-mutating tests out.
        let _env = crate::ports::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("vpnhide_config.json");
        let journal_path = dir.path().join("operations.json");
        let ctx = tarpc::context::current;

        let state = CoreState::load_with_journal(
            Publisher::new(1, StateSnapshot::default()),
            config_path.clone(),
            Some(journal_path.clone()),
            None,
        )
        .unwrap();
        let server = CoreServer::new(state);
        let replace = ConfigEdit::Replace {
            config: CanonicalConfig {
                debug: true,
                ..Default::default()
            },
        };
        server
            .clone()
            .submit_edit(
                ctx(),
                OperationId(10),
                BaseRef { revision: 0 },
                replace.clone(),
            )
            .await
            .expect("accepted");
        assert!(journal_path.exists());

        // Restart: durable revision continues, operation identity is restored.
        let restarted = CoreState::load_with_journal(
            Publisher::new(2, StateSnapshot::default()),
            config_path,
            Some(journal_path),
            None,
        )
        .unwrap();
        let server2 = CoreServer::new(restarted);
        let published = server2.clone().state_since(ctx(), 0, 0).await;
        assert_eq!(published.snapshot.config_revision, 1);
        let record = server2
            .clone()
            .get_operation(ctx(), OperationId(10))
            .await
            .expect("restored op");
        assert_eq!(record.settled, Some(PhaseOutcome::Confirmed));

        // Same id + same request → existing operation (idempotent retry).
        assert_eq!(
            server2
                .clone()
                .submit_edit(ctx(), OperationId(10), BaseRef { revision: 0 }, replace)
                .await
                .expect("deduped")
                .op,
            OperationId(10),
        );
        // Same id + different content → rejected.
        let reused = server2
            .submit_edit(
                ctx(),
                OperationId(10),
                BaseRef { revision: 1 },
                ConfigEdit::Toggle {
                    field: ConfigToggle::Debug,
                    enabled: false,
                },
            )
            .await;
        assert!(matches!(reused, Err(SubmitError::IdReused)));
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn restart_at_native_boundary_preserves_uncertainty_without_replay() {
        // Submits read the Ports env; keep env-mutating tests out.
        let _env = crate::ports::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("vpnhide_config.json");
        let journal_path = dir.path().join("operations.json");

        // Simulate a crash after Persist confirmed, Native still Running.
        let mut journal = Journal {
            config_revision: 1,
            ..Journal::default()
        };
        let candidate = CanonicalConfig {
            debug: true,
            ..Default::default()
        };
        journal.upsert(JournalEntry {
            base: BaseRef { revision: 0 },
            edit: ConfigEdit::Replace {
                config: candidate.clone(),
            },
            candidate: Some(candidate.clone()),
            record: OperationRecord {
                op: OperationId(42),
                phases: vec![
                    PhaseRecord {
                        phase: ConfigPhase::Persist,
                        outcome: PhaseOutcome::Confirmed,
                    },
                    PhaseRecord {
                        phase: ConfigPhase::Native,
                        outcome: PhaseOutcome::Running,
                    },
                ],
                settled: None,
                ports: None,
            },
        });
        journal::write_atomic(&journal_path, &journal).unwrap();
        // Config file already reflects the Persist (crash after write, before Native done).
        config_file::write_atomic(
            &config_path,
            &CanonicalConfig {
                debug: true,
                ..Default::default()
            },
        )
        .unwrap();

        let restarted = CoreState::load_with_journal(
            Publisher::new(9, StateSnapshot::default()),
            config_path,
            Some(journal_path.clone()),
            // Even with a ctl path, recovery must not re-dispatch Native.
            Some(dir.path().join("missing_ctl")),
        )
        .unwrap();
        let server = CoreServer::new(restarted);
        let ctx = tarpc::context::current;
        let record = server
            .clone()
            .get_operation(ctx(), OperationId(42))
            .await
            .expect("fenced op");
        assert_eq!(record.settled, Some(PhaseOutcome::Unknown));
        assert_eq!(
            record.phases[1].outcome,
            PhaseOutcome::Unknown,
            "Running Native must become Unknown, not Confirmed"
        );
        // New edits are blocked while uncertainty remains.
        let blocked = server
            .submit_edit(
                ctx(),
                OperationId(43),
                BaseRef { revision: 1 },
                ConfigEdit::Toggle {
                    field: ConfigToggle::AutoHideName,
                    enabled: true,
                },
            )
            .await;
        assert!(matches!(blocked, Err(SubmitError::Unavailable)));
        // Journal on disk should no longer say Running after fencing write-back.
        let on_disk = journal::read(&journal_path).unwrap();
        assert_eq!(
            on_disk.get(OperationId(42)).unwrap().record.settled,
            Some(PhaseOutcome::Unknown)
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn journal_revision_continues_across_restarts() {
        // Submits read the Ports env; keep env-mutating tests out.
        let _env = crate::ports::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("vpnhide_config.json");
        let journal_path = dir.path().join("operations.json");
        let ctx = tarpc::context::current;

        let state = CoreState::load_with_journal(
            Publisher::new(1, StateSnapshot::default()),
            config_path.clone(),
            Some(journal_path.clone()),
            None,
        )
        .unwrap();
        let server = CoreServer::new(state);
        server
            .clone()
            .submit_edit(
                ctx(),
                OperationId(1),
                BaseRef { revision: 0 },
                ConfigEdit::Replace {
                    config: CanonicalConfig::default(),
                },
            )
            .await
            .unwrap();
        server
            .clone()
            .submit_edit(
                ctx(),
                OperationId(2),
                BaseRef { revision: 1 },
                ConfigEdit::Toggle {
                    field: ConfigToggle::Debug,
                    enabled: true,
                },
            )
            .await
            .unwrap();

        // Without a journal, load would reconstruct revision=1; with a journal it continues at 2.
        let restarted = CoreState::load_with_journal(
            Publisher::new(2, StateSnapshot::default()),
            config_path,
            Some(journal_path),
            None,
        )
        .unwrap();
        let published = CoreServer::new(restarted).state_since(ctx(), 0, 0).await;
        assert_eq!(published.snapshot.config_revision, 2);
    }
}
