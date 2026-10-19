//! The operation lifecycle: an accepted edit runs an ordered **phase plan**, each phase
//! producing a [`PhaseOutcome`], and the whole settles into a queryable operation record
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
//! Persist and Native are real phases. A KPM dispatch without completion evidence
//! returns Unknown and is not retried. Core blocks new edits while Unknown remains.
//! Durable operation records and restart fencing live in [`crate::journal`].

use std::path::PathBuf;

use vpnhide_core_rpc::{CanonicalConfig, ConfigPhase, PhaseOutcome, PhaseRecord};

use crate::config_file;

/// Runs one phase of an operation against the candidate config and reports its outcome.
/// The real runner performs the side effect (Persist writes the durable file); D adds the
/// backend-activation runners. Tests inject a fake that returns scripted outcomes.
pub trait PhaseRunner {
    fn run(&self, phase: ConfigPhase, candidate: &CanonicalConfig) -> PhaseOutcome;
}

/// Observes phase boundaries so a durable journal can sync evidence before/after effects.
/// `before` runs immediately prior to dispatching a phase that may mutate external state;
/// returning `Err` aborts that phase as the given outcome (typically `FailedKnown`).
pub trait PhaseHook {
    fn before(&mut self, phase: ConfigPhase) -> Result<(), PhaseOutcome>;
    fn after(&mut self, phase: ConfigPhase, outcome: PhaseOutcome);
}

/// What an admitted edit needs to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanKind {
    /// Durable config only (no native control node, or config unchanged).
    PersistOnly,
    /// Persist then activate (config changed and a control node is configured).
    PersistThenNative,
    /// Persist then Ports (config changed, Ports opted in, no native node).
    PersistThenPorts,
    /// Persist, Native, then Ports.
    PersistThenNativeThenPorts,
    /// Activation repair: re-apply native policy without rewriting canonical config.
    NativeOnly,
    /// Ports-only repair / re-apply without rewriting canonical config.
    PortsOnly,
    /// Repair with both Native and Ports (no Persist).
    NativeThenPorts,
}

/// The phase plan for an edit, in [`ConfigPhase`] order.
/// Coupled `Secret`/`Cleanup` join later. The plan reflects what is actually
/// runnable; a planned phase should not settle `NotAttempted` under normal wiring.
pub fn plan_for(native: bool) -> Vec<ConfigPhase> {
    plan_kind(if native {
        PlanKind::PersistThenNative
    } else {
        PlanKind::PersistOnly
    })
}

pub fn plan_kind(kind: PlanKind) -> Vec<ConfigPhase> {
    match kind {
        PlanKind::PersistOnly => vec![ConfigPhase::Persist],
        PlanKind::PersistThenNative => vec![ConfigPhase::Persist, ConfigPhase::Native],
        PlanKind::PersistThenPorts => vec![ConfigPhase::Persist, ConfigPhase::Ports],
        PlanKind::PersistThenNativeThenPorts => {
            vec![ConfigPhase::Persist, ConfigPhase::Native, ConfigPhase::Ports]
        }
        PlanKind::NativeOnly => vec![ConfigPhase::Native],
        PlanKind::PortsOnly => vec![ConfigPhase::Ports],
        PlanKind::NativeThenPorts => vec![ConfigPhase::Native, ConfigPhase::Ports],
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
    run_plan_hooked(plan, candidate, changed, runner, None)
}

/// Like [`run_plan`], but notifies `hook` before each dispatched effect and after each
/// outcome so a journal can make evidence durable at crash boundaries.
pub fn run_plan_hooked(
    plan: &[ConfigPhase],
    candidate: &CanonicalConfig,
    changed: bool,
    runner: &dyn PhaseRunner,
    mut hook: Option<&mut dyn PhaseHook>,
) -> PlanOutcome {
    let mut phases = Vec::with_capacity(plan.len());
    let mut settled = PhaseOutcome::Confirmed;
    let mut persisted = false;

    for (i, &phase) in plan.iter().enumerate() {
        // A no-op edit's Persist needs no write — the durable config already matches.
        let outcome = if phase == ConfigPhase::Persist && !changed {
            PhaseOutcome::Confirmed
        } else {
            let blocked = hook.as_mut().and_then(|h| h.before(phase).err());
            match blocked {
                Some(fail) => fail,
                None => {
                    let outcome = runner.run(phase, candidate);
                    if let Some(h) = hook.as_mut() {
                        h.after(phase, outcome);
                    }
                    outcome
                }
            }
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
                phases.push(PhaseRecord {
                    phase: rest,
                    outcome: PhaseOutcome::NotAttempted,
                });
            }
            break;
        }
    }

    PlanOutcome {
        phases,
        settled,
        persisted,
    }
}

/// The production runner. `Persist` writes the durable config atomically (no path → confirmed
/// no-op, for host tests). `Native` projects the config to the built-in control-v2 wire and
/// delivers it to the configured control node (step D); it is only ever planned when a node
/// path is set, so a `Native` with no path is a bug, reported `NotAttempted`. `Ports` applies
/// firewall rules when opted in via `VPNHIDE_PORTS`. `Secret`/`Cleanup` land later.
pub struct RealPhaseRunner {
    pub config_path: Option<PathBuf>,
    /// The kernel control node to deliver `Native` to. `None` disables native delivery
    /// entirely (the dev sandbox default — it never touches the real `/proc/vpnhide_ctl`).
    pub kmod_ctl_path: Option<PathBuf>,
    /// When true, `Ports` runs the real apply path (env `VPNHIDE_PORTS`).
    pub ports_enabled: bool,
    selected: std::cell::Cell<Option<vpnhide_core_rpc::NativeBackend>>,
    ports_report: std::cell::RefCell<Option<vpnhide_core_rpc::PortsFamilyReport>>,
}

impl RealPhaseRunner {
    pub fn new(config_path: Option<PathBuf>, kmod_ctl_path: Option<PathBuf>) -> Self {
        Self::new_with_ports(config_path, kmod_ctl_path, crate::ports::ports_enabled_from_env())
    }

    pub fn new_with_ports(
        config_path: Option<PathBuf>,
        kmod_ctl_path: Option<PathBuf>,
        ports_enabled: bool,
    ) -> Self {
        Self {
            config_path,
            kmod_ctl_path,
            ports_enabled,
            selected: std::cell::Cell::new(None),
            ports_report: std::cell::RefCell::new(None),
        }
    }

    pub fn selected(&self) -> Option<vpnhide_core_rpc::NativeBackend> {
        self.selected.get()
    }

    pub fn take_ports_report(&self) -> Option<vpnhide_core_rpc::PortsFamilyReport> {
        self.ports_report.borrow_mut().take()
    }
}

impl PhaseRunner for RealPhaseRunner {
    fn run(&self, phase: ConfigPhase, candidate: &CanonicalConfig) -> PhaseOutcome {
        match phase {
            ConfigPhase::Persist => match &self.config_path {
                Some(path) => {
                    // Best-effort fence: host sandboxes may lack /data/adb. A
                    // successful claim makes legacy activators no-op; failure
                    // must not block Persist itself.
                    if let Err(e) = crate::ownership::claim(crate::ownership::OwnedResource::Config)
                    {
                        eprintln!("core: config ownership claim failed: {e}");
                    }
                    match config_file::write_atomic(path, candidate) {
                        Ok(()) => PhaseOutcome::Confirmed,
                        Err(_) => PhaseOutcome::FailedKnown,
                    }
                }
                None => PhaseOutcome::Confirmed,
            },
            ConfigPhase::Native => match &self.kmod_ctl_path {
                // Select the live kernel owner, or load the explicitly enabled kmod
                // artifact when the node is definitely absent. Project for that
                // family and deliver (which re-verifies identity on the open node).
                Some(path) => {
                    let modules = crate::backend::kernel_modules_from_env();
                    let kpm_lock = crate::backend::kpm_lock_from_env();
                    let zygisk_module = crate::zygisk::module_from_env();
                    let backend = crate::backend::select_native(
                        path,
                        modules.as_deref(),
                        kpm_lock.as_deref(),
                        zygisk_module.as_deref(),
                        candidate
                            .settings
                            .optional_features
                            .contains(vpnhide_activator::OPTIONAL_FEATURE_FILESYSTEM_IFACE_PATHS),
                    );
                    match backend {
                        Ok(backend) => {
                            self.selected.set(Some(backend.family()));
                            match crate::activation::project_native_wire(
                                candidate,
                                backend.family(),
                            ) {
                                Ok(wire) => {
                                    if let Err(e) = crate::ownership::claim_before_native(
                                        backend.family(),
                                    ) {
                                        eprintln!("core: native ownership claim failed: {e}");
                                    }
                                    crate::activation::deliver_selected(path, &backend, &wire)
                                }
                                Err(error) => {
                                    eprintln!("core: native projection failed: {error}");
                                    PhaseOutcome::FailedKnown
                                }
                            }
                        }
                        Err(error) => {
                            eprintln!("core: native selection failed: {error}");
                            PhaseOutcome::FailedKnown
                        }
                    }
                }
                None => PhaseOutcome::NotAttempted,
            },
            ConfigPhase::Ports => {
                if !self.ports_enabled {
                    return PhaseOutcome::NotAttempted;
                }
                let (outcome, report) = crate::ports::run_ports_phase(candidate);
                *self.ports_report.borrow_mut() = Some(report);
                outcome
            }
            // Secret / Cleanup: later.
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
            self.outcomes
                .get(&phase)
                .copied()
                .unwrap_or(PhaseOutcome::NotAttempted)
        }
    }

    fn record(out: &PlanOutcome, phase: ConfigPhase) -> PhaseOutcome {
        out.phases
            .iter()
            .find(|p| p.phase == phase)
            .unwrap()
            .outcome
    }

    #[test]
    fn single_persist_change_commits() {
        let runner = FakeRunner::new(&[(ConfigPhase::Persist, PhaseOutcome::Confirmed)]);
        let out = run_plan(
            &[ConfigPhase::Persist],
            &CanonicalConfig::default(),
            true,
            &runner,
        );
        assert_eq!(out.settled, PhaseOutcome::Confirmed);
        assert!(out.persisted);
        assert_eq!(*runner.ran.borrow(), vec![ConfigPhase::Persist]);
    }

    #[test]
    fn no_op_persist_does_not_write_or_commit() {
        // changed=false: Persist is trivially confirmed and the runner is never called.
        let runner = FakeRunner::new(&[(ConfigPhase::Persist, PhaseOutcome::FailedKnown)]);
        let out = run_plan(
            &[ConfigPhase::Persist],
            &CanonicalConfig::default(),
            false,
            &runner,
        );
        assert_eq!(out.settled, PhaseOutcome::Confirmed);
        assert!(!out.persisted);
        assert!(
            runner.ran.borrow().is_empty(),
            "no-op must not invoke the persist side effect"
        );
    }

    #[test]
    fn all_phases_confirmed_settles_confirmed() {
        let runner = FakeRunner::new(&[
            (ConfigPhase::Persist, PhaseOutcome::Confirmed),
            (ConfigPhase::Native, PhaseOutcome::Confirmed),
            (ConfigPhase::Ports, PhaseOutcome::Confirmed),
        ]);
        let plan = [
            ConfigPhase::Persist,
            ConfigPhase::Native,
            ConfigPhase::Ports,
        ];
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
        let plan = [
            ConfigPhase::Persist,
            ConfigPhase::Native,
            ConfigPhase::Ports,
        ];
        let out = run_plan(&plan, &CanonicalConfig::default(), true, &runner);
        assert_eq!(out.settled, PhaseOutcome::FailedKnown);
        assert!(
            out.persisted,
            "a persisted config survives a later phase failure"
        );
        assert_eq!(record(&out, ConfigPhase::Persist), PhaseOutcome::Confirmed);
        assert_eq!(record(&out, ConfigPhase::Native), PhaseOutcome::FailedKnown);
        assert_eq!(record(&out, ConfigPhase::Ports), PhaseOutcome::NotAttempted);
        assert_eq!(
            *runner.ran.borrow(),
            vec![ConfigPhase::Persist, ConfigPhase::Native]
        );
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
        assert_eq!(
            record(&out, ConfigPhase::Native),
            PhaseOutcome::NotAttempted
        );
        assert_eq!(
            *runner.ran.borrow(),
            vec![ConfigPhase::Persist],
            "no dispatch after a failed persist"
        );
    }

    #[test]
    fn real_runner_persists_to_disk_and_reports_none_path_confirmed() {
        // No path: a confirmed no-op (host-test semantics).
        let none = RealPhaseRunner::new(None, None);
        assert_eq!(
            none.run(ConfigPhase::Persist, &CanonicalConfig::default()),
            PhaseOutcome::Confirmed,
        );
        // With a path: the file is written and the phase confirms.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vpnhide_config.json");
        let real = RealPhaseRunner::new(Some(path.clone()), None);
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

    #[test]
    fn persist_then_ports_plan_includes_ports() {
        assert_eq!(
            plan_kind(PlanKind::PersistThenPorts),
            vec![ConfigPhase::Persist, ConfigPhase::Ports]
        );
        assert_eq!(
            plan_kind(PlanKind::PersistThenNativeThenPorts),
            vec![ConfigPhase::Persist, ConfigPhase::Native, ConfigPhase::Ports]
        );
        assert_eq!(
            plan_kind(PlanKind::NativeThenPorts),
            vec![ConfigPhase::Native, ConfigPhase::Ports]
        );
    }

    #[test]
    fn real_runner_ports_disabled_is_not_attempted() {
        let runner = RealPhaseRunner::new_with_ports(None, None, false);
        assert_eq!(
            runner.run(ConfigPhase::Ports, &CanonicalConfig::default()),
            PhaseOutcome::NotAttempted
        );
        assert!(runner.take_ports_report().is_none());
    }
}
