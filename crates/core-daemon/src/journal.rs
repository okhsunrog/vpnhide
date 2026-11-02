//! Private durable command journal.
//!
//! Separate from the LSPosed-facing canonical config file. This store preserves
//! operation identity, original requests, phase progress, outcomes, and the
//! config-revision counter across daemon restarts so recovery can fence
//! unresolved work instead of inventing completion or replaying effects.
//!
//! # Durability rule
//!
//! Evidence is synced to disk **before** an external effect is dispatched, and
//! again after each phase settles. A crash between intent and completion leaves
//! a `Running` or `Unknown` record that restart recovery turns into durable
//! uncertainty — never a blind re-dispatch.
//!
//! # Retention
//!
//! All terminal receipts are capped (`retain_settled`), with one pinned debug
//! restoration receipt. Expired Unknown receipts retain a bounded resource hold
//! and at most one canonical admission proof, without retaining old requests.
//! Core issues monotonic identities; pruning advances a durable expiry floor,
//! avoiding permanent random-ID tombstones. Explicit repair remains admissible
//! at the retention bound; retained receipts are checked before expiry.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use vpnhide_core_rpc::{
    BaseRef, CanonicalConfig, ConfigEdit, ConfigPhase, NativeBackend, OperationId, OperationRecord,
    PhaseOutcome, PhaseRecord,
};

/// Production path of the private journal (root-owned, not LSPosed-readable by design).
pub const DEFAULT_JOURNAL_PATH: &str = "/data/adb/vpnhide/core/operations.json";

/// Env override so the dev sandbox never touches the production journal.
pub const JOURNAL_PATH_ENV: &str = "VPNHIDE_JOURNAL_PATH";

pub mod preflight;
pub mod revocation;

const JOURNAL_VERSION: u32 = vpnhide_build_contract::JOURNAL_VERSION;

/// Default number of terminal receipts to keep, including Unknown.
pub const DEFAULT_RETAIN_SETTLED: usize = 64;

/// Resolve the journal path from the environment.
///
/// Priority: `VPNHIDE_JOURNAL_PATH` → beside `VPNHIDE_CONFIG_PATH` (dev sandbox /
/// host tests) → production default. Colocating with an isolated config keeps the
/// sandbox from writing `/data/adb/...` while production still uses the private path.
pub fn journal_path_from_env() -> PathBuf {
    match std::env::var(JOURNAL_PATH_ENV) {
        Ok(p) if !p.is_empty() => return PathBuf::from(p),
        _ => {}
    }
    match std::env::var(crate::config_file::CONFIG_PATH_ENV) {
        Ok(config) if !config.is_empty() => {
            PathBuf::from(config).with_file_name("core_operations.json")
        }
        _ => PathBuf::from(DEFAULT_JOURNAL_PATH),
    }
}

/// One accepted operation as stored for dedupe and recovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    #[serde(default)]
    pub maintenance: Option<crate::maintenance::MaintenancePlan>,
    #[serde(default)]
    pub maintenance_report: Option<crate::maintenance::MaintenanceReport>,
    /// Actual admission revisions and write set, independent of a stale client base.
    #[serde(default)]
    pub admission: Option<Admission>,
    /// Non-secret command identity. Secret payloads are deliberately never durable.
    #[serde(default)]
    pub command: Option<CommandIntent>,
    /// A later explicit desired-state repair replaced this entry's held resources.
    /// The original Unknown receipt is retained unchanged.
    #[serde(default)]
    pub superseded_by: Option<OperationId>,
    #[serde(default)]
    pub superseded_phases: Vec<ConfigPhase>,
    /// Once uncertainty was returned/recovered as terminal, later desired-state
    /// writes cannot retroactively turn that historical receipt into success/failure.
    #[serde(default)]
    pub finalized_unknown: bool,
    pub base: BaseRef,
    pub edit: ConfigEdit,
    /// Intended config after this edit — required for Persist/Native evidence readback.
    /// Absent only on journals written before this field existed; those stay Unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<CanonicalConfig>,
    /// Exact delivery intent, synced before Native dispatch. Older entries have no
    /// trustworthy Native evidence and remain Unknown after an interrupted write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_intent: Option<NativeIntent>,
    pub record: OperationRecord,
}

impl JournalEntry {
    pub(crate) fn is_revocation_only_repair(&self) -> bool {
        self.command == Some(CommandIntent::Edit)
            && self.edit == ConfigEdit::RepairActivation
            && self.candidate.is_none()
            && self.native_intent.is_none()
            && self.record.phases.len() == 1
            && self.record.phases[0].phase == ConfigPhase::Native
            && self
                .admission
                .as_ref()
                .is_some_and(|a| a.from_revision == a.commit_revision && a.writes.is_empty())
    }

    pub fn held_phases(&self) -> Vec<ConfigPhase> {
        if self.superseded_by.is_some() || self.record.settled != Some(PhaseOutcome::Unknown) {
            return Vec::new();
        }
        let unresolved: Vec<_> = self
            .record
            .phases
            .iter()
            .filter(|phase| {
                matches!(
                    phase.outcome,
                    PhaseOutcome::Unknown | PhaseOutcome::Running | PhaseOutcome::NotAttempted
                )
            })
            .map(|phase| phase.phase)
            .collect();
        let phases = if unresolved.is_empty() {
            self.record.phases.iter().map(|phase| phase.phase).collect()
        } else {
            unresolved
        };
        phases
            .into_iter()
            .filter(|phase| !self.superseded_phases.contains(phase))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Admission {
    pub from_revision: u64,
    pub commit_revision: u64,
    pub writes: Vec<String>,
}

/// Bounded proof for the most recent expired uncertain canonical publication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingPersist {
    pub admission: Option<Admission>,
    pub candidate: Option<CanonicalConfig>,
    pub reset: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandIntent {
    Edit,
    SetSuperkey {
        remember: bool,
    },
    ClearSuperkey,
    Import {
        cleanup_legacy: bool,
    },
    MigrateLegacy {
        mode: vpnhide_core_rpc::LegacyImportMode,
    },
    CleanupLegacy,
    FullReset,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZygiskAuthorizationIntent {
    pub boot_id: String,
    pub generation: u64,
    pub directory_identity: (u64, u64),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeIntent {
    pub backend: NativeBackend,
    pub wire: String,
    /// The actual recipient selected for this operation, never the current env default.
    pub zygisk_module: Option<PathBuf>,
    #[serde(default)]
    pub zygisk_authorization: Option<ZygiskAuthorizationIntent>,
}

/// Independent of receipt-ring retention. This is the latest admitted authority,
/// not evidence that publication, consumption or hook installation succeeded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZygiskAuthorityRecord {
    pub operation: OperationId,
    pub intent: NativeIntent,
}
impl ZygiskAuthorityRecord {
    pub fn reference(&self) -> Option<vpnhide_core_rpc::ZygiskAuthorizationRef> {
        use sha2::{Digest, Sha256};
        let a = self.intent.zygisk_authorization.as_ref()?;
        let envelope = vpnhide_protocol::zygisk_activation::Authorization {
            boot_id: a.boot_id.clone(),
            generation: a.generation,
            wire: self.intent.wire.clone(),
        };
        if self.intent.backend != NativeBackend::Zygisk
            || !self
                .intent
                .zygisk_module
                .as_ref()
                .is_some_and(|p| p.is_absolute())
            || a.directory_identity.1 == 0
            || self.operation.0 != a.generation
            || envelope.encode().is_none()
        {
            return None;
        }
        Some(vpnhide_core_rpc::ZygiskAuthorizationRef {
            operation: self.operation.0,
            boot_id: a.boot_id.clone(),
            generation: a.generation,
            policy_sha256: format!("{:x}", Sha256::digest(self.intent.wire.as_bytes())),
            directory_device: a.directory_identity.0,
            directory_inode: a.directory_identity.1,
        })
    }
}

/// On-disk journal document.
/// Initial activation is not an app command, but has the same before-effect
/// durability requirement. Missing completion means uncertainty, across worker
/// restarts AND reboots. Never infer completion from native_boot.json.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeStartup {
    pub boot_id: String,
    pub config_revision: u64,
    pub filesystem_hiding: bool,
    #[serde(default)]
    pub intent: Option<crate::native_boot::prepared::TransitionIntent>,
    #[serde(default)]
    pub repair_op: Option<OperationId>,
    pub completed: Option<NativeStartupResult>,
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

impl NativeStartup {
    fn valid(&self) -> bool {
        use crate::native_boot::prepared::TransitionAction;
        if self.boot_id.is_empty()
            || self.boot_id.len() > 128
            || self
                .completed
                .as_ref()
                .is_some_and(|r| r.status.boot_id != self.boot_id)
        {
            return false;
        }
        let Some(intent) = &self.intent else {
            return self
                .completed
                .as_ref()
                .is_none_or(|r| r.rejection.is_none());
        }; // schema 3
        if intent.boot_id != self.boot_id
            || intent.filesystem_hiding != self.filesystem_hiding
            || (intent.retire_directory_identity.is_some() && intent.retire_zygisk.is_none())
        {
            return false;
        }
        if let Some(bundle) = &intent.bundle
            && (bundle.root.as_os_str().is_empty()
                || bundle.root_identity.is_some_and(|(_, inode)| inode == 0)
                || !valid_sha256(&bundle.manifest_sha256)
                || bundle.source_commit.len() != 40
                || !bundle.source_commit.bytes().all(|b| b.is_ascii_hexdigit())
                || bundle.wire.len() > vpnhide_protocol::zygisk_activation::MAX_BYTES
                || vpnhide_protocol::parse_config(bundle.wire.as_bytes()).is_none()
                || matches!(intent.action, TransitionAction::Observe))
        {
            return false;
        }
        if self
            .completed
            .as_ref()
            .and_then(|r| r.rejection.as_ref())
            .is_some_and(|proof| !proof.valid_for(self))
        {
            return false;
        }
        if let Some(payload) = &intent.payload
            && (!matches!(
                intent.action,
                TransitionAction::Load(NativeBackend::Kmod | NativeBackend::Kpm)
            ) || payload.bytes == 0
                || payload.bytes > crate::payloads::MAX_PAYLOAD_BYTES
                || !valid_sha256(&payload.sha256)
                || payload.init_contract.is_some_and(|contract| {
                    contract != 1
                        || intent.bundle.is_none()
                        || intent.action != TransitionAction::Load(NativeBackend::Kmod)
                })
                || payload.source.as_os_str().is_empty())
        {
            return false;
        }
        !matches!(
            intent.action,
            TransitionAction::Load(NativeBackend::Builtin | NativeBackend::Zygisk)
        )
    }

    pub fn uncertain(&self) -> bool {
        match &self.completed {
            None => true,
            Some(result) => {
                matches!(
                    result.status.outcome,
                    vpnhide_core_rpc::NativeBootOutcome::LoadFailed { .. }
                ) && !result
                    .rejection
                    .as_ref()
                    .is_some_and(|proof| proof.valid_for(self))
                    || matches!(
                        result.status.zygisk_targets,
                        vpnhide_core_rpc::ZygiskTargets::RetireFailed { .. }
                    )
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeStartupResult {
    pub status: vpnhide_core_rpc::NativeBootStatus,
    pub selection: Option<crate::native_boot::Selection>,
    #[serde(default)]
    pub rejection: Option<crate::native_boot::evidence::RejectionProof>,
}

/// Bounded audit of superseded startup attempts. An unresolved attempt stays in
/// native_startup until explicit repair establishes a safe replacement boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeAttemptHistory {
    pub attempt: NativeStartup,
    pub superseded_by: Option<OperationId>,
}
pub const MAX_NATIVE_ATTEMPTS: usize = 16;
const MAX_ZYGISK_EXPOSURE_BOOTS: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeLoadBudget {
    pub boot_id: String,
    pub attempted: Vec<vpnhide_core_rpc::NativeBackend>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeQuarantine {
    pub backend: vpnhide_core_rpc::NativeBackend,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    #[serde(default)]
    pub zygisk_authority: Option<ZygiskAuthorityRecord>,
    /// Last exact grant observed on disk before admitting a replacement. Failed
    /// publication cannot erase the evidence required to revoke the old grant.
    #[serde(default)]
    pub prior_zygisk_authority: Option<ZygiskAuthorityRecord>,
    #[serde(default)]
    pub zygisk_revocation: Option<revocation::ZygiskRevocation>,
    pub version: u32,
    #[serde(default)]
    pub kmod_preflights: Option<preflight::PreflightLedger>,
    #[serde(default)]
    pub native_startup: Option<NativeStartup>,
    #[serde(default)]
    pub native_attempts: Vec<NativeAttemptHistory>,
    #[serde(default)]
    pub native_load_budget: Option<NativeLoadBudget>,
    #[serde(default)]
    pub native_quarantine: Vec<NativeQuarantine>,
    /// Future-process authorization may have been consumed in this boot.
    /// Receipt pruning and revocation cannot prove that those processes exited.
    #[serde(default)]
    pub zygisk_exposure_boot: Option<String>,
    /// Migration can discover multiple boot identities in retained evidence.
    /// Never let an older single marker erase a later possible exposure.
    #[serde(default)]
    pub zygisk_exposure_boots: Vec<String>,
    /// Durable config-revision counter — continues across restarts.
    pub config_revision: u64,
    pub retain_settled: usize,
    pub operations: Vec<JournalEntry>,
    /// Legacy migration only. New journals keep this empty and advance expired_through.
    pub retired_ids: Vec<OperationId>,
    #[serde(default)]
    pub issued_through: u64,
    #[serde(default)]
    pub expired_through: u64,
    /// Older editor bases cannot be conflict-checked after bounded history compaction.
    #[serde(default)]
    pub conflict_floor: u64,
    #[serde(default)]
    pub debug_leases: Vec<vpnhide_core_rpc::DebugLease>,
    #[serde(default)]
    pub debug_restore: Option<(bool, bool)>,
    #[serde(default)]
    pub debug_restore_op: Option<OperationId>,
    /// Resource uncertainty outlives receipt expiry without retaining old requests.
    #[serde(default)]
    pub expired_holds: Vec<ConfigPhase>,
    #[serde(default)]
    pub pending_persist: Option<PendingPersist>,
    /// Last config_revision that committed each field path (field-level conflicts).
    #[serde(default)]
    pub field_revisions: std::collections::BTreeMap<String, u64>,
}

impl Default for Journal {
    fn default() -> Self {
        Journal {
            version: JOURNAL_VERSION,
            zygisk_authority: None,
            prior_zygisk_authority: None,
            zygisk_revocation: None,
            kmod_preflights: None,
            native_startup: None,
            native_attempts: Vec::new(),
            native_load_budget: None,
            native_quarantine: Vec::new(),
            zygisk_exposure_boot: None,
            zygisk_exposure_boots: vec![],
            config_revision: 0,
            retain_settled: DEFAULT_RETAIN_SETTLED,
            operations: Vec::new(),
            retired_ids: Vec::new(),
            issued_through: 0,
            expired_through: 0,
            conflict_floor: 0,
            debug_leases: Vec::new(),
            debug_restore: None,
            debug_restore_op: None,
            expired_holds: Vec::new(),
            pending_persist: None,
            field_revisions: std::collections::BTreeMap::new(),
        }
    }
}

impl Journal {
    pub(crate) fn note_zygisk_exposure(&mut self, boot: &str) {
        self.zygisk_exposure_boot = Some(boot.into());
        if !self.zygisk_exposure_boots.iter().any(|b| b == boot) {
            self.zygisk_exposure_boots.push(boot.into());
            let extra = self
                .zygisk_exposure_boots
                .len()
                .saturating_sub(MAX_ZYGISK_EXPOSURE_BOOTS);
            self.zygisk_exposure_boots.drain(..extra);
        }
    }
    pub(crate) fn zygisk_exposed_in(&self, boot: &str) -> bool {
        self.zygisk_exposure_boot.as_deref() == Some(boot)
            || self.zygisk_exposure_boots.iter().any(|b| b == boot)
    }
    /// Lookup by operation id.
    pub fn get(&self, op: OperationId) -> Option<&JournalEntry> {
        self.operations.iter().find(|e| e.record.op == op)
    }

    pub fn get_mut(&mut self, op: OperationId) -> Option<&mut JournalEntry> {
        self.operations.iter_mut().find(|e| e.record.op == op)
    }

    pub fn is_retired(&self, op: OperationId) -> bool {
        op.0 <= self.expired_through || self.retired_ids.contains(&op)
    }

    /// True when any stored operation still carries unresolved uncertainty.
    pub fn has_unknown(&self) -> bool {
        self.native_lifecycle_uncertain()
            || !self.expired_holds.is_empty()
            || self
                .operations
                .iter()
                .any(|entry| !entry.held_phases().is_empty())
    }

    pub fn native_startup_uncertain(&self) -> bool {
        self.native_startup
            .as_ref()
            .is_some_and(NativeStartup::uncertain)
    }

    pub(crate) fn check_native_safety(
        &self,
        intent: &crate::native_boot::prepared::TransitionIntent,
    ) -> Result<(), String> {
        use crate::native_boot::prepared::TransitionAction;
        if self.revocation_uncertain() {
            return Err("Zygisk authorization revocation is unresolved; reconcile it before native activation".into());
        }
        if self.preflight_uncertain() {
            return Err(
                "loader preflight outcome is unknown; native activation remains held".into(),
            );
        }
        if matches!(intent.action, TransitionAction::Load(NativeBackend::Kmod))
            && let Some(ledger) = &self.kmod_preflights
            && ledger.plan.boot_id == intent.boot_id
        {
            match ledger.evidence() {
                preflight::LoaderEvidence::OneSentinel { sha256 }
                    if intent.payload.as_ref().is_some_and(|p| p.sha256 == sha256) => {}
                _ => return Err(
                    "current preflight evidence does not identify one matching real-load payload"
                        .into(),
                ),
            }
        }
        if matches!(intent.action, TransitionAction::Load(b) | TransitionAction::Adopt(b) if b != NativeBackend::Zygisk)
            && (self.zygisk_exposed_in(&intent.boot_id) || intent.retire_zygisk.is_some())
        {
            return Err("Zygisk authorization may have been consumed in this boot; reboot before kernel backend activation".into());
        }
        Ok(())
    }
    pub fn check_native_attempt(
        &self,
        intent: &crate::native_boot::prepared::TransitionIntent,
        explicit: bool,
    ) -> Result<(), String> {
        use crate::native_boot::prepared::TransitionAction;
        self.check_native_safety(intent)?;
        if let TransitionAction::Load(backend) = intent.action {
            if intent.payload.is_none() {
                return Ok(());
            }
            if self.native_load_budget.as_ref().is_some_and(|budget| {
                budget.boot_id == intent.boot_id && budget.attempted.contains(&backend)
            }) {
                return Err(
                    "this backend was already attempted in this boot; reboot before retrying"
                        .into(),
                );
            }
            if !explicit
                && intent.payload.as_ref().is_some_and(|payload| {
                    self.native_quarantine
                        .iter()
                        .any(|q| q.backend == backend && q.sha256 == payload.sha256)
                })
            {
                return Err("previous payload activation was not confirmed; explicit RepairActivation is required".into());
            }
        }
        Ok(())
    }

    /// A consumed budget is never refunded. Its candidate can only be skipped
    /// when every retained matching attempt proves no effect in this same boot.
    pub(crate) fn backend_rejected(&self, boot: &str, backend: NativeBackend) -> bool {
        use crate::native_boot::prepared::TransitionAction;
        if !self
            .native_load_budget
            .as_ref()
            .is_some_and(|b| b.boot_id == boot && b.attempted.contains(&backend))
        {
            return false;
        }
        let mut count = 0;
        for attempt in self
            .native_attempts
            .iter()
            .map(|h| &h.attempt)
            .chain(self.native_startup.iter())
            .filter(|a| {
                a.boot_id == boot
                    && a.intent.as_ref().is_some_and(|i| {
                        i.action == TransitionAction::Load(backend) && i.payload.is_some()
                    })
            })
        {
            count += 1;
            if !attempt
                .completed
                .as_ref()
                .and_then(|r| r.rejection.as_ref())
                .is_some_and(|p| p.valid_for(attempt))
            {
                return false;
            }
        }
        count > 0
    }

    pub fn complete_native_startup(&mut self, result: NativeStartupResult) {
        use crate::native_boot::prepared::TransitionAction;
        if let Some(attempt) = &mut self.native_startup {
            if let Some(intent) = &attempt.intent
                && let TransitionAction::Load(backend) = intent.action
                && result
                    .selection
                    .is_some_and(|selected| selected.backend == backend)
            {
                self.native_quarantine.retain(|q| q.backend != backend);
            }
            attempt.completed = Some(result);
        }
    }

    pub fn replace_native_startup(&mut self, attempt: NativeStartup) {
        if attempt
            .intent
            .as_ref()
            .is_some_and(|intent| intent.possible_zygisk_exposure())
        {
            self.note_zygisk_exposure(&attempt.boot_id);
        }
        if let Some(intent) = &attempt.intent
            && let (crate::native_boot::prepared::TransitionAction::Load(backend), Some(payload)) =
                (intent.action, &intent.payload)
        {
            if self
                .native_load_budget
                .as_ref()
                .is_none_or(|budget| budget.boot_id != intent.boot_id)
            {
                self.native_load_budget = Some(NativeLoadBudget {
                    boot_id: intent.boot_id.clone(),
                    attempted: Vec::new(),
                });
            }
            let budget = self.native_load_budget.as_mut().expect("load budget");
            if !budget.attempted.contains(&backend) {
                budget.attempted.push(backend);
            }
            self.native_quarantine.retain(|q| q.backend != backend);
            self.native_quarantine.push(NativeQuarantine {
                backend,
                sha256: payload.sha256.clone(),
            });
        }

        if let Some(previous) = self.native_startup.replace(attempt.clone()) {
            self.native_attempts.push(NativeAttemptHistory {
                attempt: previous,
                superseded_by: attempt.repair_op,
            });
            let overflow = self
                .native_attempts
                .len()
                .saturating_sub(MAX_NATIVE_ATTEMPTS);
            self.native_attempts.drain(..overflow);
        }
    }

    /// All durable resource holds, including startup work without an app receipt.
    pub fn held_resources(&self) -> Vec<ConfigPhase> {
        let mut held: Vec<_> = self
            .expired_holds
            .iter()
            .copied()
            .chain(self.operations.iter().flat_map(JournalEntry::held_phases))
            .collect();
        if self.native_lifecycle_uncertain() {
            held.push(ConfigPhase::Native);
        }
        held.sort_by_key(|phase| *phase as u8);
        held.dedup();
        held
    }

    /// Insert or replace one entry, then apply retention.
    pub fn upsert(&mut self, entry: JournalEntry) {
        let op = entry.record.op;
        self.issued_through = self.issued_through.max(op.0);
        if let Some(slot) = self.operations.iter_mut().find(|e| e.record.op == op) {
            *slot = entry;
        } else {
            self.operations.push(entry);
        }
        self.prune();
    }

    /// Drop old settled/superseded receipts and advance bounded identity/conflict floors.
    pub fn prune(&mut self) {
        let retain = self.retain_settled;
        let revocation_op = self
            .zygisk_revocation
            .as_ref()
            .filter(|r| r.uncertain())
            .and_then(|r| r.operation);
        let settled_idxs: Vec<usize> = self
            .operations
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.record.settled.is_some()
                    && Some(e.record.op) != self.debug_restore_op
                    && !(Some(e.record.op) == revocation_op && e.is_revocation_only_repair())
            })
            .map(|(i, _)| i)
            .collect();
        if settled_idxs.len() <= retain {
            return;
        }
        let drop_count = settled_idxs.len() - retain;
        let drop_set: BTreeSet<usize> = settled_idxs.into_iter().take(drop_count).collect();
        let mut kept = Vec::with_capacity(self.operations.len() - drop_count);
        for (i, entry) in self.operations.drain(..).enumerate() {
            if drop_set.contains(&i) {
                let held = entry.held_phases();
                if held.contains(&ConfigPhase::Persist) {
                    self.pending_persist = Some(PendingPersist {
                        admission: entry.admission.clone(),
                        candidate: entry.candidate.clone(),
                        reset: entry.command == Some(CommandIntent::FullReset),
                    });
                }
                for phase in held {
                    if !self.expired_holds.contains(&phase) {
                        self.expired_holds.push(phase);
                    }
                }
                self.expired_through = self.expired_through.max(entry.record.op.0);
                if let Some(admission) = entry.admission {
                    self.conflict_floor = self
                        .conflict_floor
                        .max(admission.commit_revision.min(self.config_revision));
                }
            } else {
                kept.push(entry);
            }
        }
        self.operations = kept;
        self.field_revisions
            .retain(|_, revision| *revision > self.conflict_floor);
    }

    /// Restart fencing: any in-flight (`Running`) phase becomes durable `Unknown`.
    /// An accepted but unfinished operation (`settled == None`) is also held as
    /// `Unknown` so remaining `NotAttempted` phases are **not** continued blindly.
    /// Effects are never re-dispatched here; [`crate::recovery`] may resolve Unknown
    /// from durable evidence afterward.
    pub fn fence_unresolved(&mut self) {
        if let Some(record) = &self.zygisk_revocation
            && record.state == revocation::RevocationState::Confirmed
            && let Some(operation) = record.operation
            && let Some(entry) = self.get_mut(operation)
            && entry.is_revocation_only_repair()
            && !entry.finalized_unknown
            && entry.record.settled.is_none()
        {
            // The exact subeffect completion is already durable. This does not
            // rewrite a returned/recovered Unknown or infer a policy publication.
            entry.record.phases[0].outcome = PhaseOutcome::Confirmed;
            entry.record.settled = Some(PhaseOutcome::Confirmed);
        }
        if let Some(record) = self.zygisk_revocation.as_mut()
            && record.state == revocation::RevocationState::Running
        {
            record.state = revocation::RevocationState::Unknown;
        }
        if let Some(ledger) = &mut self.kmod_preflights {
            ledger.fence_unresolved();
        }
        for entry in &mut self.operations {
            if entry.record.settled == Some(PhaseOutcome::Unknown) {
                entry.finalized_unknown = true;
            }
            let mut uncertain = entry.record.settled == Some(PhaseOutcome::Unknown)
                || entry.record.settled.is_none();
            for phase in &mut entry.record.phases {
                if phase.outcome == PhaseOutcome::Running {
                    phase.outcome = PhaseOutcome::Unknown;
                    uncertain = true;
                }
            }
            if uncertain {
                entry.record.settled = Some(PhaseOutcome::Unknown);
            }
        }
    }

    /// Build the in-memory operation map restored after fencing.
    pub fn into_store_map(self) -> HashMap<OperationId, (BaseRef, ConfigEdit, OperationRecord)> {
        self.operations
            .into_iter()
            .map(|e| (e.record.op, (e.base, e.edit, e.record)))
            .collect()
    }
}

/// Read the journal. Missing means first installation; a present blank/corrupt
/// journal is unavailable, so operation identities are never silently reset.
pub fn read(path: &Path) -> io::Result<Journal> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Journal::default()),
        Err(e) => return Err(e),
    };
    if raw.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "empty operation journal",
        ));
    }
    let journal: Journal = serde_json::from_str(&raw)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("journal parse: {e}")))?;
    if journal.version == 0 || journal.version > JOURNAL_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "journal version {} unsupported (maximum {})",
                journal.version, JOURNAL_VERSION
            ),
        ));
    }
    let invalid_startup = journal
        .native_startup
        .iter()
        .chain(journal.native_attempts.iter().map(|item| &item.attempt))
        .any(|attempt| !attempt.valid());
    if journal
        .zygisk_authority
        .iter()
        .chain(journal.prior_zygisk_authority.iter())
        .any(|a| {
            a.reference().is_none()
                || (journal.version >= 9 && a.operation.0 > journal.issued_through)
        })
        || journal
            .zygisk_revocation
            .as_ref()
            .is_some_and(|r| !r.valid(journal.issued_through))
        || invalid_startup
        || journal.kmod_preflights.as_ref().is_some_and(|l| !l.valid())
        || journal.operations.iter().any(|entry| {
            entry
                .maintenance
                .as_ref()
                .is_some_and(|p| !p.valid() || p.operation != entry.record.op)
        })
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid native startup identity",
        ));
    }
    if journal
        .zygisk_exposure_boot
        .as_ref()
        .is_some_and(|boot| boot.is_empty() || boot.len() > 128)
        || journal.zygisk_exposure_boots.len() > MAX_ZYGISK_EXPOSURE_BOOTS
        || journal
            .zygisk_exposure_boots
            .iter()
            .any(|b| b.is_empty() || b.len() > 128)
        || journal.native_attempts.len() > MAX_NATIVE_ATTEMPTS
        || journal.native_quarantine.len() > 4
        || journal
            .native_quarantine
            .iter()
            .any(|q| !valid_sha256(&q.sha256))
        || journal
            .native_load_budget
            .as_ref()
            .is_some_and(|b| b.attempted.len() > 4 || b.boot_id.is_empty() || b.boot_id.len() > 128)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "native lifecycle budget exceeded",
        ));
    }
    for entry in &journal.operations {
        if let Some(intent) = &entry.native_intent
            && let Some(authorization) = &intent.zygisk_authorization
        {
            let envelope = vpnhide_protocol::zygisk_activation::Authorization {
                boot_id: authorization.boot_id.clone(),
                generation: authorization.generation,
                wire: intent.wire.clone(),
            };
            if intent.backend != NativeBackend::Zygisk
                || intent.zygisk_module.is_none()
                || authorization.generation != entry.record.op.0
                || envelope.encode().is_none()
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid journaled Zygisk authorization",
                ));
            }
        }
    }
    let mut journal = journal;
    if journal.zygisk_authority.is_none() {
        journal.zygisk_authority = journal
            .operations
            .iter()
            .filter(|e| {
                e.record.phases.iter().any(|p| {
                    p.phase == ConfigPhase::Native && p.outcome != PhaseOutcome::NotAttempted
                })
            })
            .filter_map(|e| {
                e.native_intent.clone().map(|intent| ZygiskAuthorityRecord {
                    operation: e.record.op,
                    intent,
                })
            })
            .rfind(|a| a.reference().is_some());
    }
    // Older journals have no independent exposure marker. Preserve the retained
    // startup evidence before pruning; a restart is not a process-generation reset.
    let mut discovered = Vec::new();
    if let Some(boot) = journal.zygisk_exposure_boot.clone() {
        discovered.push(boot);
    }
    for attempt in journal
        .native_attempts
        .iter()
        .map(|i| &i.attempt)
        .chain(journal.native_startup.iter())
    {
        if attempt
            .intent
            .as_ref()
            .is_some_and(|i| i.possible_zygisk_exposure())
            || attempt
                .completed
                .as_ref()
                .and_then(|r| r.selection)
                .is_some_and(|s| s.backend == NativeBackend::Zygisk)
        {
            discovered.push(attempt.boot_id.clone());
        }
    }
    if let Some(a) = journal
        .zygisk_authority
        .as_ref()
        .and_then(|a| a.intent.zygisk_authorization.as_ref())
    {
        discovered.push(a.boot_id.clone());
    }
    for boot in discovered {
        if !journal.zygisk_exposure_boots.contains(&boot) {
            journal.zygisk_exposure_boots.push(boot);
        }
    }
    if journal.zygisk_exposure_boot.is_none() {
        journal.zygisk_exposure_boot = journal.zygisk_exposure_boots.last().cloned();
    }
    let extra = journal
        .zygisk_exposure_boots
        .len()
        .saturating_sub(MAX_ZYGISK_EXPOSURE_BOOTS);
    journal.zygisk_exposure_boots.drain(..extra);
    journal.conflict_floor = journal.conflict_floor.min(journal.config_revision);
    journal.version = JOURNAL_VERSION;
    journal.retain_settled = journal.retain_settled.min(DEFAULT_RETAIN_SETTLED);
    journal.expired_through = journal
        .retired_ids
        .iter()
        .map(|id| id.0)
        .max()
        .unwrap_or(0)
        .max(journal.expired_through);
    journal.issued_through = journal
        .operations
        .iter()
        .map(|e| e.record.op.0)
        .max()
        .unwrap_or(0)
        .max(journal.issued_through)
        .max(journal.expired_through);
    journal.retired_ids.clear();
    journal.prune();
    Ok(journal)
}

#[cfg(test)]
mod fault_thread_state {
    thread_local! {
        pub(super) static WRITE_FAULT: std::cell::Cell<Option<(usize, bool)>> = const { std::cell::Cell::new(None) };
    }
}
#[cfg(test)]
use fault_thread_state::WRITE_FAULT;

/// Writer-thread-local injection; never enabled in production builds.
#[cfg(test)]
pub(crate) fn fail_write(number: usize, after_rename: bool) {
    WRITE_FAULT.set(Some((number, after_rename)));
}

#[cfg(test)]
fn take_write_fault() -> Option<bool> {
    WRITE_FAULT.with(|fault| match fault.get() {
        Some((1, after)) => {
            fault.set(None);
            Some(after)
        }
        Some((n, after)) => {
            fault.set(Some((n - 1, after)));
            None
        }
        None => None,
    })
}

/// Serialize and write atomically (temp + fsync + rename), mode `0600`.
pub fn write_atomic(path: &Path, journal: &Journal) -> io::Result<()> {
    #[cfg(test)]
    let fault = take_write_fault();
    #[cfg(test)]
    if fault == Some(false) {
        return Err(io::Error::other("injected journal temp write failure"));
    }

    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut text = serde_json::to_string_pretty(journal)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    text.push('\n');

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&tmp)?;
        f.set_permissions(fs::Permissions::from_mode(0o600))?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    #[cfg(test)]
    if fault == Some(true) {
        // Exercise the same post-rename error wrapper as a failed directory sync.
        return crate::config_file::sync_after_rename(&dir.join("missing-fault-directory"));
    }
    crate::config_file::sync_after_rename(dir)
}

/// Record intent for `op` with `phase` marked `Running` (and prior phases as given), sync.
/// Call **before** dispatching that phase's external effect.
#[allow(clippy::too_many_arguments)]
pub fn record_phase_running(
    path: &Path,
    journal: &mut Journal,
    op: OperationId,
    base: BaseRef,
    edit: &ConfigEdit,
    candidate: &CanonicalConfig,
    plan: &[ConfigPhase],
    completed: &[(ConfigPhase, PhaseOutcome)],
    running: ConfigPhase,
    native_intent: Option<NativeIntent>,
) -> io::Result<()> {
    let mut phases = Vec::with_capacity(plan.len());
    for &phase in plan {
        let outcome = if phase == running {
            PhaseOutcome::Running
        } else if let Some((_, o)) = completed.iter().find(|(p, _)| *p == phase) {
            *o
        } else {
            PhaseOutcome::NotAttempted
        };
        phases.push(PhaseRecord { phase, outcome });
    }
    let native_intent = native_intent.or_else(|| {
        journal
            .get(op)
            .and_then(|entry| entry.native_intent.clone())
    });
    if let Some(authorization) = native_intent
        .as_ref()
        .and_then(|i| i.zygisk_authorization.as_ref())
    {
        journal.note_zygisk_exposure(&authorization.boot_id);
        if running == ConfigPhase::Native {
            if let Some(previous) = &journal.zygisk_authority
                && let Some(path) = &previous.intent.zygisk_module
                && let Ok(target) = crate::zygisk::ZygiskTarget::for_revocation(path)
                && let Some(reference) = previous.reference()
                && target.revalidate_identity()
                    == Ok((reference.directory_device, reference.directory_inode))
                && target
                    .read_authorized(&reference.boot_id)
                    .is_ok_and(|actual| {
                        actual.is_some_and(|a| {
                            a.generation == reference.generation && a.wire == previous.intent.wire
                        })
                    })
            {
                journal.prior_zygisk_authority = Some(previous.clone());
            }
            journal.zygisk_authority = Some(ZygiskAuthorityRecord {
                operation: op,
                intent: native_intent.clone().expect("authorization intent"),
            });
        }
    }
    let prior = journal.get(op).cloned();
    journal.upsert(JournalEntry {
        maintenance: prior.as_ref().and_then(|entry| entry.maintenance.clone()),
        maintenance_report: prior
            .as_ref()
            .and_then(|entry| entry.maintenance_report.clone()),
        admission: prior.as_ref().and_then(|entry| entry.admission.clone()),
        command: prior.as_ref().and_then(|entry| entry.command.clone()),
        superseded_by: prior.and_then(|entry| entry.superseded_by),
        superseded_phases: journal
            .get(op)
            .map(|entry| entry.superseded_phases.clone())
            .unwrap_or_default(),
        finalized_unknown: false,
        base,
        edit: edit.clone(),
        candidate: Some(candidate.clone()),
        native_intent,
        record: OperationRecord {
            op,
            phases,
            settled: None,
            ports: None,
        },
    });
    write_atomic(path, journal)
}

/// Persist the settled operation record (and optional revision bump) after phases finish.
#[allow(clippy::too_many_arguments)]
pub fn record_settled(
    path: &Path,
    journal: &mut Journal,
    op: OperationId,
    base: BaseRef,
    edit: &ConfigEdit,
    candidate: &CanonicalConfig,
    record: OperationRecord,
    config_revision: u64,
) -> io::Result<()> {
    journal.config_revision = config_revision;
    let native_intent = journal
        .get(op)
        .and_then(|entry| entry.native_intent.clone());
    let prior = journal.get(op).cloned();
    journal.upsert(JournalEntry {
        maintenance: prior.as_ref().and_then(|entry| entry.maintenance.clone()),
        maintenance_report: prior
            .as_ref()
            .and_then(|entry| entry.maintenance_report.clone()),
        admission: prior.as_ref().and_then(|entry| entry.admission.clone()),
        command: prior.as_ref().and_then(|entry| entry.command.clone()),
        superseded_by: prior.and_then(|entry| entry.superseded_by),
        superseded_phases: journal
            .get(op)
            .map(|entry| entry.superseded_phases.clone())
            .unwrap_or_default(),
        finalized_unknown: record.settled == Some(PhaseOutcome::Unknown),
        base,
        edit: edit.clone(),
        candidate: Some(candidate.clone()),
        native_intent,
        record,
    });
    write_atomic(path, journal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpnhide_core_rpc::{CanonicalConfig, ConfigToggle};

    #[test]
    fn startup_schema_upgrade_preserves_hold_and_rejects_corrupt_intent() {
        use crate::native_boot::prepared::{PayloadIdentity, TransitionAction, TransitionIntent};
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("journal.json");
        let startup = NativeStartup {
            boot_id: "boot".into(),
            config_revision: 9,
            filesystem_hiding: false,
            intent: None,
            repair_op: None,
            completed: None,
        };
        let mut journal = Journal {
            version: 3,
            native_startup: Some(startup),
            ..Journal::default()
        };
        write_atomic(&path, &journal).unwrap();
        let migrated = read(&path).unwrap();
        assert_eq!(migrated.version, 11);
        assert!(migrated.native_startup_uncertain());
        assert_eq!(migrated.native_startup, journal.native_startup);
        journal.version = 4;
        journal.native_startup.as_mut().unwrap().intent = Some(TransitionIntent {
            boot_id: "boot".into(),
            filesystem_hiding: false,
            action: TransitionAction::Load(NativeBackend::Kmod),
            bundle: None,
            payload: Some(PayloadIdentity {
                source: "/fixture/module.ko".into(),
                sha256: "a".repeat(64),
                bytes: 42,
                init_contract: None,
            }),
            retire_zygisk: None,
            retire_directory_identity: None,
        });
        write_atomic(&path, &journal).unwrap();
        assert!(read(&path).unwrap().native_startup_uncertain());
        let original = journal.clone();
        for change in 0..5 {
            journal = original.clone();
            let intent = journal
                .native_startup
                .as_mut()
                .unwrap()
                .intent
                .as_mut()
                .unwrap();
            match change {
                0 => intent.boot_id = "other".into(),
                1 => intent.filesystem_hiding = true,
                2 => intent.payload.as_mut().unwrap().sha256 = "bad".into(),
                3 => intent.payload.as_mut().unwrap().bytes = 0,
                _ => intent.action = TransitionAction::Observe,
            }
            write_atomic(&path, &journal).unwrap();
            let bytes = fs::read(&path).unwrap();
            assert!(read(&path).is_err());
            assert_eq!(
                fs::read(&path).unwrap(),
                bytes,
                "invalid source must be preserved"
            );
        }
        journal = original;
        journal.version = 12;
        write_atomic(&path, &journal).unwrap();
        assert!(read(&path).is_err(), "never reset unknown schema");
    }

    #[test]
    fn a_kernel_intent_cannot_introduce_its_own_unaccounted_zygisk_exposure() {
        use crate::native_boot::prepared::{TransitionAction, TransitionIntent};
        let journal = Journal::default();
        for action in [
            TransitionAction::Adopt(NativeBackend::Builtin),
            TransitionAction::Load(NativeBackend::Kmod),
            TransitionAction::Load(NativeBackend::Kpm),
        ] {
            let mut intent = TransitionIntent {
                boot_id: "boot".into(),
                filesystem_hiding: false,
                action,
                payload: None,
                bundle: None,
                retire_zygisk: None,
                retire_directory_identity: None,
            };
            assert!(journal.check_native_attempt(&intent, false).is_ok());
            intent.retire_zygisk = Some("/verified/legacy-zygisk".into());
            intent.retire_directory_identity = Some((1, 2));
            assert!(journal.check_native_attempt(&intent, false).is_err());
            assert!(journal.check_native_attempt(&intent, true).is_err());
            intent.action = TransitionAction::Observe;
            assert!(
                journal.check_native_attempt(&intent, true).is_ok(),
                "retirement itself remains possible"
            );
        }
        assert!(
            !journal.zygisk_exposed_in("boot"),
            "admission rejection dispatches no effect and invents no history"
        );
    }

    fn edit_debug(on: bool) -> ConfigEdit {
        ConfigEdit::Toggle {
            field: ConfigToggle::Debug,
            enabled: on,
        }
    }

    fn confirmed(op: u64) -> JournalEntry {
        JournalEntry {
            maintenance: None,
            maintenance_report: None,
            admission: None,
            command: None,
            superseded_by: None,
            superseded_phases: Vec::new(),
            finalized_unknown: false,
            native_intent: None,
            base: BaseRef { revision: 0 },
            edit: edit_debug(true),
            candidate: Some(CanonicalConfig {
                debug: true,
                ..Default::default()
            }),
            record: OperationRecord {
                op: OperationId(op),
                phases: vec![PhaseRecord {
                    phase: ConfigPhase::Persist,
                    outcome: PhaseOutcome::Confirmed,
                }],
                settled: Some(PhaseOutcome::Confirmed),
                ports: None,
            },
        }
    }

    #[test]
    fn authority_migration_uses_execution_order_instead_of_reserved_id_order() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("journal.json");
        let mut journal = Journal::default();
        for id in [8, 7] {
            let intent = NativeIntent {
                backend: NativeBackend::Zygisk,
                wire: "vpnhide 2 config\ndebug 0\ndefault 0\nend 0\n".into(),
                zygisk_module: Some(root.path().join("module")),
                zygisk_authorization: Some(ZygiskAuthorizationIntent {
                    boot_id: "12345678-1234-1234-1234-123456789abc".into(),
                    generation: id,
                    directory_identity: (1, 2),
                }),
            };
            record_phase_running(
                &path,
                &mut journal,
                OperationId(id),
                BaseRef { revision: 0 },
                &edit_debug(true),
                &CanonicalConfig::default(),
                &[ConfigPhase::Native],
                &[],
                ConfigPhase::Native,
                Some(intent),
            )
            .unwrap();
            record_settled(
                &path,
                &mut journal,
                OperationId(id),
                BaseRef { revision: 0 },
                &edit_debug(true),
                &CanonicalConfig::default(),
                OperationRecord {
                    op: OperationId(id),
                    phases: vec![PhaseRecord {
                        phase: ConfigPhase::Native,
                        outcome: PhaseOutcome::Confirmed,
                    }],
                    settled: Some(PhaseOutcome::Confirmed),
                    ports: None,
                },
                0,
            )
            .unwrap();
        }
        let mut old = serde_json::to_value(&journal).unwrap();
        old["version"] = 8.into();
        old.as_object_mut().unwrap().remove("zygisk_authority");
        fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        assert_eq!(
            read(&path).unwrap().zygisk_authority.unwrap().operation,
            OperationId(7)
        );
    }

    #[test]
    fn authority_survives_receipt_pruning_and_migrates_schema_eight_without_losing_exposure() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("journal.json");
        let intent = NativeIntent {
            backend: NativeBackend::Zygisk,
            wire: "vpnhide 2 config\ndebug 0\ndefault 0\nend 0\n".into(),
            zygisk_module: Some(root.path().join("module")),
            zygisk_authorization: Some(ZygiskAuthorizationIntent {
                boot_id: "12345678-1234-1234-1234-123456789abc".into(),
                generation: 7,
                directory_identity: (1, 2),
            }),
        };
        let mut journal = Journal::default();
        record_phase_running(
            &path,
            &mut journal,
            OperationId(7),
            BaseRef { revision: 0 },
            &edit_debug(true),
            &CanonicalConfig::default(),
            &[ConfigPhase::Native],
            &[],
            ConfigPhase::Native,
            Some(intent.clone()),
        )
        .unwrap();
        let authority = journal.zygisk_authority.clone().unwrap();
        assert!(authority.reference().is_some());
        assert_eq!(
            read(&path).unwrap().zygisk_authority,
            Some(authority.clone())
        );
        let mut old = serde_json::to_value(&journal).unwrap();
        old["version"] = 8.into();
        old.as_object_mut().unwrap().remove("zygisk_authority");
        fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        let upgraded = read(&path).unwrap();
        assert_eq!(upgraded.zygisk_authority, Some(authority.clone()));
        assert_eq!(upgraded.zygisk_exposure_boot, journal.zygisk_exposure_boot);
        journal.retain_settled = 0;
        record_settled(
            &path,
            &mut journal,
            OperationId(7),
            BaseRef { revision: 0 },
            &edit_debug(true),
            &CanonicalConfig::default(),
            OperationRecord {
                op: OperationId(7),
                phases: vec![PhaseRecord {
                    phase: ConfigPhase::Native,
                    outcome: PhaseOutcome::Confirmed,
                }],
                settled: Some(PhaseOutcome::Confirmed),
                ports: None,
            },
            0,
        )
        .unwrap();
        journal.prune();
        assert!(journal.operations.is_empty());
        write_atomic(&path, &journal).unwrap();
        assert_eq!(read(&path).unwrap().zygisk_authority, Some(authority));
        assert!(read(&path).unwrap().zygisk_exposure_boot.is_some());
        journal.zygisk_authority.as_mut().unwrap().operation = OperationId(8);
        journal
            .zygisk_authority
            .as_mut()
            .unwrap()
            .intent
            .zygisk_authorization
            .as_mut()
            .unwrap()
            .generation = 8;
        assert!(
            journal
                .zygisk_authority
                .as_ref()
                .unwrap()
                .reference()
                .is_some()
        );
        write_atomic(&path, &journal).unwrap();
        let bytes = fs::read(&path).unwrap();
        assert!(read(&path).is_err());
        assert_eq!(fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn authorization_generation_must_belong_to_its_durable_operation() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("journal.json");
        let mut journal = Journal::default();
        let mut entry = confirmed(7);
        entry.native_intent = Some(NativeIntent {
            backend: NativeBackend::Zygisk,
            wire: "vpnhide 2 config\ndebug 0\ndefault 0\nend 0\n".into(),
            zygisk_module: Some(root.path().join("module")),
            zygisk_authorization: Some(ZygiskAuthorizationIntent {
                boot_id: "12345678-1234-1234-1234-123456789abc".into(),
                generation: 7,
                directory_identity: (1, 2),
            }),
        });
        journal.upsert(entry);
        write_atomic(&path, &journal).unwrap();
        assert_eq!(read(&path).unwrap().version, 11);
        journal
            .get_mut(OperationId(7))
            .unwrap()
            .native_intent
            .as_mut()
            .unwrap()
            .zygisk_authorization
            .as_mut()
            .unwrap()
            .generation = 6;
        write_atomic(&path, &journal).unwrap();
        let before = fs::read(&path).unwrap();
        assert!(read(&path).is_err());
        assert_eq!(fs::read(path).unwrap(), before);
    }

    #[test]
    fn write_then_read_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("operations.json");
        let mut j = Journal {
            config_revision: 3,
            ..Journal::default()
        };
        j.upsert(confirmed(1));
        write_atomic(&path, &j).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(read(&path).unwrap(), j);
    }

    #[test]
    fn present_blank_journal_cannot_reset_issued_identities() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("operations.json");
        fs::write(&path, "\n").unwrap();
        assert_eq!(read(&path).unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn fence_turns_running_into_unknown_without_inventing_success() {
        let mut j = Journal::default();
        j.upsert(JournalEntry {
            maintenance: None,
            maintenance_report: None,
            admission: None,
            command: None,
            superseded_by: None,
            superseded_phases: Vec::new(),
            finalized_unknown: false,
            native_intent: None,
            base: BaseRef { revision: 1 },
            edit: edit_debug(true),
            candidate: Some(CanonicalConfig {
                debug: true,
                ..Default::default()
            }),
            record: OperationRecord {
                op: OperationId(7),
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
        j.fence_unresolved();
        let e = j.get(OperationId(7)).unwrap();
        assert_eq!(e.record.phases[1].outcome, PhaseOutcome::Unknown);
        assert_eq!(e.record.settled, Some(PhaseOutcome::Unknown));
        assert_eq!(e.edit, edit_debug(true));
    }

    #[test]
    fn retention_retires_ids_so_retry_cannot_mutate() {
        let mut j = Journal {
            retain_settled: 2,
            ..Journal::default()
        };
        j.upsert(confirmed(1));
        j.upsert(confirmed(2));
        j.upsert(confirmed(3));
        assert!(j.get(OperationId(1)).is_none());
        assert!(j.is_retired(OperationId(1)));
        assert!(j.get(OperationId(2)).is_some());
        assert!(j.get(OperationId(3)).is_some());
        // Unknown receipts may expire; resource uncertainty outlives the receipt.
        j.upsert(JournalEntry {
            maintenance: None,
            maintenance_report: None,
            admission: None,
            command: None,
            superseded_by: None,
            superseded_phases: Vec::new(),
            finalized_unknown: false,
            native_intent: None,
            base: BaseRef { revision: 0 },
            edit: ConfigEdit::Replace {
                config: CanonicalConfig::default(),
            },
            candidate: Some(CanonicalConfig::default()),
            record: OperationRecord {
                op: OperationId(99),
                phases: vec![PhaseRecord {
                    phase: ConfigPhase::Native,
                    outcome: PhaseOutcome::Unknown,
                }],
                settled: Some(PhaseOutcome::Unknown),
                ports: None,
            },
        });
        j.upsert(confirmed(4));
        j.upsert(confirmed(5));
        assert!(j.get(OperationId(99)).is_none());
        assert!(j.expired_holds.contains(&ConfigPhase::Native));
        assert!(j.has_unknown());
        assert!(j.is_retired(OperationId(2)) || j.get(OperationId(2)).is_none());
    }

    #[test]
    fn crash_window_intent_survives_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("operations.json");
        let mut j = Journal::default();
        let candidate = CanonicalConfig {
            debug: true,
            ..Default::default()
        };
        record_phase_running(
            &path,
            &mut j,
            OperationId(1),
            BaseRef { revision: 0 },
            &edit_debug(true),
            &candidate,
            &[ConfigPhase::Persist, ConfigPhase::Native],
            &[],
            ConfigPhase::Persist,
            None,
        )
        .unwrap();
        // Simulate kill before Persist completes: reload + fence.
        let mut reloaded = read(&path).unwrap();
        reloaded.fence_unresolved();
        let e = reloaded.get(OperationId(1)).unwrap();
        assert_eq!(e.record.settled, Some(PhaseOutcome::Unknown));
        assert_eq!(e.edit, edit_debug(true));
        assert!(
            e.record
                .phases
                .iter()
                .any(|p| p.phase == ConfigPhase::Persist && p.outcome == PhaseOutcome::Unknown)
        );
    }

    #[test]
    fn native_exact_intent_survives_later_phases_and_settle() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("operations.json");
        let mut journal = Journal::default();
        let candidate = CanonicalConfig::default();
        let edit = ConfigEdit::RepairActivation;
        let intent = NativeIntent {
            zygisk_authorization: None,
            backend: NativeBackend::Zygisk,
            wire: "vpnhide 2 config\ndebug 0\ntargets f 278b\nend 1\n".into(),
            zygisk_module: Some(dir.path().join("zygisk")),
        };
        let plan = [ConfigPhase::Native, ConfigPhase::Ports];
        record_phase_running(
            &path,
            &mut journal,
            OperationId(1),
            BaseRef { revision: 0 },
            &edit,
            &candidate,
            &plan,
            &[],
            ConfigPhase::Native,
            Some(intent.clone()),
        )
        .unwrap();
        record_phase_running(
            &path,
            &mut journal,
            OperationId(1),
            BaseRef { revision: 0 },
            &edit,
            &candidate,
            &plan,
            &[(ConfigPhase::Native, PhaseOutcome::Confirmed)],
            ConfigPhase::Ports,
            None,
        )
        .unwrap();
        let record = journal.get(OperationId(1)).unwrap().record.clone();
        record_settled(
            &path,
            &mut journal,
            OperationId(1),
            BaseRef { revision: 0 },
            &edit,
            &candidate,
            record,
            0,
        )
        .unwrap();
        assert_eq!(
            read(&path)
                .unwrap()
                .get(OperationId(1))
                .unwrap()
                .native_intent,
            Some(intent)
        );
    }
}
