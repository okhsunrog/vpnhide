//! Phase plans for core-owned commands. Persistence and coupled secret/cleanup
//! effects are dependencies; Native and Ports remain independent after known
//! failures. A confirmed Persist remains saved when a later apply fails.
//!
//! Effects are fenced by durable intent before dispatch. Returned Unknown receipts
//! remain historical facts. Explicit desired-state repair can supersede held
//! resources without replaying the interrupted command.

use std::path::PathBuf;

use vpnhide_core_rpc::{CanonicalConfig, ConfigPhase, PhaseOutcome, PhaseRecord};

use crate::config_file;

/// Runs one phase of an operation against the candidate config and reports its outcome.
/// The production runner owns canonical, backend, firewall and fixed root effects.
/// Tests inject a runner with scripted phase outcomes.
pub trait PhaseRunner {
    fn run(&self, phase: ConfigPhase, candidate: &CanonicalConfig) -> PhaseOutcome;
}

/// Observes phase boundaries so a durable journal can sync evidence before/after effects.
/// `before` runs immediately prior to dispatching a phase that may mutate external state;
/// returning `Err` aborts that phase as the given outcome (typically `FailedKnown`).
pub trait PhaseHook {
    fn before(&mut self, phase: ConfigPhase) -> Result<(), PhaseOutcome>;
    fn after(&mut self, phase: ConfigPhase, outcome: PhaseOutcome) -> Result<(), PhaseOutcome>;
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
/// These common plans compose desired-state persistence and independent runtime
/// resources. Command admission adds coupled Secret/Cleanup dependencies.
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
            vec![
                ConfigPhase::Persist,
                ConfigPhase::Native,
                ConfigPhase::Ports,
            ]
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
                    let mut outcome = runner.run(phase, candidate);
                    if let Some(h) = hook.as_mut() {
                        if let Err(uncertain) = h.after(phase, outcome) {
                            outcome = uncertain;
                        }
                    }
                    outcome
                }
            }
        };
        phases.push(PhaseRecord { phase, outcome });

        if phase == ConfigPhase::Persist && outcome == PhaseOutcome::Confirmed {
            persisted = changed;
        }

        // Persist is a dependency. Native and Ports are independent once desired
        // state is durable: a known Native failure must still permit Ports.
        if outcome != PhaseOutcome::Confirmed
            && (outcome == PhaseOutcome::Unknown
                || settled == PhaseOutcome::Confirmed
                || (outcome == PhaseOutcome::FailedKnown && settled != PhaseOutcome::Unknown))
        {
            settled = outcome;
        }
        if outcome != PhaseOutcome::Confirmed
            && !(outcome == PhaseOutcome::FailedKnown
                && matches!(phase, ConfigPhase::Native | ConfigPhase::Ports))
        {
            // Stop the plan; everything after a non-confirmed phase stays NotAttempted so the
            // record still shows what was never reached.
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

/// Runs persistence, the selected native backend, Ports and fixed typed effects.
/// Host tests may disable paths. Production never substitutes an arbitrary shell
/// command for one of these operations.
pub struct RealPhaseRunner {
    pub effect_paths: Option<crate::effects::EffectPaths>,
    pub effects: Vec<(ConfigPhase, vpnhide_core_rpc::RootEffect)>,
    pub reset_config: bool,
    pub remember_secret: bool,
    pub epoch: u64,
    pub config_path: Option<PathBuf>,
    /// The kernel control node to deliver `Native` to. `None` disables native delivery
    /// entirely (the dev sandbox default — it never touches the real `/proc/vpnhide_ctl`).
    pub kmod_ctl_path: Option<PathBuf>,
    /// When true, `Ports` runs the real apply path (env `VPNHIDE_PORTS`).
    pub ports_enabled: bool,
    /// This boot's fixed native decision. `None` = native management disabled.
    pub native: Option<NativeBootHandle>,
    /// Where a Ports pass came from (boot passes set this; edits default to `Edit`).
    pub ports_source: vpnhide_core_rpc::PortsSource,
    /// Whether netd's chains existed before a boot Ports pass.
    pub ports_netd_ready: Option<bool>,
    /// A wire admission already projected for this backend (the Save capacity check), so
    /// the Native phase does not resolve packages twice. Ignored for any other backend.
    pub native_wire: Option<(vpnhide_core_rpc::NativeBackend, String)>,
    prepared_native: std::cell::RefCell<
        Option<Result<crate::journal::NativeIntent, crate::activation::ProjectionError>>,
    >,
    selected: std::cell::Cell<Option<vpnhide_core_rpc::NativeBackend>>,
    ports_report: std::cell::RefCell<Option<vpnhide_core_rpc::PortsFamilyReport>>,
}

/// Shared handle to the per-boot native decision.
pub type NativeBootHandle = std::sync::Arc<std::sync::Mutex<crate::native_boot::NativeBoot>>;

impl RealPhaseRunner {
    pub fn new(config_path: Option<PathBuf>, kmod_ctl_path: Option<PathBuf>) -> Self {
        Self::new_with_ports(
            config_path,
            kmod_ctl_path,
            crate::ports::ports_enabled_from_env(),
        )
    }

    pub fn new_with_ports(
        config_path: Option<PathBuf>,
        kmod_ctl_path: Option<PathBuf>,
        ports_enabled: bool,
    ) -> Self {
        Self {
            effect_paths: None,
            effects: Vec::new(),
            reset_config: false,
            remember_secret: true,
            epoch: 0,
            config_path,
            kmod_ctl_path,
            ports_enabled,
            native: None,
            ports_source: vpnhide_core_rpc::PortsSource::Edit,
            ports_netd_ready: None,
            native_wire: None,
            prepared_native: std::cell::RefCell::new(None),
            selected: std::cell::Cell::new(None),
            ports_report: std::cell::RefCell::new(None),
        }
    }

    pub fn selected(&self) -> Option<vpnhide_core_rpc::NativeBackend> {
        self.selected.get()
    }

    /// Freeze the actual recipient and UID-resolved wire before journaling Native
    /// intent. The runner then dispatches these exact bytes, even if packages change.
    pub fn native_intent(
        &self,
        candidate: &CanonicalConfig,
    ) -> Option<crate::journal::NativeIntent> {
        if let Some(intent) = self.prepared_native.borrow().as_ref() {
            return intent.clone().ok();
        }
        let boot = self.native.as_ref()?.lock().expect("native boot");
        let backend = boot.selected().or_else(|| {
            boot.pending_authentication()
                .then_some(vpnhide_core_rpc::NativeBackend::Kpm)
        })?;
        let wire = match &self.native_wire {
            Some((for_backend, wire)) if *for_backend == backend => Ok(wire.clone()),
            _ => crate::activation::project_native_wire(candidate, backend),
        };
        let intent = wire.map(|wire| crate::journal::NativeIntent {
            backend,
            wire,
            zygisk_module: (backend == vpnhide_core_rpc::NativeBackend::Zygisk)
                .then(|| boot.env.module_dir(backend))
                .flatten(),
        });
        *self.prepared_native.borrow_mut() = Some(intent.clone());
        intent.ok()
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
                    if self.reset_config {
                        return match std::fs::remove_file(path) {
                            Ok(()) => match crate::config_file::sync_after_rename(
                                path.parent().unwrap_or_else(|| std::path::Path::new(".")),
                            ) {
                                Ok(()) => PhaseOutcome::Confirmed,
                                Err(_) => PhaseOutcome::Unknown,
                            },
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                                PhaseOutcome::Confirmed
                            }
                            Err(_) => PhaseOutcome::FailedKnown,
                        };
                    }
                    // Best-effort fence: host sandboxes may lack /data/adb. A
                    // claim records inventory ownership; failure
                    // must not block Persist itself.
                    if let Err(e) = crate::ownership::claim(crate::ownership::OwnedResource::Config)
                    {
                        eprintln!("core: config ownership claim failed: {e}");
                    }
                    match config_file::write_atomic(path, candidate) {
                        Ok(()) => PhaseOutcome::Confirmed,
                        Err(error) if config_file::is_post_rename_error(&error) => {
                            PhaseOutcome::Unknown
                        }
                        Err(_) => PhaseOutcome::FailedKnown,
                    }
                }
                None => PhaseOutcome::Confirmed,
            },
            ConfigPhase::Native => {
                // Selection happened once at boot; never re-select here. Deliver only
                // to that backend, or report why there is none.
                if self.kmod_ctl_path.is_none() {
                    return PhaseOutcome::NotAttempted;
                }
                let Some(native) = &self.native else {
                    return PhaseOutcome::NotAttempted;
                };
                let mut boot = native.lock().expect("native boot");
                // Authentication-triggered load is part of Native, whose exact
                // intent is already durable; Secret owns only key storage.
                let retried_authentication = boot.pending_authentication();
                if retried_authentication {
                    boot.retry_authentication(
                        self.epoch,
                        candidate
                            .settings
                            .optional_features
                            .contains(crate::config_edit::FILESYSTEM_FEATURE),
                    );
                }
                let Some(backend) = boot.selected() else {
                    return match &boot.status.outcome {
                        // Nothing native to configure: the phase is trivially satisfied.
                        vpnhide_core_rpc::NativeBootOutcome::NoBackend => PhaseOutcome::Confirmed,
                        vpnhide_core_rpc::NativeBootOutcome::LoadFailed { .. }
                            if retried_authentication =>
                        {
                            PhaseOutcome::Unknown
                        }
                        other => {
                            // Scan/auth/list refusals stopped before load or
                            // config dispatch. Their observation can remain
                            // unavailable while this apply failed definitively.
                            crate::alog::info(&format!(
                                "native: no config delivered this boot: {other:?}"
                            ));
                            PhaseOutcome::FailedKnown
                        }
                    };
                };
                self.selected.set(Some(backend));
                let prepared = self.prepared_native.borrow();
                let projected = match (&*prepared, &self.native_wire) {
                    (Some(Ok(intent)), _) if intent.backend == backend => Ok(intent.wire.clone()),
                    (Some(Err(error)), _) => Err(error.clone()),
                    (_, Some((for_backend, wire))) if *for_backend == backend => Ok(wire.clone()),
                    _ => crate::activation::project_native_wire(candidate, backend),
                };
                match projected {
                    Ok(wire) => {
                        if let Err(e) = crate::ownership::claim_before_native(backend) {
                            eprintln!("core: native ownership claim failed: {e}");
                        }
                        let (outcome, result) =
                            crate::activation::deliver_to_selected(&boot.env, backend, &wire);
                        boot.record_delivery(result);
                        outcome
                    }
                    // Refused whole: nothing reaches the backend, which keeps its
                    // previous policy. Never a truncated, partial target set.
                    Err(crate::activation::ProjectionError::Capacity(report)) => {
                        boot.record_capacity_exceeded(report);
                        PhaseOutcome::FailedKnown
                    }
                    Err(crate::activation::ProjectionError::Failed(error)) => {
                        boot.record_delivery(Err(format!("projection failed: {error}")));
                        PhaseOutcome::FailedKnown
                    }
                }
            }
            ConfigPhase::Ports => {
                if !self.ports_enabled {
                    return PhaseOutcome::NotAttempted;
                }
                let (outcome, report, detail) = crate::ports::run_ports_phase_detailed(candidate);
                crate::alog::info(&format!(
                    "ports: {:?} pass ipv4={:?} ipv6={:?} targets={}",
                    self.ports_source, report.ipv4, report.ipv6, report.target_count
                ));
                if let Some(native) = &self.native {
                    native.lock().expect("native boot").record_ports(
                        vpnhide_core_rpc::PortsStatus {
                            source: self.ports_source,
                            ipv4: report.ipv4,
                            ipv6: report.ipv6,
                            target_count: report.target_count,
                            netd_ready: self.ports_netd_ready,
                            detail,
                            at_ms: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_millis() as u64)
                                .unwrap_or(0),
                        },
                    );
                }
                *self.ports_report.borrow_mut() = Some(report);
                outcome
            }
            ConfigPhase::Secret | ConfigPhase::Cleanup => {
                let Some(paths) = &self.effect_paths else {
                    return PhaseOutcome::FailedKnown;
                };
                let Some((_, effect)) = self.effects.iter().find(|(p, _)| *p == phase) else {
                    return PhaseOutcome::NotAttempted;
                };
                let result = match effect {
                    vpnhide_core_rpc::RootEffect::SetSuperkey { superkey } => {
                        vpnhide_activator::set_session_superkey(Some(superkey.trim().to_owned()));
                        if self.remember_secret {
                            crate::effects::apply(paths, effect)
                        } else {
                            crate::effects::apply(
                                paths,
                                &vpnhide_core_rpc::RootEffect::ClearSuperkey,
                            )
                        }
                    }
                    vpnhide_core_rpc::RootEffect::ClearSuperkey
                    | vpnhide_core_rpc::RootEffect::FullReset => {
                        vpnhide_activator::set_session_superkey(None);
                        crate::effects::apply(paths, effect)
                    }
                    _ => crate::effects::apply(paths, effect),
                };
                if result.is_err() {
                    return PhaseOutcome::Unknown;
                }
                PhaseOutcome::Confirmed
            }
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
    fn known_native_failure_retains_persist_and_still_applies_ports() {
        // Persist confirmed, Native fails, and the independent Ports resource still applies.
        let runner = FakeRunner::new(&[
            (ConfigPhase::Persist, PhaseOutcome::Confirmed),
            (ConfigPhase::Native, PhaseOutcome::FailedKnown),
            (ConfigPhase::Ports, PhaseOutcome::Confirmed),
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
        assert_eq!(record(&out, ConfigPhase::Ports), PhaseOutcome::Confirmed);
        assert_eq!(
            *runner.ran.borrow(),
            vec![
                ConfigPhase::Persist,
                ConfigPhase::Native,
                ConfigPhase::Ports
            ]
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
        // A disabled backend phase is NotAttempted, not silently confirmed.
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
            vec![
                ConfigPhase::Persist,
                ConfigPhase::Native,
                ConfigPhase::Ports
            ]
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
