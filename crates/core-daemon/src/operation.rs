//! The operation lifecycle: an accepted edit runs an ordered **phase plan**, each phase
//! producing a [`PhaseOutcome`], and the whole settles into a durable [`OperationRecord`]
//! (core-2.0 §5). This is the core-side port of the execution core of the app's
//! `reduceConfigOperation` — deliberately *without* the app-only concerns that migration
//! leaves in Kotlin (drafts, UI-edit conflicts, bridge-source, defensive collection copies;
//! §3/§7): the core receives an already-resolved typed edit.
//!
//! # Scope (minimal C)
//!
//! Phases run **in order**; the first non-`Confirmed` outcome stops the plan and the
//! remaining phases stay `NotAttempted` (so a failed activation still *exposes* the ports it
//! never reached). `Persist` is the only phase that changes the durable config, and only a
//! confirmed `Persist` commits the candidate — a later phase failing keeps what `Persist`
//! already wrote (the app's "failed activation retains the persisted switch" rule).
//!
//! Recovery (an `Unknown` outcome → readback → resolve/Held) is **not** built yet: today the
//! only real phase is `Persist`, a local atomic write that is `Confirmed` or errors, never
//! `Unknown`. It lands with step D, when a backend-activation phase (`Native`/`Ports`, over
//! protocol.md) can actually return `Unknown`. The [`PhaseRunner`] seam is where those
//! executors plug in; tests already drive every outcome through a fake runner.

use std::path::PathBuf;

use vpnhide_core_rpc::{CanonicalConfig, ConfigPhase, PhaseOutcome, PhaseRecord};

use crate::config_file;

/// Runs one phase of an operation against the candidate config and reports its outcome.
/// The real runner performs the side effect (Persist writes the durable file); D adds the
/// backend-activation runners. Tests inject a fake that returns scripted outcomes.
pub trait PhaseRunner {
    fn run(&self, phase: ConfigPhase, candidate: &CanonicalConfig) -> PhaseOutcome;
}

/// The phase plan for an edit, in [`ConfigPhase`] order. `Persist` always; `Native` when the
/// candidate has a native target *and* a control node is configured to deliver to (step D —
/// the built-in/kmod family). `Ports` and the coupled `Secret`/`Cleanup` join later. The plan
/// reflects what is actually runnable, so a planned phase never settles `NotAttempted`.
pub fn plan_for(native: bool) -> Vec<ConfigPhase> {
    if native {
        vec![ConfigPhase::Persist, ConfigPhase::Native]
    } else {
        vec![ConfigPhase::Persist]
    }
}

/// What running a plan produced: the per-phase record, the settled outcome, and whether the
/// candidate was durably persisted (so the caller commits it into the in-memory store).
pub struct PlanOutcome {
    pub phases: Vec<PhaseRecord>,
    pub settled: PhaseOutcome,
    /// True iff `Persist` confirmed *a real change* — the caller should commit the candidate
    /// and bump the revision. A no-op edit persists nothing and leaves this false.
    pub persisted: bool,
}

/// Run `plan` in order against `candidate`. `changed` is whether the candidate differs from
/// the current durable config: a no-op edit's `Persist` is trivially satisfied (the durable
/// config already equals the candidate) and performs no write.
pub fn run_plan(
    plan: &[ConfigPhase],
    candidate: &CanonicalConfig,
    changed: bool,
    runner: &dyn PhaseRunner,
) -> PlanOutcome {
    let mut phases = Vec::with_capacity(plan.len());
    let mut settled = PhaseOutcome::Confirmed;
    let mut persisted = false;

    for (i, &phase) in plan.iter().enumerate() {
        // A no-op edit's Persist needs no write — the durable config already matches.
        let outcome = if phase == ConfigPhase::Persist && !changed {
            PhaseOutcome::Confirmed
        } else {
            runner.run(phase, candidate)
        };
        phases.push(PhaseRecord { phase, outcome });

        if phase == ConfigPhase::Persist && outcome == PhaseOutcome::Confirmed {
            persisted = changed;
        }

        if outcome != PhaseOutcome::Confirmed {
            // Stop the plan; everything after a non-confirmed phase stays NotAttempted so the
            // record still shows what was never reached.
            settled = outcome;
            for &rest in &plan[i + 1..] {
                phases.push(PhaseRecord { phase: rest, outcome: PhaseOutcome::NotAttempted });
            }
            break;
        }
    }

    PlanOutcome { phases, settled, persisted }
}

/// The production runner. `Persist` writes the durable config atomically (no path → confirmed
/// no-op, for host tests). `Native` projects the config to the built-in control-v2 wire and
/// delivers it to the configured control node (step D); it is only ever planned when a node
/// path is set, so a `Native` with no path is a bug, reported `NotAttempted`. `Ports`/`Secret`/
/// `Cleanup` land later.
pub struct RealPhaseRunner {
    pub config_path: Option<PathBuf>,
    /// The kernel control node to deliver `Native` to. `None` disables native delivery
    /// entirely (the dev sandbox default — it never touches the real `/proc/vpnhide_ctl`).
    pub kmod_ctl_path: Option<PathBuf>,
}

impl PhaseRunner for RealPhaseRunner {
    fn run(&self, phase: ConfigPhase, candidate: &CanonicalConfig) -> PhaseOutcome {
        match phase {
            ConfigPhase::Persist => match &self.config_path {
                Some(path) => match config_file::write_atomic(path, candidate) {
                    Ok(()) => PhaseOutcome::Confirmed,
                    Err(_) => PhaseOutcome::FailedKnown,
                },
                None => PhaseOutcome::Confirmed,
            },
            ConfigPhase::Native => match &self.kmod_ctl_path {
                Some(path) => match crate::activation::project_builtin_wire(candidate)
                    .and_then(|wire| crate::activation::deliver_builtin(path, &wire))
                {
                    Ok(()) => PhaseOutcome::Confirmed,
                    Err(_) => PhaseOutcome::FailedKnown,
                },
                None => PhaseOutcome::NotAttempted,
            },
            // Ports / Secret / Cleanup: implemented later in step D.
            _ => PhaseOutcome::NotAttempted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// A runner that returns scripted outcomes per phase and records what it was asked to run
    /// — the core-side equivalent of the app tests driving `PhaseFinished(outcome)`.
    struct FakeRunner {
        outcomes: HashMap<ConfigPhase, PhaseOutcome>,
        ran: RefCell<Vec<ConfigPhase>>,
    }

    impl FakeRunner {
        fn new(pairs: &[(ConfigPhase, PhaseOutcome)]) -> Self {
            FakeRunner {
                outcomes: pairs.iter().copied().collect(),
                ran: RefCell::new(Vec::new()),
            }
        }
    }

    impl PhaseRunner for FakeRunner {
        fn run(&self, phase: ConfigPhase, _candidate: &CanonicalConfig) -> PhaseOutcome {
            self.ran.borrow_mut().push(phase);
            self.outcomes.get(&phase).copied().unwrap_or(PhaseOutcome::NotAttempted)
        }
    }

    fn record(out: &PlanOutcome, phase: ConfigPhase) -> PhaseOutcome {
        out.phases.iter().find(|p| p.phase == phase).unwrap().outcome
    }

    #[test]
    fn single_persist_change_commits() {
        let runner = FakeRunner::new(&[(ConfigPhase::Persist, PhaseOutcome::Confirmed)]);
        let out = run_plan(&[ConfigPhase::Persist], &CanonicalConfig::default(), true, &runner);
        assert_eq!(out.settled, PhaseOutcome::Confirmed);
        assert!(out.persisted);
        assert_eq!(*runner.ran.borrow(), vec![ConfigPhase::Persist]);
    }

    #[test]
    fn no_op_persist_does_not_write_or_commit() {
        // changed=false: Persist is trivially confirmed and the runner is never called.
        let runner = FakeRunner::new(&[(ConfigPhase::Persist, PhaseOutcome::FailedKnown)]);
        let out = run_plan(&[ConfigPhase::Persist], &CanonicalConfig::default(), false, &runner);
        assert_eq!(out.settled, PhaseOutcome::Confirmed);
        assert!(!out.persisted);
        assert!(runner.ran.borrow().is_empty(), "no-op must not invoke the persist side effect");
    }

    #[test]
    fn all_phases_confirmed_settles_confirmed() {
        let runner = FakeRunner::new(&[
            (ConfigPhase::Persist, PhaseOutcome::Confirmed),
            (ConfigPhase::Native, PhaseOutcome::Confirmed),
            (ConfigPhase::Ports, PhaseOutcome::Confirmed),
        ]);
        let plan = [ConfigPhase::Persist, ConfigPhase::Native, ConfigPhase::Ports];
        let out = run_plan(&plan, &CanonicalConfig::default(), true, &runner);
        assert_eq!(out.settled, PhaseOutcome::Confirmed);
        assert!(out.persisted);
    }

    #[test]
    fn failed_activation_retains_persist_and_exposes_unattempted_ports() {
        // Persist confirmed (config committed), Native fails, Ports never reached.
        let runner = FakeRunner::new(&[
            (ConfigPhase::Persist, PhaseOutcome::Confirmed),
            (ConfigPhase::Native, PhaseOutcome::FailedKnown),
        ]);
        let plan = [ConfigPhase::Persist, ConfigPhase::Native, ConfigPhase::Ports];
        let out = run_plan(&plan, &CanonicalConfig::default(), true, &runner);
        assert_eq!(out.settled, PhaseOutcome::FailedKnown);
        assert!(out.persisted, "a persisted config survives a later phase failure");
        assert_eq!(record(&out, ConfigPhase::Persist), PhaseOutcome::Confirmed);
        assert_eq!(record(&out, ConfigPhase::Native), PhaseOutcome::FailedKnown);
        assert_eq!(record(&out, ConfigPhase::Ports), PhaseOutcome::NotAttempted);
        assert_eq!(*runner.ran.borrow(), vec![ConfigPhase::Persist, ConfigPhase::Native]);
    }

    #[test]
    fn failed_persist_commits_nothing_and_skips_the_rest() {
        let runner = FakeRunner::new(&[
            (ConfigPhase::Persist, PhaseOutcome::FailedKnown),
            (ConfigPhase::Native, PhaseOutcome::Confirmed),
        ]);
        let plan = [ConfigPhase::Persist, ConfigPhase::Native];
        let out = run_plan(&plan, &CanonicalConfig::default(), true, &runner);
        assert_eq!(out.settled, PhaseOutcome::FailedKnown);
        assert!(!out.persisted);
        assert_eq!(record(&out, ConfigPhase::Native), PhaseOutcome::NotAttempted);
        assert_eq!(*runner.ran.borrow(), vec![ConfigPhase::Persist], "no dispatch after a failed persist");
    }

    #[test]
    fn real_runner_persists_to_disk_and_reports_none_path_confirmed() {
        // No path: a confirmed no-op (host-test semantics).
        let none = RealPhaseRunner { config_path: None, kmod_ctl_path: None };
        assert_eq!(
            none.run(ConfigPhase::Persist, &CanonicalConfig::default()),
            PhaseOutcome::Confirmed,
        );
        // With a path: the file is written and the phase confirms.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vpnhide_config.json");
        let real = RealPhaseRunner { config_path: Some(path.clone()), kmod_ctl_path: None };
        assert_eq!(
            real.run(ConfigPhase::Persist, &CanonicalConfig::default()),
            PhaseOutcome::Confirmed,
        );
        assert!(path.exists());
        // An unimplemented backend phase is NotAttempted, not silently confirmed.
        assert_eq!(
            real.run(ConfigPhase::Native, &CanonicalConfig::default()),
            PhaseOutcome::NotAttempted,
        );
    }
}
