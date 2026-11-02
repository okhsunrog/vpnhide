//! Separate bounded loader-experiment receipts. Neither receipt pruning nor an
//! explicit config repair can grant a second syscall after an uncertain attempt.

use serde::{Deserialize, Serialize};
use vpnhide_core_rpc::ConfigPhase;

use super::{Journal, valid_sha256};
use crate::payloads::{Catalog, KMIS, Payload, PayloadKind, kmi};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreflightPlan {
    pub boot_id: String,
    pub manifest_sha256: String,
    pub series: kmi::LinuxSeries,
    pub candidates: Vec<Payload>,
}

impl PreflightPlan {
    /// Must run on the verified catalog before acquiring the writer. Merely
    /// making a plan never authorizes dispatch or proves private ABI coverage.
    pub fn from_catalog(catalog: &Catalog, release: &str, boot_id: String) -> Result<Self, String> {
        use sha2::{Digest, Sha256};
        let resolution = kmi::resolve(&catalog.manifest, release).map_err(|e| e.to_string())?;
        let series = resolution.series.ok_or("unknown Linux series")?;
        let plan = Self {
            boot_id,
            manifest_sha256: format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&catalog.manifest).map_err(|e| e.to_string())?)
            ),
            series,
            candidates: resolution
                .candidates
                .into_iter()
                .map(|c| catalog.payloads[c.payload_index].metadata.clone())
                .collect(),
        };
        if !plan.valid() {
            return Err("invalid or unsupported preflight plan".into());
        }
        Ok(plan)
    }

    pub(super) fn valid(&self) -> bool {
        if self.boot_id.is_empty()
            || self.boot_id.len() > 128
            || !valid_sha256(&self.manifest_sha256)
            || self.candidates.is_empty()
            || self.candidates.len() > KMIS.len()
        {
            return false;
        }
        let mut identities = std::collections::BTreeSet::new();
        let mut hashes = std::collections::BTreeSet::new();
        self.candidates.iter().all(|p| {
            let PayloadKind::Kmod { kmi } = &p.implementation else {
                return false;
            };
            KMIS.contains(&kmi.as_str())
                && kmi
                    .split_once('-')
                    .is_some_and(|(_, s)| kmi::series(s) == Some(self.series))
                && p.preflight_protocol == Some(1)
                && p.control_version == 2
                && p.bytes > 0
                && p.bytes <= crate::payloads::MAX_PAYLOAD_BYTES
                && valid_sha256(&p.sha256)
                && p.source_commit.len() == 40
                && p.source_commit.bytes().all(|b| b.is_ascii_hexdigit())
                && !p.version.is_empty()
                && p.version.len() <= 128
                && !p.version.chars().any(char::is_control)
                && identities.insert(kmi)
                && hashes.insert(&p.sha256)
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreflightLedger {
    pub plan: PreflightPlan,
    pub attempts: Vec<PreflightAttempt>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreflightAttempt {
    pub sha256: String,
    pub state: PreflightState,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum PreflightState {
    Running,
    Finished(PreflightCompletion),
    Unknown { errno: Option<i32> },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum PreflightCompletion {
    /// Exact early-init sentinel AND verified clean postconditions. Not ABI proof.
    CleanSentinel,
    /// A loader rejection with clean postconditions, scoped to this candidate.
    CandidateRejected { errno: i32 },
    /// Signature/policy restriction applies to every candidate. No next syscall.
    LoaderBlocked { errno: i32 },
}

/// Loader evidence only. Even one reached sentinel still needs independent
/// private-ABI/ownership/full-activation evidence; it is not an activation permit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoaderEvidence {
    Pending,
    Uncertain,
    Blocked,
    NoSentinel,
    OneSentinel { sha256: String },
    MultipleSentinels { sha256: Vec<String> },
}

impl PreflightLedger {
    pub fn evidence(&self) -> LoaderEvidence {
        if self.uncertain() {
            return LoaderEvidence::Uncertain;
        }
        if self.attempts.iter().any(|a| {
            matches!(
                a.state,
                PreflightState::Finished(PreflightCompletion::LoaderBlocked { .. })
            )
        }) {
            return LoaderEvidence::Blocked;
        }
        if self.attempts.len() != self.plan.candidates.len() {
            return LoaderEvidence::Pending;
        }
        let successes: Vec<_> = self
            .attempts
            .iter()
            .filter(|a| a.state == PreflightState::Finished(PreflightCompletion::CleanSentinel))
            .map(|a| a.sha256.clone())
            .collect();
        match successes.as_slice() {
            [] => LoaderEvidence::NoSentinel,
            [sha256] => LoaderEvidence::OneSentinel {
                sha256: sha256.clone(),
            },
            _ => LoaderEvidence::MultipleSentinels { sha256: successes },
        }
    }
    pub(super) fn valid(&self) -> bool {
        if !self.plan.valid() || self.attempts.len() > self.plan.candidates.len() {
            return false;
        }
        let mut hashes = std::collections::BTreeSet::new();
        self.attempts.iter().enumerate().all(|(index, a)| {
            let last = index + 1 == self.attempts.len();
            self.plan.candidates.iter().any(|p| p.sha256 == a.sha256)
                && hashes.insert(&a.sha256)
                && match a.state {
                    PreflightState::Finished(PreflightCompletion::CandidateRejected { errno }) => {
                        errno == libc::ENOEXEC
                    }
                    PreflightState::Finished(PreflightCompletion::LoaderBlocked { errno }) => {
                        last && matches!(
                            errno,
                            libc::EPERM
                                | libc::EACCES
                                | libc::ENOSYS
                                | libc::ENOKEY
                                | libc::EKEYREJECTED
                        )
                    }
                    PreflightState::Unknown { errno: Some(errno) } => {
                        last && errno > 0 && errno <= 4095
                    }
                    PreflightState::Running | PreflightState::Unknown { errno: None } => last,
                    _ => true,
                }
        })
    }
    pub fn uncertain(&self) -> bool {
        self.attempts.iter().any(|a| {
            matches!(
                a.state,
                PreflightState::Running | PreflightState::Unknown { .. }
            )
        })
    }
    pub fn fence_unresolved(&mut self) {
        for a in &mut self.attempts {
            if a.state == PreflightState::Running {
                a.state = PreflightState::Unknown { errno: None };
            }
        }
    }
}

impl Journal {
    pub fn preflight_uncertain(&self) -> bool {
        self.kmod_preflights
            .as_ref()
            .is_some_and(PreflightLedger::uncertain)
    }
    pub fn native_lifecycle_uncertain(&self) -> bool {
        self.native_startup_uncertain() || self.preflight_uncertain() || self.revocation_uncertain()
    }

    /// Call only under the writer, then sync the candidate journal BEFORE any
    /// syscall. A completed entry is queried, never dispatched again.
    pub fn begin_preflight(&mut self, plan: &PreflightPlan, sha256: &str) -> Result<(), String> {
        if !plan.valid() || !plan.candidates.iter().any(|p| p.sha256 == sha256) {
            return Err("invalid preflight payload identity".into());
        }
        if self.held_resources().iter().any(|p| {
            matches!(
                p,
                ConfigPhase::Persist | ConfigPhase::Native | ConfigPhase::Secret
            )
        }) || self.zygisk_exposed_in(&plan.boot_id)
            || self
                .native_load_budget
                .as_ref()
                .is_some_and(|b| b.boot_id == plan.boot_id && !b.attempted.is_empty())
        {
            return Err("native ownership/effect uncertainty blocks preflight".into());
        }
        if let Some(ledger) = &self.kmod_preflights {
            if ledger.uncertain() {
                return Err("unresolved preflight requires explicit reboot reconciliation".into());
            }
            if ledger.plan.boot_id == plan.boot_id {
                if ledger.plan != *plan {
                    return Err("preflight plan already frozen for this boot".into());
                }
                if ledger.attempts.iter().any(|a| {
                    a.sha256 == sha256
                        || matches!(
                            a.state,
                            PreflightState::Finished(PreflightCompletion::LoaderBlocked { .. })
                        )
                }) {
                    return Err("preflight budget consumed or loader blocked".into());
                }
            }
        }
        if self
            .kmod_preflights
            .as_ref()
            .is_none_or(|l| l.plan.boot_id != plan.boot_id)
        {
            self.kmod_preflights = Some(PreflightLedger {
                plan: plan.clone(),
                attempts: Vec::new(),
            });
        }
        let ledger = self
            .kmod_preflights
            .as_mut()
            .expect("preflight plan installed");
        if ledger.attempts.len() >= plan.candidates.len() {
            return Err("preflight budget exhausted".into());
        }
        ledger.attempts.push(PreflightAttempt {
            sha256: sha256.into(),
            state: PreflightState::Running,
        });
        Ok(())
    }

    pub fn complete_preflight(
        &mut self,
        sha256: &str,
        state: PreflightState,
    ) -> Result<(), String> {
        if state == PreflightState::Running {
            return Err("preflight completion is still running".into());
        }
        let Some(ledger) = &mut self.kmod_preflights else {
            return Err("no preflight intent".into());
        };
        let Some(attempt) = ledger.attempts.iter_mut().find(|a| a.sha256 == sha256) else {
            return Err("unknown preflight payload".into());
        };
        if attempt.state != PreflightState::Running {
            return Err("preflight receipt is immutable".into());
        }
        let prior = std::mem::replace(&mut attempt.state, state);
        if !ledger.valid() {
            ledger
                .attempts
                .iter_mut()
                .find(|a| a.sha256 == sha256)
                .expect("same attempt")
                .state = prior;
            return Err("invalid preflight result".into());
        }
        Ok(())
    }
}

/// Effect injection keeps the durable boundary executable on hosts. Production
/// dispatch remains disabled until the final-payload acceptance matrix passes.
/// The caller must supply a pinned, capability-verified payload and postcondition
/// oracle; no pathname, insmod exit code or timeout may become CleanSentinel.
pub fn run_preflight_with(
    path: &std::path::Path,
    journal: &mut Journal,
    plan: &PreflightPlan,
    sha256: &str,
    effect: impl FnOnce() -> PreflightState,
) -> Result<PreflightState, String> {
    let mut running = journal.clone();
    running.begin_preflight(plan, sha256)?;
    if let Err(error) = super::write_atomic(path, &running) {
        // Even a post-rename sync error cannot grant another in-process attempt.
        running.fence_unresolved();
        *journal = running;
        return Err(format!(
            "preflight intent storage uncertain; no syscall dispatched: {error}"
        ));
    }
    *journal = running;
    let state = effect();
    let mut completed = journal.clone();
    completed.complete_preflight(sha256, state.clone())?;
    if let Err(error) = super::write_atomic(path, &completed) {
        // The durable receipt may still be Running. Keep the same hold in memory.
        journal.fence_unresolved();
        return Err(format!("preflight completion storage uncertain: {error}"));
    }
    *journal = completed;
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::{NativeLoadBudget, NativeQuarantine, NativeStartup};

    fn plan() -> PreflightPlan {
        let mut candidates: Vec<_> = ["android13-5.15", "android14-5.15"]
            .into_iter()
            .map(|kmi| Payload {
                implementation: PayloadKind::Kmod { kmi: kmi.into() },
                version: "test".into(),
                source_commit: "a".repeat(40),
                sha256: "a".repeat(64),
                bytes: 42,
                control_version: 2,
                preflight_protocol: Some(1),
            })
            .collect();
        for (index, p) in candidates.iter_mut().enumerate() {
            p.preflight_protocol = Some(1);
            p.sha256 = (index + 1).to_string().repeat(64);
        }
        PreflightPlan {
            boot_id: "boot".into(),
            manifest_sha256: "a".repeat(64),
            series: kmi::LinuxSeries {
                major: 5,
                minor: 15,
            },
            candidates,
        }
    }
    fn success() -> PreflightState {
        PreflightState::Finished(PreflightCompletion::CleanSentinel)
    }

    #[test]
    fn legacy_migration_preserves_all_existing_effect_holds_and_budgets() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("journal.json");
        for version in 1..=9 {
            let original = Journal {
                version,
                native_startup: Some(NativeStartup {
                    boot_id: "boot".into(),
                    config_revision: 19,
                    filesystem_hiding: false,
                    intent: None,
                    repair_op: None,
                    completed: None,
                }),
                native_load_budget: Some(NativeLoadBudget {
                    boot_id: "boot".into(),
                    attempted: vec![vpnhide_core_rpc::NativeBackend::Kmod],
                }),
                native_quarantine: vec![NativeQuarantine {
                    backend: vpnhide_core_rpc::NativeBackend::Kmod,
                    sha256: "b".repeat(64),
                }],
                zygisk_exposure_boot: Some("boot".into()),
                config_revision: 19,
                expired_holds: vec![ConfigPhase::Ports],
                ..Journal::default()
            };
            let mut value = serde_json::to_value(&original).unwrap();
            value.as_object_mut().unwrap().remove("kmod_preflights");
            std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
            let migrated = super::super::read(&path).unwrap();
            assert_eq!(migrated.version, 10);
            assert_eq!(migrated.native_startup, original.native_startup);
            assert_eq!(migrated.native_load_budget, original.native_load_budget);
            assert_eq!(migrated.native_quarantine, original.native_quarantine);
            assert_eq!(migrated.zygisk_exposure_boot, original.zygisk_exposure_boot);
            assert_eq!(migrated.config_revision, original.config_revision);
            assert_eq!(
                migrated.held_resources(),
                vec![ConfigPhase::Native, ConfigPhase::Ports]
            );
            assert!(migrated.kmod_preflights.is_none());
        }
    }

    #[test]
    fn independent_frozen_budget_and_immutable_receipts() {
        let plan = plan();
        let sha = &plan.candidates[0].sha256;
        let other = &plan.candidates[1].sha256;
        let mut journal = Journal::default();
        journal.begin_preflight(&plan, sha).unwrap();
        assert!(journal.native_load_budget.is_none());
        assert!(journal.has_unknown());
        assert!(journal.begin_preflight(&plan, other).is_err());
        journal.complete_preflight(sha, success()).unwrap();
        assert!(!journal.preflight_uncertain());
        assert!(
            journal
                .complete_preflight(sha, PreflightState::Unknown { errno: None })
                .is_err()
        );
        assert!(journal.begin_preflight(&plan, sha).is_err());
        let mut changed = plan.clone();
        changed.manifest_sha256 = "b".repeat(64);
        assert!(journal.begin_preflight(&changed, other).is_err());
        journal.begin_preflight(&plan, other).unwrap();
        journal
            .complete_preflight(
                other,
                PreflightState::Finished(PreflightCompletion::LoaderBlocked { errno: 1 }),
            )
            .unwrap();
        assert!(journal.begin_preflight(&plan, sha).is_err());
    }

    #[test]
    fn two_successful_probes_preserve_ambiguity_and_global_failure_stops_next() {
        let plan = plan();
        let mut journal = Journal::default();
        for p in &plan.candidates {
            journal.begin_preflight(&plan, &p.sha256).unwrap();
            journal.complete_preflight(&p.sha256, success()).unwrap();
        }
        assert!(
            matches!(journal.kmod_preflights.as_ref().unwrap().evidence(), LoaderEvidence::MultipleSentinels { sha256 } if sha256.len() == 2)
        );
        let mut blocked = Journal::default();
        let first = &plan.candidates[0].sha256;
        blocked.begin_preflight(&plan, first).unwrap();
        blocked
            .complete_preflight(
                first,
                PreflightState::Finished(PreflightCompletion::LoaderBlocked { errno: 129 }),
            )
            .unwrap();
        assert_eq!(
            blocked.kmod_preflights.as_ref().unwrap().evidence(),
            LoaderEvidence::Blocked
        );
        assert!(
            blocked
                .begin_preflight(&plan, &plan.candidates[1].sha256)
                .is_err()
        );
    }

    #[test]
    fn real_load_requires_complete_unique_matching_loader_evidence() {
        use crate::native_boot::prepared::{PayloadIdentity, TransitionAction, TransitionIntent};
        let plan = plan();
        let first = &plan.candidates[0].sha256;
        let second = &plan.candidates[1].sha256;
        let intent = TransitionIntent {
            boot_id: plan.boot_id.clone(),
            filesystem_hiding: false,
            action: TransitionAction::Load(vpnhide_core_rpc::NativeBackend::Kmod),
            payload: Some(PayloadIdentity {
                source: "module.ko".into(),
                sha256: first.clone(),
                bytes: 42,
            }),
            bundle: None,
            retire_zygisk: None,
            retire_directory_identity: None,
        };
        let mut journal = Journal::default();
        journal.begin_preflight(&plan, first).unwrap();
        journal.complete_preflight(first, success()).unwrap();
        assert!(journal.check_native_attempt(&intent, true).is_err());
        journal.begin_preflight(&plan, second).unwrap();
        let mut ambiguous = journal.clone();
        ambiguous.complete_preflight(second, success()).unwrap();
        assert!(ambiguous.check_native_attempt(&intent, true).is_err());
        let mut blocked = journal.clone();
        blocked
            .complete_preflight(
                second,
                PreflightState::Finished(PreflightCompletion::LoaderBlocked { errno: 1 }),
            )
            .unwrap();
        assert!(blocked.check_native_attempt(&intent, true).is_err());
        journal
            .complete_preflight(
                second,
                PreflightState::Finished(PreflightCompletion::CandidateRejected { errno: 8 }),
            )
            .unwrap();
        // This checks the ledger boundary only; Prepared still requires all
        // independent runtime, ownership, catalog and complete-policy gates.
        assert!(journal.check_native_attempt(&intent, true).is_ok());
        let mut wrong = intent.clone();
        wrong.payload.as_mut().unwrap().sha256 = second.clone();
        assert!(journal.check_native_attempt(&wrong, true).is_err());
        assert!(journal.native_load_budget.is_none());
    }

    #[test]
    fn restart_unknown_survives_pruning_repairs_and_boot_changes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("journal.json");
        let plan = plan();
        let sha = &plan.candidates[0].sha256;
        let mut journal = Journal::default();
        journal.begin_preflight(&plan, sha).unwrap();
        super::super::write_atomic(&path, &journal).unwrap();
        let mut journal = super::super::read(&path).unwrap();
        journal.fence_unresolved();
        assert_eq!(
            journal.kmod_preflights.as_ref().unwrap().attempts[0].state,
            PreflightState::Unknown { errno: None }
        );
        journal.retain_settled = 0;
        journal.prune();
        for _ in 0..32 {
            journal.replace_native_startup(NativeStartup {
                boot_id: "new-boot".into(),
                config_revision: 0,
                filesystem_hiding: false,
                intent: None,
                repair_op: None,
                completed: None,
            });
        }
        assert!(journal.preflight_uncertain());
        let mut new_boot = plan.clone();
        new_boot.boot_id = "new-boot".into();
        assert!(journal.begin_preflight(&new_boot, sha).is_err());
        let intent = crate::native_boot::prepared::TransitionIntent {
            boot_id: "new-boot".into(),
            filesystem_hiding: false,
            action: crate::native_boot::prepared::TransitionAction::Adopt(
                vpnhide_core_rpc::NativeBackend::Builtin,
            ),
            payload: None,
            bundle: None,
            retire_zygisk: None,
            retire_directory_identity: None,
        };
        for explicit in [false, true] {
            assert!(journal.check_native_attempt(&intent, explicit).is_err());
        }
        assert!(journal.held_resources().contains(&ConfigPhase::Native));
        assert!(journal.complete_preflight(sha, success()).is_err());
    }

    #[test]
    fn intent_write_errors_never_dispatch_and_completion_errors_keep_hold() {
        let plan = plan();
        let sha = &plan.candidates[0].sha256;
        for (skip, after_rename, calls) in
            [(1, false, 0), (1, true, 0), (2, false, 1), (2, true, 1)]
        {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("journal.json");
            let mut journal = Journal::default();
            let count = std::cell::Cell::new(0);
            super::super::fail_write(skip, after_rename);
            assert!(
                run_preflight_with(&path, &mut journal, &plan, sha, || {
                    count.set(count.get() + 1);
                    success()
                })
                .is_err()
            );
            assert_eq!(count.get(), calls);
            assert!(journal.preflight_uncertain());
            assert!(journal.begin_preflight(&plan, sha).is_err());
            if path.exists() {
                let mut recovered = super::super::read(&path).unwrap();
                recovered.fence_unresolved();
                // If completion rename happened, disk has a complete receipt;
                // otherwise it retains the hold. Neither permits a second call.
                assert!(recovered.begin_preflight(&plan, sha).is_err());
            }
        }
    }

    #[test]
    fn malformed_ledger_never_resets_or_rewrites_journal() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("journal.json");
        let plan = plan();
        let mut journal = Journal::default();
        journal
            .begin_preflight(&plan, &plan.candidates[0].sha256)
            .unwrap();
        let original = serde_json::to_value(journal).unwrap();
        for change in 0..8 {
            let mut corrupt = original.clone();
            let ledger = &mut corrupt["kmod_preflights"];
            match change {
                0 => ledger["plan"]["candidates"][0]["preflight_protocol"] = 2.into(),
                1 => ledger["plan"]["boot_id"] = "".into(),
                2 => ledger["attempts"][0]["sha256"] = "c".repeat(64).into(),
                3 => ledger["plan"]["series"]["minor"] = 10.into(),
                4 => {
                    let duplicate = ledger["attempts"][0].clone();
                    ledger["attempts"].as_array_mut().unwrap().push(duplicate);
                }
                5 | 6 => {
                    let second = serde_json::json!({"sha256": plan.candidates[1].sha256, "state": "Running"});
                    ledger["attempts"].as_array_mut().unwrap().push(second);
                    if change == 6 {
                        ledger["attempts"][0]["state"] =
                            serde_json::json!({"Finished":{"LoaderBlocked":{"errno":1}}});
                    }
                }
                _ => {
                    ledger["attempts"][0]["state"] =
                        serde_json::json!({"Finished":{"CandidateRejected":{"errno":4}}})
                }
            }
            let bytes = serde_json::to_vec(&corrupt).unwrap();
            std::fs::write(&path, &bytes).unwrap();
            assert!(super::super::read(&path).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }

    #[test]
    fn core_startup_debug_capture_and_explicit_repair_respect_preflight_hold() {
        use crate::{CoreState, Publisher};
        use vpnhide_core_rpc::{
            BaseRef, CanonicalConfig, ConfigEdit, CoreCommand, DebugLeaseId, DebugLeaseOwner,
            NativeBootOutcome, StateSnapshot,
        };
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("journal.json");
        let config = root.path().join("config.json");
        crate::config_file::write_atomic(&config, &CanonicalConfig::default()).unwrap();
        let plan = plan();
        let mut journal = Journal::default();
        journal
            .begin_preflight(&plan, &plan.candidates[0].sha256)
            .unwrap();
        super::super::write_atomic(&path, &journal).unwrap();
        std::fs::write(root.path().join("boot-id"), "new-boot").unwrap();
        let state = CoreState::load_with_journal(
            Publisher::new(1, StateSnapshot::default()),
            config,
            Some(path.clone()),
            Some(root.path().join("ctl")),
        )
        .unwrap();
        state.start_native_boot(
            crate::native_boot::NativeEnv {
                ctl: root.path().join("ctl"),
                modules: None,
                kpm_lock: None,
                zygisk_module: None,
                boot_id_path: root.path().join("boot-id"),
                state_file: root.path().join("native.json"),
                legacy_root: None,
                apatch_kpms: root.path().join("no-apatch"),
                payload_override: None,
                loader: None,
            },
            1,
        );
        assert!(matches!(
            state.native_boot_status().unwrap().outcome,
            NativeBootOutcome::Unknown { .. }
        ));
        assert!(
            state
                .acquire_debug_lease(DebugLeaseId(1), DebugLeaseOwner::Forensic)
                .is_err()
        );
        let operation = state.issue_operation_id().unwrap();
        let submitted = state.submit_command(
            operation,
            CoreCommand::Edit {
                base: BaseRef {
                    revision: state.config_revision(),
                },
                edit: ConfigEdit::RepairActivation,
            },
        );
        if submitted.is_ok() {
            assert_ne!(
                state.get_operation_record(operation).unwrap().settled,
                Some(vpnhide_core_rpc::PhaseOutcome::Confirmed)
            );
        }
        assert!(state.has_unknown_hold());
        assert!(super::super::read(&path).unwrap().preflight_uncertain());
        assert!(!root.path().join("native.json").exists());
        state.shutdown();
    }
}
