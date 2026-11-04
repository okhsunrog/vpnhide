//! Durable device-local intent. Canonical imports never create backend consent.
use serde::{Deserialize, Serialize};
use vpnhide_core_rpc::{BackendChoiceRequest, OperationId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendChoiceRecord {
    pub revision: u64,
    pub request: BackendChoiceRequest,
    pub operation: OperationId,
    pub requested_in_boot: String,
}

impl BackendChoiceRecord {
    pub(super) fn valid(&self, issued: u64) -> bool {
        self.revision > 0
            && self.request.expected_revision.checked_add(1) == Some(self.revision)
            && self.request.validate().is_ok()
            && self.operation.0 > 0
            && self.operation.0 <= issued
            && !self.requested_in_boot.is_empty()
            && self.requested_in_boot.len() <= 128
            && !self.requested_in_boot.chars().any(char::is_control)
    }
}

pub(super) fn validate_entries(journal: &super::Journal) -> bool {
    use vpnhide_core_rpc::{ConfigEdit, ConfigPhase, PhaseOutcome};
    for entry in &journal.operations {
        let Some(super::CommandIntent::StageBackendChoice { request }) = &entry.command else {
            continue;
        };
        if request.validate().is_err()
            || entry.admission.as_ref().is_none_or(|a| {
                a.from_revision != entry.base.revision
                    || a.commit_revision != entry.base.revision
                    || !a.writes.is_empty()
            })
            || entry.edit != ConfigEdit::RepairActivation
            || entry.candidate.is_some()
            || entry.native_intent.is_some()
            || entry.maintenance.is_some()
            || entry.maintenance_report.is_some()
            || entry.record.phases.len() != 1
            || entry.record.phases[0].phase != ConfigPhase::Maintenance
        {
            return false;
        }
    }
    journal.backend_choice.as_ref().is_none_or(|record| {
        journal.get(record.operation).is_none_or(|entry| {
            entry.command
                == Some(super::CommandIntent::StageBackendChoice {
                    request: record.request.clone(),
                })
                && entry.record.settled == Some(PhaseOutcome::Confirmed)
                && entry
                    .record
                    .phases
                    .iter()
                    .all(|p| p.outcome == PhaseOutcome::Confirmed)
        })
    })
}

/// The existing writer's one recorder for managed preparation. It preserves the
/// dispatch history and atomically couples the private choice with its receipt.
/// Persist the resulting Journal before returning from the executor callback.
/// This does not admit a request, authorize hooks or dispatch a manager itself.
pub fn record_projection(
    journal: &mut super::Journal,
    base: vpnhide_core_rpc::BaseRef,
    work: &crate::maintenance::zygisk_projection::work::ProjectionWork,
) -> std::io::Result<()> {
    use vpnhide_core_rpc::{ConfigEdit, ConfigPhase, OperationRecord, PhaseOutcome, PhaseRecord};
    let invalid = || {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "projection intent cannot replace retained work",
        )
    };
    if journal.version != super::JOURNAL_VERSION
        || !work.valid(journal.issued_through)
        || base.revision != journal.config_revision
    {
        return Err(invalid());
    }
    let prior = journal.zygisk_projection.as_ref();
    if let Some(previous) = prior.filter(|p| p.operation == work.operation) {
        if !super::super::maintenance::zygisk_projection::work::validate_journal(journal)
            || journal.get(work.operation).is_none_or(|e| e.base != base)
            || !previous.permits_successor(work)
        {
            return Err(invalid());
        }
        if previous == work {
            return Ok(());
        }
    } else if prior.is_some_and(|p| p.retained())
        || journal.get(work.operation).is_some()
        || journal.is_retired(work.operation)
        || work.dispatch_intent
        || work.outcome != PhaseOutcome::Running
    {
        return Err(invalid());
    }
    let revision = journal
        .backend_choice
        .as_ref()
        .map_or(0, |choice| choice.revision);
    if revision != work.request.expected_revision {
        return Err(invalid());
    }
    if work.outcome == PhaseOutcome::Confirmed {
        journal.backend_choice = Some(BackendChoiceRecord {
            revision: work.request.expected_revision + 1,
            request: work.request.clone(),
            operation: work.operation,
            requested_in_boot: work.boot_id.clone(),
        });
    }
    journal.zygisk_projection = Some(work.clone());
    journal.upsert(super::JournalEntry {
        admission: Some(super::Admission {
            from_revision: base.revision,
            commit_revision: base.revision,
            writes: vec![],
        }),
        command: Some(super::CommandIntent::StageBackendChoice {
            request: work.request.clone(),
        }),
        maintenance: None,
        maintenance_report: None,
        superseded_by: None,
        superseded_phases: vec![],
        finalized_unknown: work.outcome == PhaseOutcome::Unknown,
        base,
        edit: ConfigEdit::RepairActivation,
        candidate: None,
        native_intent: None,
        record: OperationRecord {
            op: work.operation,
            phases: vec![PhaseRecord {
                phase: ConfigPhase::Maintenance,
                outcome: work.outcome,
            }],
            settled: (work.outcome != PhaseOutcome::Running).then_some(work.outcome),
            ports: None,
        },
    });
    Ok(())
}

/// A restart never executes a staged command again. The preference and terminal
/// receipt commit atomically; a retained Running-only intent proves no preference
/// change landed. Historical terminal Unknown remains immutable.
pub(crate) fn reconcile_unfinished(journal: &mut super::Journal) -> bool {
    use vpnhide_core_rpc::{ConfigPhase, PhaseOutcome};
    let current = journal.backend_choice.as_ref().map(|r| r.operation);
    let mut changed = false;
    for entry in &mut journal.operations {
        if !matches!(
            entry.command,
            Some(super::CommandIntent::StageBackendChoice { .. })
        ) || journal
            .zygisk_projection
            .as_ref()
            .is_some_and(|work| work.operation == entry.record.op)
            || entry.finalized_unknown
            || entry.record.settled.is_some()
            || entry.record.phases.len() != 1
            || entry.record.phases[0].phase != ConfigPhase::Maintenance
        {
            continue;
        }
        let outcome = if current == Some(entry.record.op) {
            PhaseOutcome::Unknown
        } else {
            PhaseOutcome::FailedKnown
        };
        entry.record.phases[0].outcome = outcome;
        entry.record.settled = Some(outcome);
        entry.finalized_unknown = outcome == PhaseOutcome::Unknown;
        changed = true;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::{CommandIntent, Journal, JournalEntry};
    use vpnhide_core_rpc::{
        BackendChoice, BaseRef, ConfigEdit, ConfigPhase, OperationRecord, PhaseOutcome, PhaseRecord,
    };

    fn fixture(confirmed: bool) -> Journal {
        let request = BackendChoiceRequest {
            expected_revision: 0,
            choice: BackendChoice::Auto,
            acknowledged_warning: None,
        };
        let outcome = if confirmed {
            PhaseOutcome::Confirmed
        } else {
            PhaseOutcome::Running
        };
        Journal {
            issued_through: 8,
            backend_choice: confirmed.then(|| BackendChoiceRecord {
                revision: 1,
                request: request.clone(),
                operation: OperationId(8),
                requested_in_boot: "boot-a".into(),
            }),
            operations: vec![JournalEntry {
                admission: Some(crate::journal::Admission {
                    from_revision: 0,
                    commit_revision: 0,
                    writes: vec![],
                }),
                command: Some(CommandIntent::StageBackendChoice { request }),
                maintenance: None,
                maintenance_report: None,
                superseded_by: None,
                superseded_phases: vec![],
                finalized_unknown: false,
                base: BaseRef { revision: 0 },
                edit: ConfigEdit::RepairActivation,
                candidate: None,
                native_intent: None,
                record: OperationRecord {
                    op: OperationId(8),
                    phases: vec![PhaseRecord {
                        phase: ConfigPhase::Maintenance,
                        outcome,
                    }],
                    settled: confirmed.then_some(outcome),
                    ports: None,
                },
            }],
            ..Default::default()
        }
    }

    #[test]
    fn unsupported_or_inconsistent_preferences_never_reset_authority() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("journal.json");
        for version in 1..16 {
            let mut journal = fixture(true);
            journal.version = version;
            super::super::write_atomic(&path, &journal).unwrap();
            assert!(super::super::read(&path).is_err());
        }
        for variant in 0..5 {
            let mut journal = fixture(true);
            let record = journal.backend_choice.as_mut().unwrap();
            match variant {
                0 => record.revision = 0,
                1 => record.operation = OperationId(9),
                2 => record.request.expected_revision = 9,
                3 => record.requested_in_boot.clear(),
                _ => journal.operations[0].record.settled = None,
            }
            super::super::write_atomic(&path, &journal).unwrap();
            assert!(super::super::read(&path).is_err());
        }
    }

    #[test]
    fn schema16_preferences_migrate_without_losing_choice_or_holds() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("journal.json");
        let mut journal = fixture(true);
        journal.version = 16;
        journal.expired_holds = vec![ConfigPhase::Native, ConfigPhase::Ports];
        super::super::write_atomic(&path, &journal).unwrap();
        let restored = super::super::read(&path).unwrap();
        assert_eq!(restored.version, super::super::JOURNAL_VERSION);
        assert_eq!(restored.backend_choice, journal.backend_choice);
        assert_eq!(restored.operations, journal.operations);
        assert_eq!(restored.issued_through, journal.issued_through);
        assert_eq!(restored.expired_holds, journal.expired_holds);
        assert!(restored.zygisk_projection.is_none());
    }

    #[test]
    fn pruned_receipt_keeps_preference_and_holds_and_does_not_replay() {
        let mut journal = fixture(true);
        journal.operations.clear();
        journal.expired_through = 8;
        journal.expired_holds = vec![ConfigPhase::Native];
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("journal.json");
        super::super::write_atomic(&path, &journal).unwrap();
        let mut restored = super::super::read(&path).unwrap();
        assert_eq!(restored.backend_choice, journal.backend_choice);
        assert_eq!(restored.held_resources(), vec![ConfigPhase::Native]);
        assert!(!reconcile_unfinished(&mut restored));
        assert!(restored.is_retired(OperationId(8)));
    }

    #[test]
    fn unfinished_atomic_preference_is_known_no_effect_and_unknown_is_immutable() {
        let mut journal = fixture(false);
        assert!(reconcile_unfinished(&mut journal));
        assert_eq!(
            journal.operations[0].record.settled,
            Some(PhaseOutcome::FailedKnown)
        );
        assert!(journal.backend_choice.is_none());
        assert!(!reconcile_unfinished(&mut journal));
        journal.operations[0].record.settled = Some(PhaseOutcome::Unknown);
        journal.operations[0].finalized_unknown = true;
        assert!(!reconcile_unfinished(&mut journal));
        assert_eq!(
            journal.operations[0].record.settled,
            Some(PhaseOutcome::Unknown)
        );
    }
}
