//! The tarpc plumbing: the [`VpnhideCore`] service impl and the per-connection transport
//! (length-delimited framing + JSON, per core-rpc's codec decision).
//!
//! Kept apart from [`crate::listener`] so the accept-loop lifetime rules stay host-tested
//! while this side owns the tarpc/tokio-serde wiring.

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
    Accepted, BaseRef, CanonicalConfig, ClientHello, ConfigEdit, ConfigPhase, CoreCommand,
    DebugLeaseId, DebugLeaseOwner, DiagnosticsReport, DiagnosticsRequest, Handshake,
    LogcatDebugLease, LogcatDebugRestore, LogcatRecordingError, LogcatSessionId,
    LogcatSessionStatus, OperationId, OperationRecord, PROTOCOL_VERSION, PhaseOutcome, PhaseRecord,
    Published, RootEffect, SubmitError, VpnhideCore,
};

use crate::config_edit::{self, apply_canonical_edit};
use crate::config_file;
use crate::journal::{self, Journal, JournalEntry};
use crate::operation::{self, PhaseHook, PlanKind, RealPhaseRunner};
use crate::publisher::Publisher;

#[cfg(test)]
#[path = "command_tests.rs"]
mod command_tests;

/// This core build, reported in the handshake for diagnostics only.
pub const CORE_BUILD: &str = env!("CARGO_PKG_VERSION");

/// The authoritative config, its revision, and this instance's operation records.
///
/// `config_revision` changes only with canonical state. The bounded command writer
/// owns modifications; short mutex reads expose config and journal progress while
/// effects run outside the lock.
#[derive(Clone, Default)]
pub(crate) struct ConfigStore {
    config: Option<CanonicalConfig>,
    config_revision: u64,
    /// Mirror of the on-disk journal when durability is enabled; kept in lockstep with
    /// `operations` so each phase boundary can sync without rebuilding from the map.
    journal: Journal,
    /// Last revision that committed each field path (also durable via the journal).
    field_revisions: std::collections::BTreeMap<String, u64>,
}

fn wall_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
fn lease_duration(owner: DebugLeaseOwner) -> std::time::Duration {
    std::time::Duration::from_millis(match owner {
        DebugLeaseOwner::Forensic => 5 * 60_000,
        DebugLeaseOwner::Logcat => 31 * 60_000,
    })
}
fn lease_deadline(owner: DebugLeaseOwner) -> u64 {
    wall_time_ms().saturating_add(lease_duration(owner).as_millis() as u64)
}

#[derive(Clone, Copy)]
struct LiveLease {
    deadline: std::time::Instant,
    active: bool,
}

struct CommandSpec {
    admitted: Option<crate::command_engine::AdmissionReply>,
    intent: journal::CommandIntent,
    effect: Option<(ConfigPhase, RootEffect)>,
    plan: Option<Vec<ConfigPhase>>,
    reset: bool,
    lease: bool,
}

impl Default for CommandSpec {
    fn default() -> Self {
        Self {
            admitted: None,
            intent: journal::CommandIntent::Edit,
            effect: None,
            plan: None,
            reset: false,
            lease: false,
        }
    }
}

/// The core's owned state: the published-state store plus the authoritative config.
pub struct CoreState {
    pub publisher: Publisher,
    engine: crate::command_engine::CommandEngine,
    shutting_down: std::sync::atomic::AtomicBool,
    shutdown_gate: Mutex<()>,
    logcat_gate: Mutex<()>,
    heavy_rpc: Arc<tokio::sync::Semaphore>,
    live_leases: Mutex<std::collections::HashMap<DebugLeaseId, LiveLease>>,
    /// Where the durable canonical config lives. `None` = no persistence (host tests); a
    /// path means every accepted change is written there before it is acknowledged.
    config_path: Option<PathBuf>,
    /// Private operation journal. `None` = memory-only (host tests that do not
    /// exercise restart fencing). Separate from the LSPosed-facing config file.
    journal_path: Option<PathBuf>,
    /// The kernel control node the `Native` phase delivers to. `None` disables
    /// native activation — the safe default so the dev sandbox never writes the real
    /// `/proc/vpnhide_ctl`.
    kmod_ctl_path: Option<PathBuf>,
    /// When true, config changes include a Ports phase (`VPNHIDE_PORTS`).
    ports_enabled: bool,
    /// Dispatch generation: bumped when admitting work and by [`Self::invalidate_dispatch`].
    /// A worker captures the value at admit and must still match before each external
    /// effect, so a delayed predecessor cannot dispatch after newer work (or an explicit
    /// fence) has started.
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
    // Drop singleton ownership last, after every stateful owner.
    instance_guard: std::sync::OnceLock<crate::instance::InstanceGuard>,
}

/// Internal admission options: boot passes pick their own plan and Ports source.
#[derive(Debug, Clone, Copy)]
pub struct SubmitOptions {
    pub plan: Option<PlanKind>,
    pub ports_source: vpnhide_core_rpc::PortsSource,
    pub netd_ready: Option<bool>,
    pub automatic_repair: bool,
}

impl Default for SubmitOptions {
    fn default() -> Self {
        Self {
            plan: None,
            ports_source: vpnhide_core_rpc::PortsSource::Edit,
            netd_ready: None,
            automatic_repair: false,
        }
    }
}

impl CoreState {
    /// Every detached operation holds this state, so singleton ownership cannot
    /// expire while an accepted writer or privileged probe is still running.
    pub fn own_instance_guard(&self, guard: crate::instance::InstanceGuard) -> std::io::Result<()> {
        self.instance_guard
            .set(guard)
            .map_err(|_| std::io::Error::other("instance guard already attached"))
    }

    /// Refuse new work, finish accepted FIFO commands and join the writer before
    /// the caller releases singleton ownership. Safe to call more than once.
    pub fn shutdown(&self) {
        let _shutdown = self.shutdown_gate.lock().expect("shutdown gate");
        {
            let _admission = self.engine.admission.lock().expect("writer admission");
            if self.shutting_down.swap(true, Ordering::SeqCst) {
                return;
            }
            self.engine.stopping.store(true, Ordering::SeqCst);
        }
        {
            let _logcat = self.logcat_gate.lock().expect("logcat gate");
            self.logcat.shutdown();
        }
        // Daemon-owned leases end here. Internal release remains admissible long
        // enough to restore baseline once; an uncertain restore stays durable.
        let _ = self.call_writer_owned(
            |state| {
                let mut store = state.engine.store.lock().expect("config mutex").clone();
                if store.journal.debug_leases.is_empty() && store.journal.debug_restore.is_none() {
                    return Ok(());
                }
                store.journal.debug_leases.clear();
                state.persist_store(&store)?;
                state.restore_debug_if_unleased(true)
            },
            true,
        );
        self.engine.close();
    }

    /// Start read-only after corrupt/unsupported storage; absence is not invented.
    pub fn degraded(publisher: Publisher, reason: String) -> Arc<Self> {
        publisher.update(|snapshot| {
            snapshot.startup_error = Some(reason);
            snapshot.mutation_held = true;
        });
        Self::new(publisher)
    }

    fn call_writer<T: Send + 'static>(
        &self,
        work: impl FnOnce(&CoreState) -> T + Send + 'static,
    ) -> Result<T, SubmitError> {
        self.call_writer_owned(work, false)
    }

    fn call_writer_owned<T: Send + 'static>(
        &self,
        work: impl FnOnce(&CoreState) -> T + Send + 'static,
        internal_release: bool,
    ) -> Result<T, SubmitError> {
        if self.shutting_down.load(Ordering::SeqCst) && !internal_release {
            return Err(SubmitError::Unavailable);
        }
        self.engine.call(work, internal_release)
    }

    pub fn issue_operation_id(&self) -> Result<OperationId, SubmitError> {
        self.call_writer(|state| state.issue_operation_id_direct())?
    }

    fn issue_operation_id_direct(&self) -> Result<OperationId, SubmitError> {
        if self.snapshot().startup_error.is_some() || self.snapshot().storage_fault.is_some() {
            return Err(SubmitError::Unavailable);
        }
        let mut store = self.engine.store.lock().expect("config mutex").clone();
        store.journal.issued_through = store
            .journal
            .issued_through
            .checked_add(1)
            .ok_or(SubmitError::Unavailable)?;
        if let Some(path) = &self.journal_path {
            if let Err(error) = journal::write_atomic(path, &store.journal) {
                let previous = self.engine.store.lock().expect("config mutex").clone();
                self.freeze_storage_fault(&previous, OperationId(0), &error);
                return Err(SubmitError::Unavailable);
            }
        }
        let id = OperationId(store.journal.issued_through);
        *self.engine.store.lock().expect("config mutex") = store;
        Ok(id)
    }

    pub fn submit_command(
        &self,
        op: OperationId,
        command: CoreCommand,
    ) -> Result<Accepted, SubmitError> {
        self.call_writer(move |state| state.execute_command(op, command))?
    }

    fn execute_command(
        &self,
        op: OperationId,
        command: CoreCommand,
    ) -> Result<Accepted, SubmitError> {
        self.execute_command_admitted(op, command, None)
    }

    pub(crate) fn execute_command_admitted(
        &self,
        op: OperationId,
        command: CoreCommand,
        admitted: Option<crate::command_engine::AdmissionReply>,
    ) -> Result<Accepted, SubmitError> {
        if self.snapshot().startup_error.is_some() || self.snapshot().storage_fault.is_some() {
            return Err(SubmitError::Unavailable);
        }
        if let Some(entry) = self
            .engine
            .store
            .lock()
            .expect("config mutex")
            .journal
            .get(op)
        {
            if matches!(
                (&command, &entry.command),
                (
                    CoreCommand::FullReset,
                    Some(journal::CommandIntent::FullReset)
                ) | (
                    CoreCommand::CleanupLegacy,
                    Some(journal::CommandIntent::CleanupLegacy)
                )
            ) {
                return Ok(Accepted { op });
            }
        }
        let original_base = self
            .engine
            .store
            .lock()
            .expect("config mutex")
            .journal
            .get(op)
            .map(|entry| entry.base);
        let current_base = original_base.unwrap_or_else(|| BaseRef {
            revision: self.config_revision(),
        });
        let remember_edit = |enabled| ConfigEdit::Toggle {
            field: vpnhide_core_rpc::ConfigToggle::RememberSuperkey,
            enabled,
        };
        let (base, edit, mut spec) = match command {
            CoreCommand::Edit { base, edit } => (base, edit, CommandSpec::default()),
            CoreCommand::SetSuperkey {
                base,
                remember,
                superkey,
            } => {
                let key = superkey.trim();
                if key.is_empty()
                    || key.len() > crate::effects::MAX_SUPERKEY_BYTES
                    || key.contains(['\n', '\0'])
                {
                    return Err(SubmitError::Rejected {
                        reason: "invalid superkey".into(),
                    });
                }
                (
                    base,
                    remember_edit(remember),
                    CommandSpec {
                        intent: journal::CommandIntent::SetSuperkey { remember },
                        effect: Some((ConfigPhase::Secret, RootEffect::SetSuperkey { superkey })),
                        ..Default::default()
                    },
                )
            }
            CoreCommand::ClearSuperkey { base } => (
                base,
                remember_edit(false),
                CommandSpec {
                    intent: journal::CommandIntent::ClearSuperkey,
                    effect: Some((ConfigPhase::Secret, RootEffect::ClearSuperkey)),
                    ..Default::default()
                },
            ),
            CoreCommand::Import {
                base,
                config,
                cleanup_legacy,
            } => (
                base,
                ConfigEdit::Replace { config },
                CommandSpec {
                    intent: journal::CommandIntent::Import { cleanup_legacy },
                    effect: cleanup_legacy
                        .then_some((ConfigPhase::Cleanup, RootEffect::DeleteLegacyConfig)),
                    ..Default::default()
                },
            ),
            CoreCommand::CleanupLegacy => (
                current_base,
                ConfigEdit::RepairActivation,
                CommandSpec {
                    intent: journal::CommandIntent::CleanupLegacy,
                    effect: Some((ConfigPhase::Cleanup, RootEffect::DeleteLegacyConfig)),
                    plan: Some(vec![ConfigPhase::Cleanup]),
                    ..Default::default()
                },
            ),
            CoreCommand::FullReset => {
                if self.logcat.is_active() || self.debug_lease_held() {
                    return Err(SubmitError::Busy);
                }
                (
                    current_base,
                    ConfigEdit::Replace {
                        config: CanonicalConfig::default(),
                    },
                    CommandSpec {
                        intent: journal::CommandIntent::FullReset,
                        effect: Some((ConfigPhase::Cleanup, RootEffect::FullReset)),
                        plan: Some(
                            std::iter::once(ConfigPhase::Persist)
                                .chain(self.native_enabled().then_some(ConfigPhase::Native))
                                .chain(self.ports_enabled().then_some(ConfigPhase::Ports))
                                .chain(std::iter::once(ConfigPhase::Cleanup))
                                .collect(),
                        ),
                        reset: true,
                        lease: false,
                        admitted: None,
                    },
                )
            }
        };
        if spec.effect.is_some() && self.effect_paths.read().expect("effect paths").is_none() {
            return Err(SubmitError::Rejected {
                reason: "root effects disabled".into(),
            });
        }
        spec.admitted = admitted;
        self.execute_edit_opts(op, base, &edit, SubmitOptions::default(), spec)
    }

    fn publish_progress(&self, journal: &Journal) {
        let mut store = self.engine.store.lock().expect("config mutex");
        store.journal = journal.clone();
        for entry in &journal.operations {
            if entry.record.phases.iter().any(|phase| {
                phase.phase == ConfigPhase::Persist && phase.outcome == PhaseOutcome::Confirmed
            }) {
                if let Some(admission) = &entry.admission {
                    if admission.commit_revision > store.config_revision {
                        store.config = if entry.command == Some(journal::CommandIntent::FullReset) {
                            None
                        } else {
                            entry.candidate.clone()
                        };
                        store.config_revision = admission.commit_revision;
                    }
                }
            }
        }
        self.publish_store(&store);
    }

    fn freeze_storage_fault(&self, store: &ConfigStore, op: OperationId, error: &std::io::Error) {
        *self.engine.store.lock().expect("config mutex") = store.clone();
        self.publish_store(store);
        self.publisher.update(|snapshot| {
            snapshot.storage_fault = Some(format!(
                "journal storage fault for operation {}: {}; restart core to reconcile durable evidence",
                op.0, error
            ));
            snapshot.mutation_held = true;
            snapshot.operations_in_flight.clear();
        });
    }

    fn publish_store(&self, store: &ConfigStore) {
        self.publisher.update(|snapshot| {
            snapshot.config = store.config.clone();
            snapshot.debug_leases = store.journal.debug_leases.clone();
            snapshot.debug_lease = !snapshot.debug_leases.is_empty();
            snapshot.config_revision = store.config_revision;
            snapshot.operations = store
                .journal
                .operations
                .iter()
                .map(|entry| entry.record.clone())
                .collect();
            snapshot.operations_in_flight = store
                .journal
                .operations
                .iter()
                .filter(|entry| entry.record.settled.is_none())
                .map(|entry| entry.record.op)
                .collect();
            snapshot.mutation_held =
                snapshot.storage_fault.is_some() || store.journal.has_unknown();
            snapshot.held_resources = store
                .journal
                .expired_holds
                .iter()
                .copied()
                .chain(
                    store
                        .journal
                        .operations
                        .iter()
                        .flat_map(JournalEntry::held_phases),
                )
                .collect();
            snapshot.held_resources.sort_by_key(|phase| *phase as u8);
            snapshot.held_resources.dedup();
            for entry in &store.journal.operations {
                let revision = entry
                    .admission
                    .as_ref()
                    .map_or(store.config_revision, |admission| admission.commit_revision);
                for phase in &entry.record.phases {
                    if matches!(
                        phase.phase,
                        ConfigPhase::Native | ConfigPhase::Ports | ConfigPhase::Secret
                    ) && phase.outcome != PhaseOutcome::NotAttempted
                    {
                        snapshot
                            .apply_evidence
                            .retain(|evidence| evidence.phase != phase.phase);
                        snapshot
                            .apply_evidence
                            .push(vpnhide_core_rpc::ApplyEvidence {
                                phase: phase.phase,
                                desired_revision: revision,
                                op: entry.record.op,
                                outcome: phase.outcome,
                            });
                    }
                }
            }
        });
    }

    pub fn acquire_debug_lease(
        &self,
        id: DebugLeaseId,
        owner: DebugLeaseOwner,
    ) -> Result<(), SubmitError> {
        self.call_writer(move |state| {
            let mut store = state.engine.store.lock().expect("config mutex").clone();
            let existing = store
                .journal
                .debug_leases
                .iter()
                .find(|lease| lease.id == id);
            if existing.is_some_and(|lease| lease.owner != owner) {
                return Err(SubmitError::IdReused);
            }
            if existing.is_none() && store.journal.debug_leases.len() >= 16 {
                return Err(SubmitError::Busy);
            }
            let relevant_hold = |phase| {
                phase == ConfigPhase::Persist
                    || (state.native_enabled()
                        && matches!(phase, ConfigPhase::Native | ConfigPhase::Secret))
            };
            if store
                .journal
                .expired_holds
                .iter()
                .copied()
                .any(relevant_hold)
                || store
                    .journal
                    .operations
                    .iter()
                    .flat_map(JournalEntry::held_phases)
                    .any(relevant_hold)
            {
                return Err(SubmitError::Unavailable);
            }
            let already_active = state
                .live_leases
                .lock()
                .expect("live leases")
                .get(&id)
                .is_some_and(|lease| lease.active)
                && store.config.as_ref().is_some_and(|config| config.debug);
            let config = store.config.as_ref().ok_or(SubmitError::Unavailable)?;
            if store.journal.debug_restore.is_none() {
                store.journal.debug_restore = Some((config.debug, config.debug_switch));
            }
            if let Some(lease) = store
                .journal
                .debug_leases
                .iter_mut()
                .find(|lease| lease.id == id)
            {
                lease.expires_at_ms = lease_deadline(owner);
            } else {
                store
                    .journal
                    .debug_leases
                    .push(vpnhide_core_rpc::DebugLease {
                        id,
                        owner,
                        expires_at_ms: lease_deadline(owner),
                    });
            }
            state.persist_store(&store)?;
            state.live_leases.lock().expect("live leases").insert(
                id,
                LiveLease {
                    deadline: std::time::Instant::now() + lease_duration(owner),
                    active: already_active,
                },
            );
            if !already_active && !state.write_debug_direct(true) {
                return Err(SubmitError::Unavailable);
            }
            state
                .live_leases
                .lock()
                .expect("live leases")
                .get_mut(&id)
                .expect("registered lease")
                .active = true;
            Ok(())
        })?
    }

    pub fn release_debug_lease(&self, id: DebugLeaseId) -> Result<(), SubmitError> {
        self.call_writer(move |state| state.release_debug_lease_direct(id))?
    }

    fn release_recording_debug_lease(&self, id: DebugLeaseId) -> Result<(), SubmitError> {
        self.call_writer_owned(move |state| state.release_debug_lease_direct(id), true)?
    }

    fn release_debug_lease_direct(&self, id: DebugLeaseId) -> Result<(), SubmitError> {
        let mut store = self.engine.store.lock().expect("config mutex").clone();
        store.journal.debug_leases.retain(|lease| lease.id != id);
        self.persist_store(&store)?;
        self.live_leases.lock().expect("live leases").remove(&id);
        self.restore_debug_if_unleased(true)
    }

    pub(crate) fn expire_debug_leases(&self) {
        if self.shutting_down.load(Ordering::SeqCst) || self.snapshot().storage_fault.is_some() {
            return;
        }
        if self.native_enabled() && self.native_handle().is_none() {
            return;
        }
        let mut store = self.engine.store.lock().expect("config mutex").clone();
        let before = store.journal.debug_leases.len();
        let now = std::time::Instant::now();
        let live = self.live_leases.lock().expect("live leases").clone();
        store.journal.debug_leases.retain(|lease| {
            live.get(&lease.id)
                .is_some_and(|lease| lease.deadline > now)
        });
        if before != store.journal.debug_leases.len() && self.persist_store(&store).is_err() {
            return;
        }
        self.live_leases
            .lock()
            .expect("live leases")
            .retain(|id, _| {
                store
                    .journal
                    .debug_leases
                    .iter()
                    .any(|lease| lease.id == *id)
            });
        let _ = self.restore_debug_if_unleased(false);
    }

    fn reconcile_held_persist(&self, store: &mut ConfigStore) -> Result<bool, SubmitError> {
        let held = store.journal.expired_holds.contains(&ConfigPhase::Persist)
            || store
                .journal
                .operations
                .iter()
                .any(|entry| entry.held_phases().contains(&ConfigPhase::Persist));
        if !held {
            return Ok(false);
        }
        let path = self
            .config_path
            .as_deref()
            .ok_or(SubmitError::Unavailable)?;
        config_file::sync_after_rename(path.parent().unwrap_or_else(|| std::path::Path::new(".")))
            .map_err(|_| SubmitError::Unavailable)?;
        let durable = config_file::read(path).map_err(|_| SubmitError::Unavailable)?;
        let proof = store
            .journal
            .operations
            .iter()
            .rev()
            .filter(|entry| entry.held_phases().contains(&ConfigPhase::Persist))
            .map(|entry| journal::PendingPersist {
                admission: entry.admission.clone(),
                candidate: entry.candidate.clone(),
                reset: entry.command == Some(journal::CommandIntent::FullReset),
            })
            .chain(store.journal.pending_persist.clone())
            .find(|proof| crate::recovery::pending_persist_matches(proof, &durable));
        if let Some(proof) = proof {
            let admission = proof.admission.ok_or(SubmitError::Unavailable)?;
            store.config_revision = store.config_revision.max(admission.commit_revision);
            config_edit::note_writes(
                &mut store.field_revisions,
                &admission.writes,
                admission.commit_revision,
            );
            store.journal.field_revisions = store.field_revisions.clone();
            store.journal.config_revision = store.config_revision;
            store.config = durable;
        } else if durable != store.config {
            return Err(SubmitError::Unavailable);
        }
        Ok(true)
    }

    fn restore_debug_if_unleased(&self, retry_known_failure: bool) -> Result<(), SubmitError> {
        let mut store = self.engine.store.lock().expect("config mutex").clone();
        if !store.journal.debug_leases.is_empty() {
            return Ok(());
        }
        let Some((previous, previous_switch)) = store.journal.debug_restore else {
            return Ok(());
        };
        if self.reconcile_held_persist(&mut store)? {
            self.persist_store(&store)?;
        }
        let mut retrying_apply = false;
        if let Some(op) = store.journal.debug_restore_op {
            if let Some(entry) = store.journal.get(op) {
                if entry.record.settled == Some(PhaseOutcome::Confirmed) {
                    store.journal.debug_restore = None;
                    store.journal.debug_restore_op = None;
                    return self.persist_store(&store);
                }
                if entry.superseded_by.is_some()
                    || (entry.record.settled == Some(PhaseOutcome::FailedKnown)
                        && retry_known_failure)
                {
                    // A superseding repair does not prove this baseline target.
                    // Recompute it below and create new desired-state evidence.
                    retrying_apply = true;
                    store.journal.debug_restore_op = None;
                    self.persist_store(&store)?;
                } else {
                    return Err(SubmitError::Unavailable);
                }
            }
        }
        let Some(config) = &store.config else {
            store.journal.debug_restore = None;
            store.journal.debug_restore_op = None;
            return self.persist_store(&store);
        };
        let target = if config.debug_switch != previous_switch {
            config.debug_switch
        } else {
            previous
        };
        let op = if let Some(op) = store.journal.debug_restore_op {
            op
        } else {
            let op = self.issue_operation_id_direct()?;
            store = self.engine.store.lock().expect("config mutex").clone();
            store.journal.debug_restore_op = Some(op);
            self.persist_store(&store)?;
            op
        };
        self.execute_edit_opts(
            op,
            BaseRef {
                revision: self.config_revision(),
            },
            &ConfigEdit::Toggle {
                field: vpnhide_core_rpc::ConfigToggle::Debug,
                enabled: target,
            },
            SubmitOptions {
                plan: retrying_apply.then_some(
                    match (self.native_enabled(), self.ports_enabled()) {
                        (true, true) => PlanKind::PersistThenNativeThenPorts,
                        (true, false) => PlanKind::PersistThenNative,
                        (false, true) => PlanKind::PersistThenPorts,
                        (false, false) => PlanKind::PersistOnly,
                    },
                ),
                ..SubmitOptions::default()
            },
            CommandSpec {
                lease: true,
                ..Default::default()
            },
        )?;
        let mut store = self.engine.store.lock().expect("config mutex").clone();
        if store
            .journal
            .get(op)
            .is_some_and(|entry| entry.record.settled == Some(PhaseOutcome::Confirmed))
        {
            store.journal.debug_restore = None;
            store.journal.debug_restore_op = None;
            self.persist_store(&store)
        } else {
            Err(SubmitError::Unavailable)
        }
    }

    fn persist_store(&self, store: &ConfigStore) -> Result<(), SubmitError> {
        if self.snapshot().storage_fault.is_some() {
            return Err(SubmitError::Unavailable);
        }
        if let Some(path) = &self.journal_path {
            if let Err(error) = journal::write_atomic(path, &store.journal) {
                let previous = self.engine.store.lock().expect("config mutex").clone();
                self.freeze_storage_fault(&previous, OperationId(0), &error);
                return Err(SubmitError::Unavailable);
            }
        }
        *self.engine.store.lock().expect("config mutex") = store.clone();
        self.publish_store(store);
        Ok(())
    }

    fn write_debug_direct(&self, enabled: bool) -> bool {
        let Ok(op) = self.issue_operation_id_direct() else {
            return false;
        };
        let edit = ConfigEdit::Toggle {
            field: vpnhide_core_rpc::ConfigToggle::Debug,
            enabled,
        };
        let result = self.execute_edit_opts(
            op,
            BaseRef {
                revision: self.config_revision(),
            },
            &edit,
            SubmitOptions {
                plan: Some(if self.native_enabled() {
                    PlanKind::PersistThenNative
                } else {
                    PlanKind::PersistOnly
                }),
                ..Default::default()
            },
            CommandSpec {
                lease: true,
                ..Default::default()
            },
        );
        result.is_ok()
            && self
                .get_operation_record(op)
                .is_some_and(|record| record.settled == Some(PhaseOutcome::Confirmed))
            && self
                .current_debug()
                .is_some_and(|(debug, _)| debug == enabled)
    }
    /// A core with no durable file — the config lives only in memory. Used by host tests.
    pub fn new(publisher: Publisher) -> Arc<Self> {
        Arc::new(CoreState {
            publisher,
            engine: Default::default(),
            shutting_down: std::sync::atomic::AtomicBool::new(false),
            shutdown_gate: Mutex::new(()),
            logcat_gate: Mutex::new(()),
            instance_guard: std::sync::OnceLock::new(),
            heavy_rpc: Arc::new(tokio::sync::Semaphore::new(2)),
            live_leases: Mutex::new(Default::default()),
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
        .with_debug_control()
    }

    /// Point the logcat recorder's debug lease at this core (weakly: no cycle).
    fn with_debug_control(self: Arc<Self>) -> Arc<Self> {
        self.engine.start(Arc::downgrade(&self));
        self.logcat
            .set_debug_control(Some(Arc::new(CoreDebugControl(Arc::downgrade(&self)))));
        self
    }

    /// Invalidate in-flight external dispatch. Delayed workers whose admit ticket no
    /// longer matches will refuse to run further effects (no blind late dispatch).
    pub fn invalidate_dispatch(&self) {
        self.dispatch_fence.fetch_add(1, Ordering::SeqCst);
    }

    /// Load the durable config from `path` and own restart recovery. Without a journal the revision is reconstructed as 1 when a config is
    /// present (legacy behaviour). With a journal, the durable revision counter and
    /// operation records are restored and unresolved work is fenced.
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
            config_file::ConfigLoad::Present(cfg) => {
                config_edit::validate_candidate(&cfg)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                Some(cfg)
            }
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
        // Leases belong to the daemon instance, not wall-clock timestamps.
        // No capture can keep its previous daemon-owned execution across restart.
        // Preserve debug_restore so maintenance restores the user's baseline.
        journal.debug_leases.clear();

        // Evidence readback may resolve Unknown → Confirmed/FailedKnown without dispatch.
        let sources = crate::recovery::EvidenceSources {
            config_path: Some(path.as_path()),
        };
        let mut journal_dirty = false;
        for entry in &mut journal.operations {
            if crate::recovery::reconcile_entry(entry, &sources) {
                journal_dirty = true;
            }
            if entry.record.settled == Some(PhaseOutcome::Unknown) {
                entry.finalized_unknown = true;
            }
        }
        // Advance durable revision when Persist evidence proves the write landed.
        let mut config_revision_from_journal = journal.config_revision;
        for entry in &journal.operations {
            let next = crate::recovery::revision_after_reconcile(
                entry,
                config_revision_from_journal,
                &file_config,
            );
            if next != config_revision_from_journal {
                config_revision_from_journal = next;
                journal_dirty = true;
            }
        }
        if let Some(proof) = &journal.pending_persist {
            if crate::recovery::pending_persist_matches(proof, &file_config) {
                if let Some(admission) = &proof.admission {
                    config_revision_from_journal =
                        config_revision_from_journal.max(admission.commit_revision);
                    config_edit::note_writes(
                        &mut journal.field_revisions,
                        &admission.writes,
                        admission.commit_revision,
                    );
                }
            }
        }
        journal.config_revision = config_revision_from_journal;
        for entry in &journal.operations {
            if crate::recovery::persist_commit_observed(entry, &file_config) {
                if let Some(admission) = &entry.admission {
                    config_edit::note_writes(
                        &mut journal.field_revisions,
                        &admission.writes,
                        admission.commit_revision,
                    );
                }
            }
        }

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

        let field_revisions = journal.field_revisions.clone();
        let state = Arc::new(CoreState {
            publisher,
            engine: crate::command_engine::CommandEngine::with_store(ConfigStore {
                config: config.clone(),
                config_revision,
                journal,
                field_revisions,
            }),
            shutting_down: std::sync::atomic::AtomicBool::new(false),
            shutdown_gate: Mutex::new(()),
            logcat_gate: Mutex::new(()),
            instance_guard: std::sync::OnceLock::new(),
            heavy_rpc: Arc::new(tokio::sync::Semaphore::new(2)),
            live_leases: Mutex::new(Default::default()),
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
        })
        .with_debug_control();
        let store = state.engine.store.lock().expect("config mutex").clone();
        state.publish_store(&store);
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
        self.engine
            .store
            .lock()
            .expect("config mutex")
            .config
            .is_some()
    }

    /// Current config revision (boot repair base).
    pub fn config_revision(&self) -> u64 {
        self.engine
            .store
            .lock()
            .expect("config mutex")
            .config_revision
    }

    /// True when an Unknown operation still Holds mutation admission.
    pub fn has_unknown_hold(&self) -> bool {
        let store = self.engine.store.lock().expect("config mutex");
        store.journal.has_unknown()
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
        let boot = handle.lock().expect("native boot");
        let backend = boot.selected().or_else(|| {
            boot.pending_authentication()
                .then_some(vpnhide_core_rpc::NativeBackend::Kpm)
        });
        drop(boot);
        let Some(backend) = backend else {
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
        self.engine
            .store
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
        let _logcat = self.logcat_gate.lock().expect("logcat gate");
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err(LogcatRecordingError::Failed("core is shutting down".into()));
        }
        let env = self.probe_env.read().expect("probe env").clone();
        self.logcat.start(session, &env)
    }

    /// Stop a logcat session and hand its file to the app (blocking).
    pub fn stop_logcat_recording(
        &self,
        session: LogcatSessionId,
    ) -> Result<LogcatSessionStatus, LogcatRecordingError> {
        let _logcat = self.logcat_gate.lock().expect("logcat gate");
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err(LogcatRecordingError::Failed("core is shutting down".into()));
        }
        let env = self.probe_env.read().expect("probe env").clone();
        self.logcat.stop(session, &env)
    }

    /// The app saved a logcat recording: core deletes its copies (blocking).
    pub fn ack_logcat_recording(
        &self,
        session: LogcatSessionId,
    ) -> Result<LogcatSessionStatus, LogcatRecordingError> {
        let _logcat = self.logcat_gate.lock().expect("logcat gate");
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err(LogcatRecordingError::Failed("core is shutting down".into()));
        }
        let env = self.probe_env.read().expect("probe env").clone();
        self.logcat.ack(session, &env)
    }

    /// Drop a logcat session and its files (blocking).
    pub fn discard_logcat_recording(
        &self,
        session: LogcatSessionId,
    ) -> Result<(), LogcatRecordingError> {
        let _logcat = self.logcat_gate.lock().expect("logcat gate");
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err(LogcatRecordingError::Failed("core is shutting down".into()));
        }
        let env = self.probe_env.read().expect("probe env").clone();
        self.logcat.discard(session, &env)
    }

    /// The currently published snapshot (tests and diagnostics).
    pub fn snapshot(&self) -> vpnhide_core_rpc::StateSnapshot {
        self.publisher.current().snapshot
    }

    /// Whether a logcat session holds debug logging on.
    pub fn debug_lease_held(&self) -> bool {
        !self
            .engine
            .store
            .lock()
            .expect("config mutex")
            .journal
            .debug_leases
            .is_empty()
    }

    /// The stored config's `(debug, debugSwitch)`, if core holds a config.
    fn current_debug(&self) -> Option<(bool, bool)> {
        let store = self.engine.store.lock().expect("config mutex");
        store.config.as_ref().map(|c| (c.debug, c.debug_switch))
    }

    /// Run a batch of fixed probes (blocking).
    pub fn collect_diagnostics(&self, request: &DiagnosticsRequest) -> DiagnosticsReport {
        let env = self.probe_env.read().expect("probe env").clone();
        crate::diagnostics::collect(&env, request)
    }

    /// Query a settled/in-flight operation record.
    pub fn get_operation_record(&self, op: OperationId) -> Option<OperationRecord> {
        self.engine
            .store
            .lock()
            .expect("config mutex")
            .journal
            .get(op)
            .map(|entry| entry.record.clone())
    }

    /// Admit a typed edit against current field history and run its phase plan.
    /// Unrelated stale fields merge; overlapping writes reject the whole edit.
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
        let edit = edit.clone();
        self.call_writer(move |state| {
            state.execute_edit_opts(op, base, &edit, options, CommandSpec::default())
        })?
    }

    fn execute_edit_opts(
        &self,
        op: OperationId,
        base: BaseRef,
        edit: &ConfigEdit,
        options: SubmitOptions,
        mut spec: CommandSpec,
    ) -> Result<Accepted, SubmitError> {
        if self.snapshot().startup_error.is_some() || self.snapshot().storage_fault.is_some() {
            return Err(SubmitError::Unavailable);
        }
        let mut store = self.engine.store.lock().expect("config mutex").clone();
        // Deduplication precedes current-base and uncertainty gates: a lost
        // receipt may be queried using its original base, but never a new payload.
        if let Some(previous) = store.journal.get(op) {
            // No secret or secret verifier is durable. Repeating a secret-bearing
            // request cannot establish payload identity; recover it by querying
            // the original receipt rather than claiming the new secret matched.
            if matches!(spec.intent, journal::CommandIntent::SetSuperkey { .. }) {
                return Err(SubmitError::IdReused);
            }
            return if previous.base == base
                && previous.edit == *edit
                && previous
                    .command
                    .as_ref()
                    .unwrap_or(&journal::CommandIntent::Edit)
                    == &spec.intent
            {
                Ok(Accepted { op })
            } else {
                Err(SubmitError::IdReused)
            };
        }
        // Retention must not turn a pruned retry into a new mutation.
        if store.journal.is_retired(op) {
            return Err(SubmitError::Expired);
        }
        if op.0 == 0 || op.0 > store.journal.issued_through {
            return Err(SubmitError::Rejected {
                reason: "operation identity was not allocated by core".into(),
            });
        }
        config_edit::validate_edit(edit).map_err(|reason| SubmitError::Rejected { reason })?;
        let repair = matches!(edit, ConfigEdit::RepairActivation);
        let held_persist = store.journal.expired_holds.contains(&ConfigPhase::Persist)
            || store
                .journal
                .operations
                .iter()
                .any(|entry| entry.held_phases().contains(&ConfigPhase::Persist));
        let recovery_action = match spec.intent {
            journal::CommandIntent::SetSuperkey { .. } => {
                Some(vpnhide_core_rpc::recovery::RecoveryAction::SetSuperkey)
            }
            journal::CommandIntent::ClearSuperkey => {
                Some(vpnhide_core_rpc::recovery::RecoveryAction::ClearSuperkey)
            }
            journal::CommandIntent::FullReset => {
                Some(vpnhide_core_rpc::recovery::RecoveryAction::FullReset)
            }
            journal::CommandIntent::CleanupLegacy => {
                Some(vpnhide_core_rpc::recovery::RecoveryAction::Cleanup)
            }
            _ if repair => Some(vpnhide_core_rpc::recovery::RecoveryAction::Repair),
            _ => None,
        };
        let held = if held_persist {
            vec![ConfigPhase::Persist]
        } else {
            Vec::new()
        };
        if recovery_action
            .is_some_and(|action| !vpnhide_core_rpc::recovery::permits_held(action, &held))
        {
            return Err(SubmitError::Unavailable);
        }
        if options.automatic_repair {
            // This runs inside the writer, against the current journal: a hold
            // created while boot queued cannot slip past a snapshot-only check.
            let held: Vec<_> = store
                .journal
                .expired_holds
                .iter()
                .copied()
                .chain(
                    store
                        .journal
                        .operations
                        .iter()
                        .flat_map(JournalEntry::held_phases),
                )
                .collect();
            let kind =
                options
                    .plan
                    .unwrap_or(match (self.native_enabled(), self.ports_enabled()) {
                        (true, true) => PlanKind::NativeThenPorts,
                        (true, false) => PlanKind::NativeOnly,
                        (false, true) => PlanKind::PortsOnly,
                        (false, false) => PlanKind::PersistOnly,
                    });
            if !repair
                || operation::plan_kind(kind)
                    .iter()
                    .any(|phase| !vpnhide_core_rpc::recovery::permits_boot_phase(*phase, &held))
            {
                return Err(SubmitError::Unavailable);
            }
        }
        let persist_readback = if repair || spec.lease {
            self.reconcile_held_persist(&mut store)?
        } else {
            false
        };
        if store.journal.has_unknown() && !spec.lease && !repair && recovery_action.is_none() {
            return Err(SubmitError::Unavailable);
        }
        if base.revision < store.journal.conflict_floor && !repair && !spec.lease && !spec.reset {
            return Err(SubmitError::Conflict {
                field: "config history expired".into(),
            });
        }
        // Field-level conflict: unrelated edits merge against fresh state; same-field
        // (or Replace domain) overlap since base → Conflict.
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
                ConfigEdit::RepairActivation
                    if matches!(spec.intent, journal::CommandIntent::CleanupLegacy) =>
                {
                    CanonicalConfig::default()
                }
                ConfigEdit::RepairActivation => return Err(SubmitError::Unavailable),
                _ => return Err(SubmitError::Unavailable),
            },
        };
        let repair = matches!(edit, ConfigEdit::RepairActivation);
        let mut next = apply_canonical_edit(&current, edit);
        // A logcat session's lease keeps debug logging on whatever the edit says (v9).
        if !repair && self.debug_lease_held() {
            next.debug = true;
        }
        config_edit::validate_candidate(&next)
            .map_err(|reason| SubmitError::Rejected { reason })?;
        let changed = if spec.reset {
            // Reset is also a canonical generation fence, including a prior
            // uncertain bootstrap that memory could not confirm.
            true
        } else {
            store.config.as_ref() != Some(&next)
        };

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
        } else if changed || matches!(spec.effect, Some((ConfigPhase::Secret, _))) {
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
        let mut plan = operation::plan_kind(kind);
        if persist_readback && !plan.contains(&ConfigPhase::Persist) {
            plan.insert(0, ConfigPhase::Persist);
        }
        if let Some(forced) = &spec.plan {
            plan = forced.clone();
        } else if let Some((phase, _)) = &spec.effect {
            let index = usize::from(plan.first() == Some(&ConfigPhase::Persist));
            plan.insert(index, *phase);
        }
        let native_wire = if changed
            && !repair
            && !matches!(spec.effect, Some((ConfigPhase::Secret, _)))
            && plan.contains(&ConfigPhase::Native)
        {
            self.admit_native_capacity(&next)?
        } else {
            None
        };

        // Claim a dispatch ticket so a delayed predecessor (or explicit invalidate)
        // cannot run external effects after this admission.
        let ticket = self.dispatch_fence.fetch_add(1, Ordering::SeqCst) + 1;
        self.publisher.update(|snapshot| {
            snapshot.operations_in_flight.push(op);
        });

        let mut runner = RealPhaseRunner::new_with_ports(
            self.config_path.clone(),
            self.kmod_ctl_path.clone(),
            ports,
        );
        runner.native = self.native_handle();
        runner.effect_paths = self.effect_paths.read().expect("effect paths").clone();
        runner.effects = spec.effect.iter().cloned().collect();
        runner.reset_config = spec.reset;
        runner.remember_secret = !matches!(
            spec.intent,
            journal::CommandIntent::SetSuperkey { remember: false }
        );
        runner.epoch = self.epoch.load(Ordering::SeqCst);
        runner.native_wire = native_wire;
        runner.ports_source = options.ports_source;
        runner.ports_netd_ready = options.netd_ready;

        let admission = journal::Admission {
            from_revision: store.config_revision,
            commit_revision: if changed && plan.contains(&ConfigPhase::Persist) {
                store
                    .config_revision
                    .max(
                        store
                            .journal
                            .operations
                            .iter()
                            .filter(|entry| entry.held_phases().contains(&ConfigPhase::Persist))
                            .filter_map(|entry| {
                                entry
                                    .admission
                                    .as_ref()
                                    .map(|admission| admission.commit_revision)
                            })
                            .chain(store.journal.pending_persist.as_ref().and_then(|proof| {
                                proof
                                    .admission
                                    .as_ref()
                                    .map(|admission| admission.commit_revision)
                            }))
                            .max()
                            .unwrap_or(store.config_revision),
                    )
                    .checked_add(1)
                    .ok_or(SubmitError::Unavailable)?
            } else {
                store.config_revision
            },
            writes: if changed {
                config_edit::write_set(edit)
            } else {
                Vec::new()
            },
        };
        store.journal.upsert(JournalEntry {
            admission: Some(admission.clone()),
            command: Some(spec.intent.clone()),
            superseded_by: None,
            superseded_phases: Vec::new(),
            finalized_unknown: false,
            base,
            edit: edit.clone(),
            candidate: Some(next.clone()),
            native_intent: None,
            record: OperationRecord {
                op,
                phases: plan
                    .iter()
                    .map(|phase| PhaseRecord {
                        phase: *phase,
                        outcome: PhaseOutcome::NotAttempted,
                    })
                    .collect(),
                settled: None,
                ports: None,
            },
        });
        *self.engine.store.lock().expect("config mutex") = store.clone();
        self.publish_store(&store);
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
                runner: &runner,
                state: self,
                admitted: spec.admitted.take(),
            };
            operation::run_plan_hooked(&plan, &next, changed, &runner, Some(&mut hook))
        } else {
            if let Some(reply) = spec.admitted.take() {
                let _ = reply.send(Ok(Accepted { op }));
            }
            let mut hook = FenceOnlyHook {
                ticket,
                fence: &self.dispatch_fence,
            };
            operation::run_plan_hooked(&plan, &next, changed, &runner, Some(&mut hook))
        };

        if outcome.persisted {
            store.config = if spec.reset { None } else { Some(next.clone()) };
            store.config_revision = admission.commit_revision;
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
            let pending = store.journal.clone();
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
                // A failed sync is not a durable terminal receipt. Preserve the
                // pending intent in memory and on disk for evidence reconciliation.
                store.journal = pending;
                self.freeze_storage_fault(&store, op, &e);
                return Err(SubmitError::Unavailable);
            }
        }
        if self.journal_path.is_none() {
            if let Some(entry) = store.journal.get_mut(op) {
                entry.record = record.clone();
            }
        }

        let durable_settled = store.journal.clone();
        if spec.lease
            || (matches!(spec.intent, journal::CommandIntent::Edit) && repair)
            || matches!(
                spec.intent,
                journal::CommandIntent::FullReset
                    | journal::CommandIntent::CleanupLegacy
                    | journal::CommandIntent::SetSuperkey { .. }
                    | journal::CommandIntent::ClearSuperkey
            )
        {
            let replacement_confirmed = |resource| {
                outcome.phases.iter().any(|phase| {
                    phase.phase == resource && phase.outcome == PhaseOutcome::Confirmed
                }) || (spec.reset
                    && resource == ConfigPhase::Secret
                    && outcome.phases.iter().any(|phase| {
                        phase.phase == ConfigPhase::Cleanup
                            && phase.outcome == PhaseOutcome::Confirmed
                    }))
            };
            store
                .journal
                .expired_holds
                .retain(|phase| !replacement_confirmed(*phase));
            if !store.journal.expired_holds.contains(&ConfigPhase::Persist) {
                store.journal.pending_persist = None;
            }
            for entry in &mut store.journal.operations {
                if entry.record.op == op {
                    continue;
                }
                for held in entry.held_phases() {
                    if (spec.reset && held == ConfigPhase::Secret && replacement_confirmed(held))
                        || outcome.phases.iter().any(|phase| {
                            phase.phase == held
                                && (phase.outcome == PhaseOutcome::Confirmed
                                    || (phase.outcome == PhaseOutcome::FailedKnown
                                        && entry.record.phases.iter().any(|old| {
                                            old.phase == held
                                                && old.outcome == PhaseOutcome::NotAttempted
                                        })))
                        })
                    {
                        entry.superseded_phases.push(held);
                    }
                }
                if entry.record.settled == Some(PhaseOutcome::Unknown)
                    && entry.held_phases().is_empty()
                {
                    entry.superseded_by = Some(op);
                }
            }
        }
        store.journal.prune();
        store.field_revisions = store.journal.field_revisions.clone();
        if let Some(path) = &self.journal_path {
            if let Err(error) = journal::write_atomic(path, &store.journal) {
                // Settlement already synced. A failed retention/repair-metadata
                // update must never rewrite that terminal receipt as Unknown.
                store.journal = durable_settled;
                self.freeze_storage_fault(&store, op, &error);
                return Ok(Accepted { op });
            }
        }

        *self.engine.store.lock().expect("config mutex") = store.clone();
        self.publish_store(&store);
        let revision = store.config_revision;
        // Publish while still owning the mutation lane so a later accepted edit
        // cannot publish first and then be overwritten by this older snapshot.
        self.publisher.update(|snapshot| {
            if outcome.persisted {
                snapshot.config = if spec.reset { None } else { Some(next) };
                snapshot.config_revision = revision;
            }
            snapshot
                .operations_in_flight
                .retain(|running| *running != op);
            snapshot.mutation_held =
                snapshot.storage_fault.is_some() || store.journal.has_unknown();
        });
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
        self.engine
            .store
            .lock()
            .expect("config mutex")
            .journal
            .get(op)
            .map(|entry| entry.record.clone())
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

    fn after(&mut self, phase: ConfigPhase, _outcome: PhaseOutcome) -> Result<(), PhaseOutcome> {
        if phase == ConfigPhase::Persist {
            crate::recovery::maybe_crash_for_test("persist");
        }
        if phase == ConfigPhase::Native {
            crate::recovery::maybe_crash_for_test("native");
        }
        Ok(())
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
    runner: &'a RealPhaseRunner,
    state: &'a CoreState,
    admitted: Option<crate::command_engine::AdmissionReply>,
}

impl PhaseHook for JournalPhaseHook<'_> {
    fn before(&mut self, phase: ConfigPhase) -> Result<(), PhaseOutcome> {
        let native_intent = if phase == ConfigPhase::Native {
            self.runner.native_intent(self.candidate)
        } else {
            None
        };
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
            native_intent,
        )
        .map_err(|e| {
            eprintln!("core: journal intent failed before {phase:?}: {e}");
            if config_file::is_post_rename_error(&e) {
                PhaseOutcome::Unknown
            } else {
                PhaseOutcome::FailedKnown
            }
        })?;
        self.state.publish_progress(self.journal);
        if let Some(reply) = self.admitted.take() {
            let _ = reply.send(Ok(Accepted { op: self.op }));
        }
        crate::recovery::maybe_delay_for_test(phase_delay_point(phase));
        if self.fence.load(Ordering::SeqCst) != self.ticket {
            eprintln!("core: dispatch fenced before {phase:?} (delayed predecessor)");
            return Err(PhaseOutcome::FailedKnown);
        }
        crate::recovery::maybe_crash_for_test(phase_intent_point(phase));
        Ok(())
    }

    fn after(&mut self, phase: ConfigPhase, outcome: PhaseOutcome) -> Result<(), PhaseOutcome> {
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
            let prior = self
                .journal
                .get(self.op)
                .cloned()
                .expect("admitted operation");
            let entry = JournalEntry {
                admission: prior.admission,
                command: prior.command,
                superseded_by: prior.superseded_by,
                superseded_phases: prior.superseded_phases,
                finalized_unknown: prior.finalized_unknown,
                base: self.base,
                edit: self.edit.clone(),
                candidate: Some(self.candidate.clone()),
                native_intent: None,
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
                return Err(PhaseOutcome::Unknown);
            }
            self.state.publish_progress(self.journal);
            crate::recovery::maybe_crash_for_test("persist");
        }
        if phase == ConfigPhase::Native {
            crate::recovery::maybe_crash_for_test("native");
        }
        Ok(())
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
        crate::publisher::bounded_response(self.state.publisher.state_since(epoch, revision).await)
    }

    async fn allocate_operation_id(self, _ctx: Context) -> Result<OperationId, SubmitError> {
        let state = self.state;
        tokio::task::spawn_blocking(move || state.issue_operation_id())
            .await
            .unwrap_or(Err(SubmitError::Unavailable))
    }

    async fn submit_command(
        self,
        _ctx: Context,
        op: OperationId,
        command: CoreCommand,
    ) -> Result<Accepted, SubmitError> {
        let state = self.state;
        tokio::task::spawn_blocking(move || {
            if state.shutting_down.load(Ordering::SeqCst) {
                return Err(SubmitError::Unavailable);
            }
            state.engine.submit(op, command)
        })
        .await
        .unwrap_or(Err(SubmitError::Unavailable))
    }

    async fn acquire_debug_lease(
        self,
        _ctx: Context,
        id: DebugLeaseId,
        owner: DebugLeaseOwner,
    ) -> Result<(), SubmitError> {
        let state = self.state;
        tokio::task::spawn_blocking(move || state.acquire_debug_lease(id, owner))
            .await
            .unwrap_or(Err(SubmitError::Unavailable))
    }

    async fn release_debug_lease(self, _ctx: Context, id: DebugLeaseId) -> Result<(), SubmitError> {
        let state = self.state;
        tokio::task::spawn_blocking(move || state.release_debug_lease(id))
            .await
            .unwrap_or(Err(SubmitError::Unavailable))
    }

    async fn get_operation(self, _ctx: Context, op: OperationId) -> Option<OperationRecord> {
        let state = self.state;
        tokio::task::spawn_blocking(move || state.get_operation(op))
            .await
            .unwrap_or(None)
    }

    async fn collect_diagnostics(
        self,
        _ctx: Context,
        request: DiagnosticsRequest,
    ) -> DiagnosticsReport {
        let state = self.state.clone();
        let Ok(permit) = state.heavy_rpc.clone().try_acquire_owned() else {
            return DiagnosticsReport {
                outputs: request
                    .probes
                    .first()
                    .map(|probe| vpnhide_core_rpc::ProbeOutput {
                        probe: probe.clone(),
                        exit: -1,
                        output: "core diagnostics busy".into(),
                        file: None,
                        artifacts: Vec::new(),
                        truncated: false,
                    })
                    .into_iter()
                    .collect(),
                dmesg_cleared: false,
            };
        };
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            state.collect_diagnostics(&request)
        })
        .await
        .unwrap_or_default()
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

    async fn ack_logcat_recording(
        self,
        _ctx: Context,
        session: LogcatSessionId,
    ) -> Result<LogcatSessionStatus, LogcatRecordingError> {
        let state = self.state.clone();
        tokio::task::spawn_blocking(move || state.ack_logcat_recording(session))
            .await
            .unwrap_or_else(|_| Err(LogcatRecordingError::Failed("core task failed".into())))
    }

    async fn discard_logcat_recording(
        self,
        _ctx: Context,
        session: LogcatSessionId,
    ) -> Result<(), LogcatRecordingError> {
        let state = self.state.clone();
        tokio::task::spawn_blocking(move || state.discard_logcat_recording(session))
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

/// The recorder's view of this core's config for the debug-logging lease.
struct CoreDebugControl(std::sync::Weak<CoreState>);

impl crate::logcat_session::DebugControl for CoreDebugControl {
    fn acquire(
        &self,
        persist: &dyn Fn(&LogcatDebugLease) -> std::io::Result<()>,
    ) -> Option<LogcatDebugLease> {
        let state = self.0.upgrade()?;
        let (previous_debug, previous_debug_switch) = state.current_debug()?;
        let mut lease = LogcatDebugLease {
            previous_debug,
            previous_debug_switch,
            restore: LogcatDebugRestore::Pending,
        };
        persist(&lease).ok()?;
        if state
            .acquire_debug_lease(DebugLeaseId(u64::MAX), DebugLeaseOwner::Logcat)
            .is_err()
        {
            let _ = state.release_debug_lease(DebugLeaseId(u64::MAX));
            return None;
        }
        lease.restore = LogcatDebugRestore::Held;
        Some(lease)
    }
    fn release(&self, lease: &LogcatDebugLease) -> LogcatDebugRestore {
        let Some(state) = self.0.upgrade() else {
            return LogcatDebugRestore::Pending;
        };
        if state
            .release_recording_debug_lease(DebugLeaseId(u64::MAX))
            .is_err()
        {
            return LogcatDebugRestore::Pending;
        }
        if state
            .current_debug()
            .is_some_and(|(_, switch)| switch != lease.previous_debug_switch)
        {
            LogcatDebugRestore::UserChoice
        } else {
            LogcatDebugRestore::Restored
        }
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

    impl CoreServer {
        // Most legacy orchestration tests assert completed effects. Keep that
        // explicit here; real-RPC tests separately assert early admission.
        async fn submit_test_command(
            self,
            ctx: Context,
            op: OperationId,
            command: CoreCommand,
        ) -> Result<Accepted, SubmitError> {
            let state = self.state.clone();
            let accepted = self.submit_command(ctx, op, command).await?;
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    if state
                        .get_operation_record(op)
                        .is_some_and(|record| record.settled.is_some())
                    {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            })
            .await
            .expect("terminal receipt");
            Ok(accepted)
        }

        async fn submit_test_edit(
            self,
            ctx: Context,
            op: OperationId,
            base: BaseRef,
            edit: ConfigEdit,
        ) -> Result<Accepted, SubmitError> {
            while self
                .state
                .engine
                .store
                .lock()
                .unwrap()
                .journal
                .issued_through
                < op.0
            {
                self.clone().allocate_operation_id(ctx).await?;
            }
            self.submit_test_command(ctx, op, CoreCommand::Edit { base, edit })
                .await
        }
    }

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
            .submit_test_edit(
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
                .submit_test_edit(ctx(), OperationId(2), BaseRef { revision: 0 }, bootstrap)
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
                .submit_test_edit(
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
            .submit_test_edit(
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
            .submit_test_edit(
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
            .submit_test_edit(
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
                .submit_test_edit(
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
            .submit_test_edit(
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
            .submit_test_edit(
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
    async fn privileged_commands_are_refused_unless_paths_are_enabled() {
        let server = server();
        let op = server.state.issue_operation_id().unwrap();
        let result = server
            .submit_test_command(
                tarpc::context::current(),
                op,
                CoreCommand::ClearSuperkey {
                    base: BaseRef { revision: 0 },
                },
            )
            .await;
        assert!(matches!(result, Err(SubmitError::Rejected { .. })));
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
            .submit_test_edit(
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
        server
            .clone()
            .submit_test_command(
                ctx(),
                state.issue_operation_id().unwrap(),
                CoreCommand::SetSuperkey {
                    base: BaseRef {
                        revision: state.config_revision(),
                    },
                    remember: true,
                    superkey: "k".into(),
                },
            )
            .await
            .unwrap();
        assert!(fs_root.join("data/adb/vpnhide/superkey").exists());
        let before = state.config_revision();

        server
            .clone()
            .submit_test_command(
                ctx(),
                state.issue_operation_id().unwrap(),
                CoreCommand::FullReset,
            )
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
            .submit_test_edit(
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
            .submit_test_edit(
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
            .submit_test_edit(
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
            .submit_test_edit(
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
            .submit_test_edit(
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
            .submit_test_edit(ctx(), OperationId(1), BaseRef { revision: 0 }, replace)
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
                .submit_test_edit(
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
            .submit_test_edit(
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
                .submit_test_edit(ctx(), OperationId(10), BaseRef { revision: 0 }, replace)
                .await
                .expect("deduped")
                .op,
            OperationId(10),
        );
        // Same id + different content → rejected.
        let reused = server2
            .submit_test_edit(
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
            admission: None,
            command: None,
            superseded_by: None,
            superseded_phases: Vec::new(),
            finalized_unknown: false,
            native_intent: None,
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
            .submit_test_edit(
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
            .submit_test_edit(
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
            .submit_test_edit(
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
