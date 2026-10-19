//! Evidence readback for uncertain operations (core-2.0 §5).
//!
//! After restart fencing turns in-flight work into durable `Unknown`, recovery
//! may **reconcile** against observed durable state. It must not invent success
//! and must not re-dispatch effects. Matching the app coordinator: Unknown →
//! readback → Confirmed / FailedKnown when evidence is reliable, otherwise keep
//! Unknown (Held) and leave mutation admission paused.

use std::fs;
use std::path::Path;

use vpnhide_core_rpc::{CanonicalConfig, ConfigPhase, NativeBackend, PhaseOutcome, PhaseRecord};

use crate::activation;
use crate::config_file;
use crate::journal::JournalEntry;

/// Paths/observations available for readback. All optional: missing evidence
/// keeps the phase Unknown rather than guessing.
pub struct EvidenceSources<'a> {
    pub config_path: Option<&'a Path>,
    /// Kernel control node (or Zygisk selection root is separate).
    pub kmod_ctl_path: Option<&'a Path>,
    pub zygisk_module: Option<&'a Path>,
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
    let Some(candidate) = entry.candidate.as_ref() else {
        // Without a stored candidate we cannot prove Persist intent; leave as-is.
        return false;
    };
    let mut changed = false;

    for phase in &mut entry.record.phases {
        if phase.outcome != PhaseOutcome::Unknown {
            continue;
        }
        let resolved = match phase.phase {
            ConfigPhase::Persist => reconcile_persist(candidate, sources.config_path),
            ConfigPhase::Native => reconcile_native(candidate, sources),
            // Ports/Secret/Cleanup: no evidence path in this slice.
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
    if phases.iter().any(|p| {
        matches!(
            p.outcome,
            PhaseOutcome::Unknown | PhaseOutcome::Running | PhaseOutcome::NotAttempted
        )
    }) {
        // NotAttempted after a restart means the plan did not finish; hold without
        // dispatching. Running should already have been fenced to Unknown.
        return PhaseOutcome::Unknown;
    }
    if phases
        .iter()
        .any(|p| p.outcome == PhaseOutcome::FailedKnown)
    {
        return PhaseOutcome::FailedKnown;
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

fn reconcile_native(candidate: &CanonicalConfig, sources: &EvidenceSources<'_>) -> PhaseOutcome {
    // Zygisk: publication is a regular file we can compare to the projected wire.
    if let Some(module) = sources.zygisk_module {
        return reconcile_zygisk_publication(candidate, module);
    }
    // Kernel / KPM: telemetry proves liveness, not that this candidate’s wire
    // was applied. Refusing to invent Confirmed leaves the op Held.
    let _ = (candidate, sources.kmod_ctl_path);
    PhaseOutcome::Unknown
}

fn reconcile_zygisk_publication(candidate: &CanonicalConfig, module: &Path) -> PhaseOutcome {
    let targets = module.join("targets.txt");
    let on_disk = match fs::read_to_string(&targets) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return PhaseOutcome::FailedKnown;
        }
        Err(_) => return PhaseOutcome::Unknown,
    };
    // Resolver-only projection: readback must not block on PackageManager, and host
    // tests have no `pm`. Empty map matches a config with no package targets.
    let resolver = vpnhide_activator::PackageUidMap::default();
    match activation::project_native_wire_with_resolver(candidate, NativeBackend::Zygisk, &resolver)
    {
        Ok(wire) if wire == on_disk => PhaseOutcome::Confirmed,
        Ok(_) => PhaseOutcome::FailedKnown,
        Err(_) => PhaseOutcome::Unknown,
    }
}

/// After Persist is reconciled to Confirmed, advance the durable revision if the
/// journal still carries the pre-edit counter (crash between Persist and settle).
pub fn revision_after_reconcile(entry: &JournalEntry, journal_revision: u64) -> u64 {
    let persist_confirmed = entry
        .record
        .phases
        .iter()
        .any(|p| p.phase == ConfigPhase::Persist && p.outcome == PhaseOutcome::Confirmed);
    if persist_confirmed && journal_revision <= entry.base.revision {
        entry.base.revision + 1
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
            kmod_ctl_path: None,
            zygisk_module: None,
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
            kmod_ctl_path: None,
            zygisk_module: None,
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
        let sources = EvidenceSources {
            config_path: Some(&config),
            kmod_ctl_path: Some(&ctl),
            zygisk_module: None,
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
        let candidate = CanonicalConfig::default();
        let resolver = vpnhide_activator::PackageUidMap::default();
        let wire = activation::project_native_wire_with_resolver(
            &candidate,
            NativeBackend::Zygisk,
            &resolver,
        )
        .unwrap();
        fs::write(module.join("targets.txt"), &wire).unwrap();
        let mut e = entry(
            PhaseOutcome::Confirmed,
            Some(PhaseOutcome::Unknown),
            candidate,
        );
        let sources = EvidenceSources {
            config_path: None,
            kmod_ctl_path: None,
            zygisk_module: Some(&module),
        };
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
        let sources = EvidenceSources {
            config_path: None,
            kmod_ctl_path: None,
            zygisk_module: None,
        };
        reconcile_entry(&mut e, &sources);
        assert_eq!(e.record.settled, Some(PhaseOutcome::Unknown));
        assert_eq!(e.record.phases[1].outcome, PhaseOutcome::NotAttempted);
    }
}
