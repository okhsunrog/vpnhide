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

const JOURNAL_VERSION: u32 = 2;

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
    SetSuperkey { remember: bool },
    ClearSuperkey,
    Import { cleanup_legacy: bool },
    CleanupLegacy,
    FullReset,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeIntent {
    pub backend: NativeBackend,
    pub wire: String,
    /// The actual recipient selected for this operation, never the current env default.
    pub zygisk_module: Option<PathBuf>,
}

/// On-disk journal document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    pub version: u32,
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
        !self.expired_holds.is_empty()
            || self
                .operations
                .iter()
                .any(|entry| !entry.held_phases().is_empty())
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
        let settled_idxs: Vec<usize> = self
            .operations
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.record.settled.is_some() && Some(e.record.op) != self.debug_restore_op
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
    if journal.version > JOURNAL_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "journal version {} newer than supported {}",
                journal.version, JOURNAL_VERSION
            ),
        ));
    }
    let mut journal = journal;
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

/// Serialize and write atomically (temp + fsync + rename), mode `0600`.
pub fn write_atomic(path: &Path, journal: &Journal) -> io::Result<()> {
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
    let prior = journal.get(op).cloned();
    journal.upsert(JournalEntry {
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

    fn edit_debug(on: bool) -> ConfigEdit {
        ConfigEdit::Toggle {
            field: ConfigToggle::Debug,
            enabled: on,
        }
    }

    fn confirmed(op: u64) -> JournalEntry {
        JournalEntry {
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
