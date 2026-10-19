//! Private durable operation journal (core-2.0 §4 tier-3 / §5).
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
//! Settled confirmed/failed records are capped (`retain_settled`). Pruned IDs
//! move into `retired_ids` so a later retry cannot be admitted as a new
//! mutation. Unsettled and `Unknown` records are never pruned.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use vpnhide_core_rpc::{
    BaseRef, ConfigEdit, ConfigPhase, OperationId, OperationRecord, PhaseOutcome, PhaseRecord,
};

/// Production path of the private journal (root-owned, not LSPosed-readable by design).
pub const DEFAULT_JOURNAL_PATH: &str = "/data/adb/vpnhide/core/operations.json";

/// Env override so the dev sandbox never touches the production journal.
pub const JOURNAL_PATH_ENV: &str = "VPNHIDE_JOURNAL_PATH";

const JOURNAL_VERSION: u32 = 1;

/// Default number of settled (confirmed/failed-known) records to keep.
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
    pub base: BaseRef,
    pub edit: ConfigEdit,
    pub record: OperationRecord,
}

/// On-disk journal document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    pub version: u32,
    /// Durable config-revision counter — continues across restarts (§5).
    pub config_revision: u64,
    pub retain_settled: usize,
    pub operations: Vec<JournalEntry>,
    /// IDs removed by retention; reuse is rejected so a pruned retry cannot mutate again.
    pub retired_ids: Vec<OperationId>,
}

impl Default for Journal {
    fn default() -> Self {
        Journal {
            version: JOURNAL_VERSION,
            config_revision: 0,
            retain_settled: DEFAULT_RETAIN_SETTLED,
            operations: Vec::new(),
            retired_ids: Vec::new(),
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
        self.retired_ids.contains(&op)
    }

    /// True when any stored operation still carries unresolved uncertainty.
    pub fn has_unknown(&self) -> bool {
        self.operations.iter().any(|e| {
            e.record.settled == Some(PhaseOutcome::Unknown)
                || e.record
                    .phases
                    .iter()
                    .any(|p| p.outcome == PhaseOutcome::Running)
        })
    }

    /// Insert or replace one entry, then apply retention.
    pub fn upsert(&mut self, entry: JournalEntry) {
        let op = entry.record.op;
        if let Some(slot) = self.operations.iter_mut().find(|e| e.record.op == op) {
            *slot = entry;
        } else {
            self.operations.push(entry);
        }
        self.prune();
    }

    /// Drop oldest settled confirmed/failed records beyond `retain_settled`, retiring their IDs.
    fn prune(&mut self) {
        let retain = self.retain_settled;
        let settled_idxs: Vec<usize> = self
            .operations
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                matches!(
                    e.record.settled,
                    Some(PhaseOutcome::Confirmed) | Some(PhaseOutcome::FailedKnown)
                )
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
                if !self.retired_ids.contains(&entry.record.op) {
                    self.retired_ids.push(entry.record.op);
                }
            } else {
                kept.push(entry);
            }
        }
        self.operations = kept;
    }

    /// Restart fencing: any in-flight (`Running`) phase becomes durable `Unknown`.
    /// Effects are **not** re-dispatched; uncertainty is preserved until evidence resolves it.
    pub fn fence_unresolved(&mut self) {
        for entry in &mut self.operations {
            let mut uncertain = entry.record.settled == Some(PhaseOutcome::Unknown);
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

/// Read the journal. Missing/blank → empty journal. Corrupt → error (do not invent state).
pub fn read(path: &Path) -> io::Result<Journal> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Journal::default()),
        Err(e) => return Err(e),
    };
    if raw.trim().is_empty() {
        return Ok(Journal::default());
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
        let mut f = fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&tmp, path)?;
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
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
    plan: &[ConfigPhase],
    completed: &[(ConfigPhase, PhaseOutcome)],
    running: ConfigPhase,
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
    journal.upsert(JournalEntry {
        base,
        edit: edit.clone(),
        record: OperationRecord {
            op,
            phases,
            settled: None,
        },
    });
    write_atomic(path, journal)
}

/// Persist the settled operation record (and optional revision bump) after phases finish.
pub fn record_settled(
    path: &Path,
    journal: &mut Journal,
    _op: OperationId,
    base: BaseRef,
    edit: &ConfigEdit,
    record: OperationRecord,
    config_revision: u64,
) -> io::Result<()> {
    journal.config_revision = config_revision;
    journal.upsert(JournalEntry {
        base,
        edit: edit.clone(),
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
            base: BaseRef { revision: 0 },
            edit: edit_debug(true),
            record: OperationRecord {
                op: OperationId(op),
                phases: vec![PhaseRecord {
                    phase: ConfigPhase::Persist,
                    outcome: PhaseOutcome::Confirmed,
                }],
                settled: Some(PhaseOutcome::Confirmed),
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
    fn fence_turns_running_into_unknown_without_inventing_success() {
        let mut j = Journal::default();
        j.upsert(JournalEntry {
            base: BaseRef { revision: 1 },
            edit: edit_debug(true),
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
        // Unknown is never pruned even when over the settled cap.
        j.upsert(JournalEntry {
            base: BaseRef { revision: 0 },
            edit: ConfigEdit::Replace {
                config: CanonicalConfig::default(),
            },
            record: OperationRecord {
                op: OperationId(99),
                phases: vec![PhaseRecord {
                    phase: ConfigPhase::Native,
                    outcome: PhaseOutcome::Unknown,
                }],
                settled: Some(PhaseOutcome::Unknown),
            },
        });
        j.upsert(confirmed(4));
        j.upsert(confirmed(5));
        assert!(
            j.get(OperationId(99)).is_some(),
            "Unknown must survive retention"
        );
        assert!(j.is_retired(OperationId(2)) || j.get(OperationId(2)).is_none());
    }

    #[test]
    fn crash_window_intent_survives_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("operations.json");
        let mut j = Journal::default();
        record_phase_running(
            &path,
            &mut j,
            OperationId(1),
            BaseRef { revision: 0 },
            &edit_debug(true),
            &[ConfigPhase::Persist, ConfigPhase::Native],
            &[],
            ConfigPhase::Persist,
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
}
