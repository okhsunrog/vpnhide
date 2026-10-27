//! One-time evidence readback for unfinished commands.
//!
//! After restart fencing turns in-flight work into durable `Unknown`, recovery
//! reconciles exact durable evidence without dispatching effects. Once an Unknown
//! receipt was returned or recovered, it stays immutable. Explicit desired-state
//! repair records new resource evidence and supersedes held phases separately.

use std::fs;
use std::path::Path;

use vpnhide_core_rpc::{CanonicalConfig, ConfigPhase, NativeBackend, PhaseOutcome, PhaseRecord};

use crate::config_file;
use crate::journal::{JournalEntry, NativeIntent};

/// Paths/observations available for readback. All optional: missing evidence
/// keeps the phase Unknown rather than guessing.
pub struct EvidenceSources<'a> {
    pub config_path: Option<&'a Path>,
}

/// Reconcile one journal entry in place. Returns whether anything changed.
///
/// Rules:
/// - `Persist` Unknown: compare durable config file to stored `candidate`.
/// - `Native` Unknown: Zygisk `targets.txt` may prove publication; kernel ctl
///   content is **not** trusted as apply proof (proc nodes return status, not
///   the written wire). Insufficient evidence → leave Unknown.
/// - `NotAttempted` phases are never promoted to Confirmed.
/// - Overall `settled` becomes Confirmed only when every planned phase is
///   Confirmed; FailedKnown if any failed and none remain Unknown; else Unknown.
pub fn reconcile_entry(entry: &mut JournalEntry, sources: &EvidenceSources<'_>) -> bool {
    if entry.superseded_by.is_some() || entry.finalized_unknown {
        return false;
    }
    // A completed failed plan intentionally leaves later phases NotAttempted. It
    // must not become an unfinished operation just because core restarted.
    if matches!(
        entry.record.settled,
        Some(PhaseOutcome::Confirmed | PhaseOutcome::FailedKnown)
    ) && !entry
        .record
        .phases
        .iter()
        .any(|p| matches!(p.outcome, PhaseOutcome::Unknown | PhaseOutcome::Running))
    {
        return false;
    }
    let Some(candidate) = entry.candidate.as_ref() else {
        // Without a stored candidate we cannot prove Persist intent; leave as-is.
        return false;
    };
    let mut changed = false;

    for phase in &mut entry.record.phases {
        if phase.outcome != PhaseOutcome::Unknown || entry.superseded_phases.contains(&phase.phase)
        {
            continue;
        }
        let resolved = match phase.phase {
            ConfigPhase::Persist => {
                if entry.command == Some(crate::journal::CommandIntent::FullReset) {
                    match sources.config_path.map(config_file::read) {
                        Some(Ok(None)) => PhaseOutcome::Confirmed,
                        Some(Ok(Some(_))) => PhaseOutcome::FailedKnown,
                        _ => PhaseOutcome::Unknown,
                    }
                } else {
                    reconcile_persist(candidate, sources.config_path)
                }
            }
            ConfigPhase::Native => reconcile_native(entry.native_intent.as_ref()),
            // A chain's existence proves neither its contents nor this operation's
            // IPv4/IPv6 policy. No exact readback receipt is available yet.
            ConfigPhase::Ports => PhaseOutcome::Unknown,
            // Secret/Cleanup: no evidence path in this slice.
            _ => PhaseOutcome::Unknown,
        };
        if resolved != PhaseOutcome::Unknown {
            phase.outcome = resolved;
            changed = true;
        }
    }

    let new_settled = settle_from_phases(&entry.record.phases);
    if entry.record.settled != Some(new_settled) {
        entry.record.settled = Some(new_settled);
        changed = true;
    }
    changed
}

fn settle_from_phases(phases: &[PhaseRecord]) -> PhaseOutcome {
    if phases.is_empty() {
        return PhaseOutcome::Unknown;
    }
    if phases
        .iter()
        .any(|p| matches!(p.outcome, PhaseOutcome::Unknown | PhaseOutcome::Running))
    {
        // Running should already have been fenced to Unknown.
        return PhaseOutcome::Unknown;
    }
    if phases
        .iter()
        .any(|p| p.outcome == PhaseOutcome::FailedKnown)
    {
        return PhaseOutcome::FailedKnown;
    }
    if phases
        .iter()
        .any(|p| p.outcome == PhaseOutcome::NotAttempted)
    {
        // With no known failure, this is an unfinished plan. Never dispatch it.
        return PhaseOutcome::Unknown;
    }
    if phases.iter().all(|p| p.outcome == PhaseOutcome::Confirmed) {
        return PhaseOutcome::Confirmed;
    }
    PhaseOutcome::Unknown
}

/// Persist evidence: the durable config file is the sole writer’s commit point.
fn reconcile_persist(candidate: &CanonicalConfig, config_path: Option<&Path>) -> PhaseOutcome {
    let Some(path) = config_path else {
        return PhaseOutcome::Unknown;
    };
    match config_file::read(path) {
        Ok(Some(on_disk)) if on_disk == *candidate => PhaseOutcome::Confirmed,
        // Readable, and either missing or a different whole config: Persist did
        // not land. Atomic rename means we never see a torn file.
        Ok(Some(_)) | Ok(None) => PhaseOutcome::FailedKnown,
        // Unreadable / corrupt: do not invent FailedKnown or Confirmed.
        Err(_) => PhaseOutcome::Unknown,
    }
}

fn reconcile_native(intent: Option<&NativeIntent>) -> PhaseOutcome {
    let Some(intent) = intent else {
        return PhaseOutcome::Unknown;
    };
    if intent.backend == NativeBackend::Zygisk {
        let Some(module) = intent.zygisk_module.as_deref() else {
            return PhaseOutcome::Unknown;
        };
        return reconcile_zygisk_publication(module, &intent.wire);
    }
    // Kernel / KPM: telemetry proves liveness, not that this candidate’s wire
    // was applied. Refusing to invent Confirmed leaves the op Held.
    PhaseOutcome::Unknown
}

fn reconcile_zygisk_publication(module: &Path, expected_wire: &str) -> PhaseOutcome {
    let targets = module.join("targets.txt");
    let on_disk = match fs::read_to_string(&targets) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return PhaseOutcome::FailedKnown;
        }
        Err(_) => return PhaseOutcome::Unknown,
    };
    if expected_wire == on_disk {
        PhaseOutcome::Confirmed
    } else {
        PhaseOutcome::FailedKnown
    }
}

/// After Persist is reconciled to Confirmed, advance the durable revision if the
/// journal still carries the pre-edit counter (crash between Persist and settle).
pub fn persist_commit_observed(entry: &JournalEntry, durable: &Option<CanonicalConfig>) -> bool {
    entry.record.phases.iter().any(|phase| {
        phase.phase == ConfigPhase::Persist
            && (phase.outcome == PhaseOutcome::Confirmed
                || (phase.outcome == PhaseOutcome::Unknown
                    && if entry.command == Some(crate::journal::CommandIntent::FullReset) {
                        durable.is_none()
                    } else {
                        entry.candidate.is_some() && entry.candidate == *durable
                    }))
    })
}

pub fn pending_persist_matches(
    proof: &crate::journal::PendingPersist,
    durable: &Option<CanonicalConfig>,
) -> bool {
    if proof.reset {
        durable.is_none()
    } else {
        proof.candidate.is_some() && proof.candidate == *durable
    }
}

pub fn revision_after_reconcile(
    entry: &JournalEntry,
    journal_revision: u64,
    durable: &Option<CanonicalConfig>,
) -> u64 {
    let persist_confirmed = persist_commit_observed(entry, durable);
    // Never derive commit identity from the client's (possibly stale) base.
    // Legacy entries without admitted metadata retain uncertainty rather than
    // inventing a revision or losing same-field conflicts.
    if persist_confirmed {
        entry
            .admission
            .as_ref()
            .map_or(journal_revision, |admission| {
                journal_revision.max(admission.commit_revision)
            })
    } else {
        journal_revision
    }
}

/// Test-only crash injection. Production never sets `VPNHIDE_TEST_CRASH_AFTER`.
pub fn maybe_crash_for_test(point: &str) {
    match std::env::var("VPNHIDE_TEST_CRASH_AFTER") {
        Ok(v) if v == point => {
            eprintln!("core: test crash after {point}");
            std::process::exit(97);
        }
        _ => {}
    }
}

/// Test-only delay before a phase effect (`VPNHIDE_TEST_DELAY_MS` + point via
/// `VPNHIDE_TEST_DELAY_BEFORE`). Production never sets these.
pub fn maybe_delay_for_test(point: &str) {
    let Ok(before) = std::env::var("VPNHIDE_TEST_DELAY_BEFORE") else {
        return;
    };
    if before != point {
        return;
    }
    let ms: u64 = std::env::var("VPNHIDE_TEST_DELAY_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);
    eprintln!("core: test delay {ms}ms before {point}");
    std::thread::sleep(std::time::Duration::from_millis(ms));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activation;
    use crate::journal::JournalEntry;
    use vpnhide_core_rpc::{BaseRef, ConfigEdit, ConfigToggle, OperationId, OperationRecord};

    fn entry(
        persist: PhaseOutcome,
        native: Option<PhaseOutcome>,
        candidate: CanonicalConfig,
    ) -> JournalEntry {
        let mut phases = vec![PhaseRecord {
            phase: ConfigPhase::Persist,
            outcome: persist,
        }];
        if let Some(n) = native {
            phases.push(PhaseRecord {
                phase: ConfigPhase::Native,
                outcome: n,
            });
        }
        JournalEntry {
            admission: None,
            command: None,
            superseded_by: None,
            superseded_phases: Vec::new(),
            finalized_unknown: false,
            native_intent: None,
            base: BaseRef { revision: 0 },
            edit: ConfigEdit::Toggle {
                field: ConfigToggle::Debug,
                enabled: true,
            },
            candidate: Some(candidate.clone()),
            record: OperationRecord {
                op: OperationId(1),
                phases,
                settled: Some(PhaseOutcome::Unknown),
                ports: None,
            },
        }
    }

    #[test]
    fn persist_unknown_confirms_when_file_matches_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vpnhide_config.json");
        let candidate = CanonicalConfig {
            debug: true,
            ..Default::default()
        };
        config_file::write_atomic(&path, &candidate).unwrap();
        let mut e = entry(PhaseOutcome::Unknown, None, candidate);
        let sources = EvidenceSources {
            config_path: Some(&path),
        };
        assert!(reconcile_entry(&mut e, &sources));
        assert_eq!(e.record.phases[0].outcome, PhaseOutcome::Confirmed);
        assert_eq!(e.record.settled, Some(PhaseOutcome::Confirmed));
    }

    #[test]
    fn persist_unknown_fails_when_file_is_stale() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vpnhide_config.json");
        config_file::write_atomic(&path, &CanonicalConfig::default()).unwrap();
        let candidate = CanonicalConfig {
            debug: true,
            ..Default::default()
        };
        let mut e = entry(PhaseOutcome::Unknown, None, candidate);
        let sources = EvidenceSources {
            config_path: Some(&path),
        };
        assert!(reconcile_entry(&mut e, &sources));
        assert_eq!(e.record.phases[0].outcome, PhaseOutcome::FailedKnown);
        assert_eq!(e.record.settled, Some(PhaseOutcome::FailedKnown));
    }

    #[test]
    fn native_kernel_stays_unknown_without_inventing_success() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("cfg.json");
        let ctl = dir.path().join("ctl");
        let candidate = CanonicalConfig {
            debug: true,
            ..Default::default()
        };
        config_file::write_atomic(&config, &candidate).unwrap();
        // A ctl file that happens to contain text must not confirm Native.
        fs::write(&ctl, "anything").unwrap();
        let mut e = entry(
            PhaseOutcome::Confirmed,
            Some(PhaseOutcome::Unknown),
            candidate,
        );
        e.native_intent = Some(NativeIntent {
            backend: NativeBackend::Builtin,
            wire: "vpnhide 2 config\ndebug 1\nend 0\n".into(),
            // A production Zygisk path exists in the profile for every backend;
            // it must never decide which backend this operation delivered to.
            zygisk_module: Some(dir.path().join("missing-zygisk")),
        });
        let sources = EvidenceSources {
            config_path: Some(&config),
        };
        assert!(
            !reconcile_entry(&mut e, &sources)
                || e.record.phases[1].outcome == PhaseOutcome::Unknown
        );
        assert_eq!(e.record.phases[1].outcome, PhaseOutcome::Unknown);
        assert_eq!(e.record.settled, Some(PhaseOutcome::Unknown));
    }

    #[test]
    fn native_zygisk_confirms_when_targets_match_projection() {
        let dir = tempfile::tempdir().unwrap();
        let module = dir.path().join("module");
        fs::create_dir_all(&module).unwrap();
        fs::write(module.join("module.prop"), "id=vpnhide_zygisk\n").unwrap();
        let mut candidate = CanonicalConfig::default();
        candidate.apps.insert(
            "com.example.app".into(),
            vpnhide_core_rpc::AppConfig {
                native: vpnhide_core_rpc::NativeRole {
                    enabled: true,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let resolver = vpnhide_activator::parse_pm_packages("package:com.example.app uid:10123\n");
        let wire = activation::project_native_wire_with_resolver(
            &candidate,
            NativeBackend::Zygisk,
            &resolver,
        )
        .unwrap();
        assert!(
            wire.contains("278b"),
            "resolved UID must be in the actual wire"
        );
        fs::write(module.join("targets.txt"), &wire).unwrap();
        let mut e = entry(
            PhaseOutcome::Confirmed,
            Some(PhaseOutcome::Unknown),
            candidate,
        );
        e.native_intent = Some(NativeIntent {
            backend: NativeBackend::Zygisk,
            wire,
            zygisk_module: Some(module.clone()),
        });
        let sources = EvidenceSources { config_path: None };
        assert!(reconcile_entry(&mut e, &sources));
        assert_eq!(e.record.phases[1].outcome, PhaseOutcome::Confirmed);
        assert_eq!(e.record.settled, Some(PhaseOutcome::Confirmed));
    }

    #[test]
    fn incomplete_not_attempted_native_keeps_hold() {
        let candidate = CanonicalConfig {
            debug: true,
            ..Default::default()
        };
        let mut e = entry(
            PhaseOutcome::Confirmed,
            Some(PhaseOutcome::NotAttempted),
            candidate,
        );
        e.record.settled = Some(PhaseOutcome::Unknown);
        let sources = EvidenceSources { config_path: None };
        reconcile_entry(&mut e, &sources);
        assert_eq!(e.record.settled, Some(PhaseOutcome::Unknown));
        assert_eq!(e.record.phases[1].outcome, PhaseOutcome::NotAttempted);
    }

    #[test]
    fn known_failure_with_skipped_later_phases_is_terminal() {
        let mut e = entry(
            PhaseOutcome::Confirmed,
            Some(PhaseOutcome::FailedKnown),
            CanonicalConfig::default(),
        );
        e.record.phases.push(PhaseRecord {
            phase: ConfigPhase::Ports,
            outcome: PhaseOutcome::NotAttempted,
        });
        e.record.settled = Some(PhaseOutcome::FailedKnown);
        let before = e.clone();
        assert!(!reconcile_entry(
            &mut e,
            &EvidenceSources { config_path: None }
        ));
        assert_eq!(e, before);
        // A crash before writing the overall terminal result is also recoverable.
        e.record.settled = Some(PhaseOutcome::Unknown);
        assert!(reconcile_entry(
            &mut e,
            &EvidenceSources { config_path: None }
        ));
        assert_eq!(e.record.settled, Some(PhaseOutcome::FailedKnown));
    }

    #[test]
    fn legacy_native_without_exact_intent_keeps_uncertainty() {
        let mut e = entry(
            PhaseOutcome::Confirmed,
            Some(PhaseOutcome::Unknown),
            CanonicalConfig::default(),
        );
        assert!(!reconcile_entry(
            &mut e,
            &EvidenceSources { config_path: None }
        ));
        assert_eq!(e.record.phases[1].outcome, PhaseOutcome::Unknown);
    }

    #[test]
    fn ports_without_exact_rule_receipt_stays_unknown() {
        let mut e = entry(PhaseOutcome::Confirmed, None, CanonicalConfig::default());
        e.record.phases.push(PhaseRecord {
            phase: ConfigPhase::Ports,
            outcome: PhaseOutcome::Unknown,
        });
        reconcile_entry(&mut e, &EvidenceSources { config_path: None });
        assert_eq!(e.record.settled, Some(PhaseOutcome::Unknown));
    }
}
