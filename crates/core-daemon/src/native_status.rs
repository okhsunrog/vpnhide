//! One native read projection for UI, agent and export. No device I/O, loading,
//! inventory resolution or inference from the hooks which happened to install.
use sha2::{Digest, Sha256};
use vpnhide_core_rpc::*;
use vpnhide_native_policy::{NativeHookFamily, NativeSelection, NativeSelectionDetail};
use vpnhide_protocol::hook_ids::{Hook, StatusError};

use crate::journal::{Journal, NativeStartup};
use crate::native_boot::prepared::TransitionAction;

pub(crate) fn publish_selection_evidence(boot: &mut NativeBootStatus, journal: &Journal) {
    boot.selection_evidence = journal
        .native_selection_evidence
        .clone()
        .filter(|e| e.boot_id == boot.boot_id)
        .or(boot.selection_evidence.take());
    boot.selection_conflict = journal
        .native_selection_conflict
        .clone()
        .filter(|e| e.boot_id == boot.boot_id);
}

fn implementation(attempt: &NativeStartup) -> Option<NativeImplementation> {
    let intent = attempt.intent.as_ref()?;
    let backend = match intent.action {
        TransitionAction::Observe => return None,
        TransitionAction::Adopt(backend) | TransitionAction::Load(backend) => backend,
    };
    let source = if backend == NativeBackend::Builtin {
        NativeImplementationSource::Builtin
    } else if intent.bundle.is_some() {
        NativeImplementationSource::Bundled
    } else if intent.payload.is_some() || matches!(intent.action, TransitionAction::Adopt(_)) {
        NativeImplementationSource::Legacy
    } else {
        NativeImplementationSource::Unknown
    };
    Some(NativeImplementation {
        backend,
        source,
        manifest_sha256: intent
            .bundle
            .as_ref()
            .map(|bundle| bundle.manifest_sha256.clone()),
        source_commit: intent
            .bundle
            .as_ref()
            .map(|bundle| bundle.source_commit.clone()),
        payload_sha256: intent
            .payload
            .as_ref()
            .map(|payload| payload.sha256.clone()),
    })
}
fn attempt_view(attempt: &NativeStartup) -> NativeAttemptView {
    let disposition = match &attempt.completed {
        None => NativeAttemptDisposition::Pending,
        Some(result) if result.rejection.is_some() => NativeAttemptDisposition::RejectedNoEffect,
        Some(result) if matches!(result.status.outcome, NativeBootOutcome::Selected { .. }) => {
            NativeAttemptDisposition::Selected
        }
        Some(_)
            if attempt
                .intent
                .as_ref()
                .is_some_and(|intent| intent.action == TransitionAction::Observe) =>
        {
            NativeAttemptDisposition::Refused
        }
        Some(_) => NativeAttemptDisposition::Unknown,
    };
    NativeAttemptView {
        boot_id: attempt.boot_id.clone(),
        config_revision: attempt.config_revision,
        operation: attempt.repair_op,
        implementation: implementation(attempt),
        disposition,
    }
}

/// Only this owner reads durable attempts and wire identities. Receipt pruning
/// may make delivery identity unknown; it never erases desired required coverage.
pub(crate) fn publish_journal(view: &mut NativeReadModel, journal: &Journal) {
    view.attempts = journal
        .native_attempts
        .iter()
        .map(|history| attempt_view(&history.attempt))
        .chain(journal.native_startup.iter().map(attempt_view))
        .collect();
    view.delivery = journal
        .operations
        .iter()
        .rev()
        .filter_map(|entry| {
            let intent = entry.native_intent.as_ref();
            let phase = entry
                .record
                .phases
                .iter()
                .find(|phase| phase.phase == ConfigPhase::Native)?;
            // Ignore a skipped phase, but never skip a newer failed attempt merely
            // because preparation failed before producing its wire intent.
            if phase.outcome == PhaseOutcome::NotAttempted && intent.is_none() {
                return None;
            }
            Some(NativeDeliveryView {
                backend: intent.map(|intent| intent.backend),
                operation: entry.record.op,
                config_revision: entry
                    .admission
                    .as_ref()
                    .map_or(0, |admission| admission.commit_revision),
                outcome: phase.outcome,
                wire_sha256: intent
                    .map(|intent| format!("{:x}", Sha256::digest(intent.wire.as_bytes()))),
            })
        })
        .next();
}
fn family(backend: NativeBackend) -> NativeHookFamily {
    match backend {
        NativeBackend::Kmod => NativeHookFamily::Kmod,
        NativeBackend::Builtin => NativeHookFamily::Builtin,
        NativeBackend::Kpm => NativeHookFamily::Kpm,
        NativeBackend::Zygisk => NativeHookFamily::Zygisk,
    }
}
fn desired_hooks(config: &CanonicalConfig, backend: NativeBackend) -> u32 {
    let selections = config
        .apps
        .iter()
        .map(|(package, app)| {
            (
                package.clone(),
                NativeSelection::Detailed(NativeSelectionDetail {
                    enabled: app.native.enabled,
                    kernel: app.native.kernel.clone(),
                    zygisk: app.native.zygisk.clone(),
                }),
            )
        })
        .collect();
    vpnhide_native_policy::required_hooks(
        &selections,
        family(backend),
        config
            .settings
            .optional_features
            .contains(crate::config_edit::FILESYSTEM_FEATURE),
    )
}
fn unknown_source(backend: NativeBackend) -> NativeImplementation {
    NativeImplementation {
        backend,
        source: if backend == NativeBackend::Builtin {
            NativeImplementationSource::Builtin
        } else {
            NativeImplementationSource::Unknown
        },
        manifest_sha256: None,
        source_commit: None,
        payload_sha256: None,
    }
}
fn source_for(snapshot: &StateSnapshot, backend: NativeBackend) -> NativeImplementation {
    let boot_id = snapshot
        .backend
        .boot
        .as_ref()
        .map(|boot| boot.boot_id.as_str());
    snapshot
        .native_view
        .attempts
        .iter()
        .rev()
        .filter(|attempt| Some(attempt.boot_id.as_str()) == boot_id)
        .filter(|attempt| attempt.disposition == NativeAttemptDisposition::Selected)
        .filter_map(|attempt| attempt.implementation.as_ref())
        .find(|source| source.backend == backend)
        .cloned()
        .unwrap_or_else(|| unknown_source(backend))
}
fn setup(snapshot: &StateSnapshot, view: &NativeReadModel) -> NativeSetupHealth {
    use NativeSetupHealth as H;
    if snapshot.backend.runtime_conflict {
        return H::RuntimeConflict;
    }
    let Some(selected) = view.selected.as_ref().map(|source| source.backend) else {
        return if snapshot.backend.boot.is_some() {
            H::NoSelection
        } else {
            H::Unavailable
        };
    };
    if let Some(active) = view.observed
        && active != selected
    {
        return H::RuntimeConflict;
    }
    let Some(delivery) = &view.delivery else {
        return H::DeliveryUnknown;
    };
    if delivery.config_revision != snapshot.config_revision {
        return if snapshot.operations_in_flight.is_empty() {
            H::DeliveryUnknown
        } else {
            H::DeliveryPending
        };
    }
    match delivery.outcome {
        PhaseOutcome::NotAttempted | PhaseOutcome::Running => return H::DeliveryPending,
        PhaseOutcome::FailedKnown => return H::DeliveryFailed,
        PhaseOutcome::Unknown => return H::DeliveryUnknown,
        PhaseOutcome::Confirmed => {}
    }
    if delivery.backend != Some(selected) {
        return H::DeliveryUnknown;
    }
    if snapshot.mutation_held && snapshot.held_resources.contains(&ConfigPhase::Native) {
        return H::DeliveryUnknown;
    }
    if view.applied != Some(selected) {
        return H::DeliveryUnknown;
    }
    if selected == NativeBackend::Zygisk {
        return H::AwaitingProcessEvidence;
    }
    if view.observation != ObservationFreshness::Current || view.runtime.is_none() {
        return H::AwaitingRuntime;
    }
    let Some(missing) = view.missing_hooks else {
        return H::PolicyUnknown;
    };
    if missing != 0
        || view
            .runtime
            .as_ref()
            .is_some_and(|status| status.status_error == StatusError::PartialHooks as u32)
    {
        return H::PartialHooks;
    }
    if view
        .runtime
        .as_ref()
        .is_some_and(|status| status.status_error != 0)
    {
        return H::BackendError;
    }
    H::SetupReady
}
fn delivery_current(
    snapshot: &StateSnapshot,
    view: &NativeReadModel,
    backend: NativeBackend,
) -> bool {
    view.applied == Some(backend)
        && view.delivery.as_ref().is_some_and(|delivery| {
            delivery.backend == Some(backend)
                && delivery.config_revision == snapshot.config_revision
                && delivery.outcome == PhaseOutcome::Confirmed
        })
        && !(snapshot.mutation_held && snapshot.held_resources.contains(&ConfigPhase::Native))
}
fn filesystem(snapshot: &StateSnapshot, view: &NativeReadModel) -> NativeFilesystemSetup {
    use NativeFilesystemSetup as F;
    let Some(backend) = view.displayed.as_ref().map(|source| source.backend) else {
        return F::Unavailable;
    };
    if view.setup == NativeSetupHealth::RuntimeConflict
        || (backend != NativeBackend::Zygisk && view.observed != Some(backend))
    {
        return F::Unknown;
    }
    let Some(config) = &snapshot.config else {
        return F::Unknown;
    };
    let desired = config
        .settings
        .optional_features
        .contains(crate::config_edit::FILESYSTEM_FEATURE);
    if !delivery_current(snapshot, view, backend) {
        return F::Unknown;
    }
    if backend == NativeBackend::Zygisk {
        return if desired {
            F::PendingEnable
        } else {
            F::Unknown
        };
    }
    let Some(installed) = view
        .installed_hooks
        .map(|hooks| hooks & Hook::FilesystemIfacePaths.bit() != 0)
    else {
        return F::Unknown;
    };
    // Builtin gates this feature in each delivered UID mask. Keeping registered
    // filesystem hooks is not an outstanding unload or reboot requirement.
    if backend == NativeBackend::Builtin && !desired {
        return F::Disabled;
    }
    match (desired, installed) {
        (true, true) => F::Enabled,
        (false, true) => F::PendingDisable,
        (false, false) => F::Disabled,
        (true, false) => {
            let requested = snapshot
                .backend
                .boot
                .as_ref()
                .and_then(|boot| match boot.outcome {
                    NativeBootOutcome::Selected {
                        filesystem_hiding, ..
                    } => filesystem_hiding,
                    _ => None,
                });
            if backend == NativeBackend::Builtin || requested == Some(true) {
                F::SetupError
            } else if requested == Some(false) {
                F::PendingEnable
            } else {
                F::Unknown
            }
        }
    }
}

/// Refresh at every atomic publication boundary, after its owning inputs change.
pub(crate) fn refresh(snapshot: &mut StateSnapshot) {
    let mut view = snapshot.native_view.clone();
    view.boot = snapshot.backend.boot.clone();
    view.selected = snapshot
        .backend
        .selected
        .map(|backend| source_for(snapshot, backend));
    view.applied = snapshot.backend.applied;
    view.observation = snapshot.backend.observed_freshness;
    view.observed = (view.observation == ObservationFreshness::Current)
        .then_some(snapshot.backend.active)
        .flatten();
    view.runtime = snapshot
        .backend
        .runtime_status
        .clone()
        .filter(|status| Some(status.backend) == view.observed);
    view.observation_error = snapshot.backend.observation_error.clone();
    view.displayed = view
        .selected
        .clone()
        .or_else(|| view.observed.map(unknown_source));
    view.legacy_installed = snapshot.backend.installed.clone();
    view.required_hooks = snapshot
        .config
        .as_ref()
        .zip(view.displayed.as_ref())
        .map(|(config, source)| desired_hooks(config, source.backend));
    view.installed_hooks = view
        .runtime
        .as_ref()
        .filter(|status| {
            view.displayed
                .as_ref()
                .is_some_and(|source| source.backend == status.backend)
        })
        .map(|status| status.installed_hooks);
    view.missing_hooks = view
        .required_hooks
        .zip(view.installed_hooks)
        .map(|(required, installed)| required & !installed);
    view.process_coverage_complete = false;
    view.setup = setup(snapshot, &view);
    view.filesystem = filesystem(snapshot, &view);
    snapshot.native_view = view;
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state(backend: NativeBackend) -> StateSnapshot {
        let mut config = CanonicalConfig::default();
        config.apps.insert(
            "app".into(),
            AppConfig {
                native: NativeRole {
                    enabled: true,
                    kernel: None,
                    zygisk: None,
                },
                ..Default::default()
            },
        );
        let required = desired_hooks(&config, backend);
        StateSnapshot {
            config: Some(config),
            config_revision: 7,
            backend: NativeBackendStatus {
                selected: Some(backend),
                applied: Some(backend),
                active: Some(backend),
                observed_freshness: ObservationFreshness::Current,
                runtime_status: Some(NativeRuntimeStatus {
                    backend,
                    kernel_version: 0x60100,
                    installed_hooks: required,
                    status_error: 0,
                }),
                ..Default::default()
            },
            native_view: NativeReadModel {
                delivery: Some(NativeDeliveryView {
                    backend: Some(backend),
                    operation: OperationId(3),
                    config_revision: 7,
                    outcome: PhaseOutcome::Confirmed,
                    wire_sha256: Some("a".repeat(64)),
                }),
                ..Default::default()
            },
            ..Default::default()
        }
    }
    #[test]
    fn builtin_display_and_required_policy_do_not_follow_installed_legacy_priority() {
        let mut snapshot = state(NativeBackend::Builtin);
        snapshot.backend.installed = vec![NativeBackend::Kmod, NativeBackend::Zygisk];
        refresh(&mut snapshot);
        let view = &snapshot.native_view;
        assert_eq!(
            view.displayed.as_ref().unwrap().backend,
            NativeBackend::Builtin
        );
        assert_eq!(
            view.displayed.as_ref().unwrap().source,
            NativeImplementationSource::Builtin
        );
        assert_eq!(view.setup, NativeSetupHealth::SetupReady);
        assert_eq!(view.missing_hooks, Some(0));
        assert!(!view.process_coverage_complete);
        let required = view.required_hooks;
        snapshot
            .backend
            .runtime_status
            .as_mut()
            .unwrap()
            .installed_hooks = 0;
        refresh(&mut snapshot);
        assert_eq!(snapshot.native_view.required_hooks, required);
        assert_eq!(snapshot.native_view.setup, NativeSetupHealth::PartialHooks);
    }
    #[test]
    fn new_config_unknown_delivery_and_runtime_failures_never_inherit_setup_success() {
        let initial = state(NativeBackend::Builtin);
        for change in 0..6 {
            let mut snapshot = initial.clone();
            match change {
                0 => snapshot.config_revision += 1,
                1 => {
                    snapshot.native_view.delivery.as_mut().unwrap().outcome = PhaseOutcome::Unknown
                }
                2 => {
                    snapshot.native_view.delivery.as_mut().unwrap().outcome =
                        PhaseOutcome::FailedKnown
                }
                3 => snapshot.backend.observed_freshness = ObservationFreshness::Unknown,
                4 => snapshot.native_view.delivery = None,
                _ => {
                    snapshot.mutation_held = true;
                    snapshot.held_resources = vec![ConfigPhase::Native];
                }
            }
            refresh(&mut snapshot);
            assert_ne!(snapshot.native_view.setup, NativeSetupHealth::SetupReady);
            assert_ne!(
                snapshot.native_view.filesystem,
                NativeFilesystemSetup::Enabled
            );
        }
    }
    #[test]
    fn builtin_filesystem_is_policy_gated_and_missing_optional_coverage_is_not_hidden() {
        let mut snapshot = state(NativeBackend::Builtin);
        snapshot
            .backend
            .runtime_status
            .as_mut()
            .unwrap()
            .installed_hooks |= Hook::FilesystemIfacePaths.bit();
        refresh(&mut snapshot);
        assert_eq!(
            snapshot.native_view.filesystem,
            NativeFilesystemSetup::Disabled
        );
        snapshot
            .config
            .as_mut()
            .unwrap()
            .settings
            .optional_features
            .insert(crate::config_edit::FILESYSTEM_FEATURE.into());
        snapshot
            .backend
            .runtime_status
            .as_mut()
            .unwrap()
            .installed_hooks &= !Hook::FilesystemIfacePaths.bit();
        refresh(&mut snapshot);
        assert_eq!(snapshot.native_view.setup, NativeSetupHealth::PartialHooks);
        assert_eq!(
            snapshot.native_view.filesystem,
            NativeFilesystemSetup::SetupError
        );
        assert_ne!(
            snapshot.native_view.required_hooks.unwrap() & Hook::FilesystemIfacePaths.bit(),
            0
        );
    }
    #[test]
    fn zygisk_publication_with_no_legacy_module_is_not_absence_or_global_hook_success() {
        let mut snapshot = state(NativeBackend::Zygisk);
        snapshot.backend.active = None;
        snapshot.backend.runtime_status = None;
        refresh(&mut snapshot);
        assert_eq!(
            snapshot.native_view.selected.as_ref().unwrap().backend,
            NativeBackend::Zygisk
        );
        assert_eq!(
            snapshot.native_view.displayed.as_ref().unwrap().backend,
            NativeBackend::Zygisk
        );
        assert!(snapshot.native_view.legacy_installed.is_empty());
        assert_eq!(snapshot.native_view.observed, None);
        assert_eq!(snapshot.native_view.installed_hooks, None);
        assert_eq!(
            snapshot.native_view.setup,
            NativeSetupHealth::AwaitingProcessEvidence
        );
        assert!(!snapshot.native_view.process_coverage_complete);
    }
    fn native_entry(
        id: u64,
        outcome: PhaseOutcome,
        has_intent: bool,
    ) -> crate::journal::JournalEntry {
        crate::journal::JournalEntry {
            maintenance: None,
            maintenance_report: None,
            command: None,
            superseded_by: None,
            superseded_phases: vec![],
            finalized_unknown: false,
            admission: Some(crate::journal::Admission {
                from_revision: 7,
                commit_revision: 7,
                writes: vec![],
            }),
            base: BaseRef { revision: 7 },
            edit: ConfigEdit::RepairActivation,
            candidate: None,
            native_intent: has_intent.then(|| crate::journal::NativeIntent {
                backend: NativeBackend::Builtin,
                wire: "frozen-wire".into(),
                zygisk_module: None,
                zygisk_authorization: None,
            }),
            record: OperationRecord {
                op: OperationId(id),
                phases: vec![PhaseRecord {
                    phase: ConfigPhase::Native,
                    outcome,
                }],
                settled: Some(outcome),
                ports: None,
            },
        }
    }
    #[test]
    fn failed_same_revision_repair_without_wire_never_revives_an_older_confirmed_delivery() {
        for outcome in [
            PhaseOutcome::FailedKnown,
            PhaseOutcome::Unknown,
            PhaseOutcome::Running,
        ] {
            let mut snapshot = state(NativeBackend::Builtin);
            let journal = Journal {
                operations: vec![
                    native_entry(1, PhaseOutcome::Confirmed, true),
                    native_entry(2, outcome, false),
                ],
                ..Default::default()
            };
            publish_journal(&mut snapshot.native_view, &journal);
            refresh(&mut snapshot);
            let delivery = snapshot.native_view.delivery.as_ref().unwrap();
            assert_eq!(delivery.operation, OperationId(2));
            assert_eq!(delivery.backend, None);
            assert_eq!(delivery.wire_sha256, None);
            assert_ne!(snapshot.native_view.setup, NativeSetupHealth::SetupReady);
            assert_eq!(
                snapshot.native_view.filesystem,
                NativeFilesystemSetup::Unknown
            );
        }
    }
    #[test]
    fn pruned_receipts_withdraw_delivery_proof_without_erasing_desired_required_hooks() {
        let mut snapshot = state(NativeBackend::Builtin);
        refresh(&mut snapshot);
        let required = snapshot.native_view.required_hooks;
        publish_journal(&mut snapshot.native_view, &Journal::default());
        refresh(&mut snapshot);
        assert_eq!(snapshot.native_view.required_hooks, required);
        assert_eq!(snapshot.native_view.delivery, None);
        assert_eq!(
            snapshot.native_view.setup,
            NativeSetupHealth::DeliveryUnknown
        );
    }
    #[test]
    fn observed_only_runtime_does_not_borrow_bundled_payload_provenance_from_a_previous_attempt() {
        let mut snapshot = state(NativeBackend::Kpm);
        snapshot.backend.selected = None;
        snapshot.native_view.attempts.push(NativeAttemptView {
            boot_id: "same-boot".into(),
            config_revision: 7,
            operation: None,
            implementation: Some(NativeImplementation {
                backend: NativeBackend::Kpm,
                source: NativeImplementationSource::Bundled,
                manifest_sha256: Some("a".repeat(64)),
                source_commit: Some("b".repeat(40)),
                payload_sha256: Some("c".repeat(64)),
            }),
            disposition: NativeAttemptDisposition::Selected,
        });
        refresh(&mut snapshot);
        let display = snapshot.native_view.displayed.as_ref().unwrap();
        assert_eq!(display.backend, NativeBackend::Kpm);
        assert_eq!(display.source, NativeImplementationSource::Unknown);
        assert_eq!(display.payload_sha256, None);
        assert_ne!(snapshot.native_view.setup, NativeSetupHealth::SetupReady);
    }
    #[test]
    fn conflicting_observed_family_cannot_supply_selected_policy_coverage() {
        let mut snapshot = state(NativeBackend::Builtin);
        snapshot.backend.active = Some(NativeBackend::Kpm);
        snapshot.backend.runtime_status.as_mut().unwrap().backend = NativeBackend::Kpm;
        refresh(&mut snapshot);
        assert_eq!(
            snapshot.native_view.setup,
            NativeSetupHealth::RuntimeConflict
        );
        assert_eq!(snapshot.native_view.installed_hooks, None);
        assert_eq!(snapshot.native_view.missing_hooks, None);
        assert_eq!(
            snapshot.native_view.filesystem,
            NativeFilesystemSetup::Unknown
        );
    }
    #[test]
    fn chosen_bundled_source_is_bound_to_completed_current_boot_attempt() {
        use crate::native_boot::prepared::{BundleIdentity, PayloadIdentity, TransitionIntent};
        let mut snapshot = state(NativeBackend::Kmod);
        let status = NativeBootStatus {
            selection_evidence: None,
            selection_conflict: None,
            boot_id: "boot".into(),
            decided_epoch: 1,
            decided_at_ms: 1,
            updated_at_ms: 2,
            outcome: NativeBootOutcome::Selected {
                backend: NativeBackend::Kmod,
                mode: SelectionMode::Loaded,
                filesystem_hiding: Some(false),
            },
            installed: vec![],
            live: vec![],
            staged: vec![],
            kmod: None,
            zygisk_targets: ZygiskTargets::NotInstalled,
            ports: None,
            obsolete_modules: vec![],
        };
        snapshot.backend.boot = Some(status.clone());
        let startup = NativeStartup {
            boot_id: "boot".into(),
            config_revision: 7,
            filesystem_hiding: false,
            repair_op: Some(OperationId(3)),
            intent: Some(TransitionIntent {
                selection_evidence: None,
                boot_id: "boot".into(),
                filesystem_hiding: false,
                action: TransitionAction::Load(NativeBackend::Kmod),
                payload: Some(PayloadIdentity {
                    source: "/payload".into(),
                    sha256: "c".repeat(64),
                    bytes: 123,
                    init_contract: Some(1),
                }),
                bundle: Some(BundleIdentity {
                    manifest_file_sha256: None,
                    root: "/bundle".into(),
                    manifest_sha256: "a".repeat(64),
                    source_commit: "b".repeat(40),
                    root_identity: Some((1, 2)),
                    wire: "frozen-wire".into(),
                }),
                retire_zygisk: None,
                retire_directory_identity: None,
            }),
            completed: Some(crate::journal::NativeStartupResult {
                status,
                selection: None,
                rejection: None,
            }),
        };
        let mut journal = Journal {
            native_startup: Some(startup.clone()),
            ..Default::default()
        };
        publish_journal(&mut snapshot.native_view, &journal);
        refresh(&mut snapshot);
        let selected = snapshot.native_view.selected.as_ref().unwrap();
        assert_eq!(selected.source, NativeImplementationSource::Bundled);
        assert_eq!(selected.payload_sha256, Some("c".repeat(64)));
        assert_eq!(selected.manifest_sha256, Some("a".repeat(64)));
        assert_eq!(
            snapshot.native_view.attempts[0].disposition,
            NativeAttemptDisposition::Selected
        );
        // Prepared-but-not-completed and another boot cannot claim loaded bytes.
        for (boot, completed) in [("boot", None), ("other-boot", startup.completed.clone())] {
            let attempt = journal.native_startup.as_mut().unwrap();
            attempt.boot_id = boot.into();
            attempt.completed = completed;
            publish_journal(&mut snapshot.native_view, &journal);
            refresh(&mut snapshot);
            assert_eq!(
                snapshot.native_view.selected.as_ref().unwrap().source,
                NativeImplementationSource::Unknown
            );
        }
    }
}
