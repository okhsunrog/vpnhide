use super::*;
use crate::journal::{self, Journal};
use crate::maintenance::zygisk_projection::work::{
    ProjectionFailure, ProjectionObservation, ProjectionWork,
};
use vpnhide_core_rpc::{
    BackendChoice, BackendChoiceRequest, BaseRef, ConfigPhase, NativeBackend, PhaseOutcome,
    ZygiskChoiceWarning,
};

const BOOT_A: &str = "11111111-1111-1111-1111-111111111111";
const BOOT_B: &str = "22222222-2222-2222-2222-222222222222";
fn request() -> BackendChoiceRequest {
    BackendChoiceRequest {
        expected_revision: 0,
        choice: BackendChoice::Fixed {
            backend: NativeBackend::Zygisk,
        },
        acknowledged_warning: Some(ZygiskChoiceWarning::ProcessInjectionV1),
    }
}
struct Owner {
    path: PathBuf,
    saved: Journal,
    writes: usize,
    fail: Option<(usize, bool)>,
}
impl Owner {
    fn new(prepared: &PreparedProjection) -> Self {
        Self {
            path: prepared.paths.persistent.join("journal.json"),
            saved: Journal {
                issued_through: prepared.descriptor.operation,
                retain_settled: 0,
                ..Default::default()
            },
            writes: 0,
            fail: None,
        }
    }
    fn persist(&mut self, work: &ProjectionWork) -> io::Result<()> {
        self.writes += 1;
        if self.fail == Some((self.writes, false)) {
            return Err(io::Error::other("before atomic write"));
        }
        let mut next = self.saved.clone();
        journal::backend_choice::record_projection(&mut next, BaseRef { revision: 0 }, work)?;
        journal::write_atomic(&self.path, &next)?;
        assert_eq!(
            journal::read(&self.path)?.zygisk_projection.as_ref(),
            Some(work)
        );
        self.saved = next;
        if self.fail == Some((self.writes, true)) {
            return Err(io::Error::other("after atomic write"));
        }
        Ok(())
    }
    fn restart(&self) -> Journal {
        let mut saved = journal::read(&self.path).unwrap();
        journal::backend_choice::reconcile_unfinished(&mut saved);
        saved.fence_unresolved();
        saved
    }
}
fn dispatched(root: &Path) -> u32 {
    fs::read_to_string(root.join("data/adb/dispatch-count"))
        .unwrap_or_default()
        .parse()
        .unwrap_or(0)
}

#[test]
fn atomic_preparation_receipt_and_warning_never_claim_hook_delivery() {
    let (root, prepared) = fixture();
    for choice in [
        BackendChoice::Auto,
        BackendChoice::Fixed {
            backend: NativeBackend::Zygisk,
        },
    ] {
        let mut request = request();
        request.choice = choice;
        request.acknowledged_warning = None;
        assert!(prepared.work(request).is_err());
    }
    let mut owner = Owner::new(&prepared);
    let work = prepared
        .dispatch(prepared.work(request()).unwrap(), |w| {
            if !w.dispatch_intent {
                assert_eq!(dispatched(root.path()), 0);
            }
            owner.persist(w)
        })
        .unwrap();
    assert_eq!(owner.writes, 3);
    assert_eq!(dispatched(root.path()), 1);
    assert_eq!(work.outcome, PhaseOutcome::Confirmed);
    assert!(work.dispatch_intent);
    assert!(owner.saved.zygisk_authority.is_none());
    assert!(owner.saved.zygisk_exposure_boots.is_empty());
    assert!(owner.saved.native_startup.is_none());
    assert_eq!(
        owner.saved.backend_choice.as_ref().unwrap().operation,
        work.operation
    );
    assert!(!prepared.paths.active().exists());
    assert!(
        !prepared
            .paths
            .staged()
            .join(vpnhide_protocol::zygisk_activation::FILE_NAME)
            .exists()
    );
    assert!(matches!(
        work.observe_owned(&prepared.paths, BOOT_A, &prepared.running, prepared.uid)
            .unwrap(),
        ProjectionObservation::PreparedInSameBoot(_)
    ));
    assert!(
        prepared
            .dispatch(work.clone(), |_| panic!("no redispatch"))
            .is_err()
    );
    assert_eq!(dispatched(root.path()), 1);
    let mut restored = owner.restart();
    restored.prune();
    assert_eq!(
        restored.get(work.operation).unwrap().record.settled,
        Some(PhaseOutcome::Confirmed)
    );
    assert_eq!(restored.zygisk_projection, Some(work));
}

#[test]
fn faults_at_every_atomic_boundary_stop_spawn_or_preserve_uncertainty() {
    for write in 1..=3 {
        for after in [false, true] {
            let (root, prepared) = fixture();
            let mut owner = Owner::new(&prepared);
            owner.fail = Some((write, after));
            assert!(
                prepared
                    .dispatch(prepared.work(request()).unwrap(), |w| owner.persist(w))
                    .is_err()
            );
            assert_eq!(
                dispatched(root.path()),
                if write == 3 { 1 } else { 0 },
                "write={write} after={after}"
            );
            if write == 1 && !after {
                assert!(!owner.path.exists());
                continue;
            }
            let restored = owner.restart();
            let work = restored.zygisk_projection.as_ref().unwrap();
            let expected = match (write, after) {
                (1, true) | (2, false) => PhaseOutcome::FailedKnown,
                (3, true) => PhaseOutcome::Confirmed,
                _ => PhaseOutcome::Unknown,
            };
            assert_eq!(work.outcome, expected, "write={write} after={after}");
            assert_eq!(
                restored.get(work.operation).unwrap().record.settled,
                Some(expected)
            );
            assert_eq!(
                restored.backend_choice.is_some(),
                expected == PhaseOutcome::Confirmed
            );
            assert_eq!(
                restored
                    .held_resources()
                    .contains(&ConfigPhase::Maintenance),
                expected == PhaseOutcome::Unknown
            );
            if write == 3 && !after {
                let snapshot = work.clone();
                assert!(matches!(
                    work.observe_owned(&prepared.paths, BOOT_A, &prepared.running, prepared.uid)
                        .unwrap(),
                    ProjectionObservation::PreparedInSameBoot(_)
                ));
                assert_eq!(
                    work, &snapshot,
                    "readback never rewrites Unknown into consent or success"
                );
            }
            assert!(
                prepared
                    .dispatch(work.clone(), |_| panic!("never replay accepted work"))
                    .is_err()
            );
            assert_eq!(dispatched(root.path()), if write == 3 { 1 } else { 0 });
        }
    }
}

#[test]
fn pre_spawn_source_and_boot_changes_are_proven_no_effect_without_erasing_intent() {
    for after_intent in [false, true] {
        let (root, prepared) = fixture();
        let mut owner = Owner::new(&prepared);
        let work = prepared
            .dispatch(prepared.work(request()).unwrap(), |w| {
                owner.persist(w)?;
                if owner.writes == if after_intent { 2 } else { 1 } {
                    fs::write(
                        prepared.paths.root.join("proc/sys/kernel/random/boot_id"),
                        BOOT_B,
                    )?;
                }
                Ok(())
            })
            .unwrap();
        assert_eq!(work.outcome, PhaseOutcome::FailedKnown);
        assert_eq!(
            work.failure,
            Some(ProjectionFailure::SourcesChangedBeforeSpawn)
        );
        assert_eq!(work.dispatch_intent, after_intent);
        assert_eq!(dispatched(root.path()), 0);
        assert!(!work.retained());
        assert!(owner.saved.backend_choice.is_none());
        assert!(!owner.saved.has_unknown());
    }
}

#[test]
fn exit_zero_partial_output_and_failed_child_keep_a_durable_unknown_without_authority() {
    for mode in ["none", "projection-missing-abi", "projection-extra", "fail"] {
        let (root, prepared) = fixture();
        fs::write(root.path().join("data/adb/mode"), mode).unwrap();
        let mut owner = Owner::new(&prepared);
        let work = prepared
            .dispatch(prepared.work(request()).unwrap(), |w| owner.persist(w))
            .unwrap();
        assert_eq!(work.outcome, PhaseOutcome::Unknown, "{mode}");
        assert_eq!(dispatched(root.path()), 1);
        assert!(work.dispatch_intent);
        assert!(work.retained());
        assert!(owner.saved.backend_choice.is_none());
        assert!(owner.saved.zygisk_authority.is_none());
        assert!(
            owner
                .saved
                .held_resources()
                .contains(&ConfigPhase::Maintenance)
        );
        let restored = owner.restart();
        assert_eq!(restored.zygisk_projection.as_ref(), Some(&work));
        assert_eq!(
            restored.get(work.operation).unwrap().record.settled,
            Some(PhaseOutcome::Unknown)
        );
        let before = fs::read(&owner.path).unwrap();
        assert!(
            prepared
                .dispatch(prepared.work(request()).unwrap(), |w| owner.persist(w))
                .is_err(),
            "a freshly rebuilt intent must not replay {mode}"
        );
        assert_eq!(dispatched(root.path()), 1);
        assert_eq!(fs::read(&owner.path).unwrap(), before);
    }
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    fs::set_permissions(to, fs::metadata(from).unwrap().permissions()).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}
#[test]
fn a_new_boot_observes_exact_code_after_manager_swap_not_hook_or_manager_readiness() {
    let (root, prepared) = fixture();
    let mut owner = Owner::new(&prepared);
    let work = prepared
        .dispatch(prepared.work(request()).unwrap(), |w| owner.persist(w))
        .unwrap();
    assert!(
        work.observe_owned(&prepared.paths, BOOT_B, &prepared.running, prepared.uid)
            .is_err(),
        "caller cannot invent a boot"
    );
    fs::write(
        prepared.paths.root.join("proc/sys/kernel/random/boot_id"),
        BOOT_B,
    )
    .unwrap();
    assert!(
        work.observe_owned(&prepared.paths, BOOT_B, &prepared.running, prepared.uid)
            .is_err(),
        "staging is not active"
    );
    fs::rename(prepared.paths.staged(), prepared.paths.active()).unwrap();
    assert!(matches!(
        work.observe_owned(&prepared.paths, BOOT_B, &prepared.running, prepared.uid)
            .unwrap(),
        ProjectionObservation::ActiveAfterBoot(_)
    ));
    let old = root.path().join("old-source");
    fs::rename(prepared.paths.source(), &old).unwrap();
    copy_tree(&old, &prepared.paths.source());
    assert!(
        work.observe_owned(&prepared.paths, BOOT_B, &prepared.running, prepared.uid)
            .is_err(),
        "old running inode cannot certify copied source"
    );
    let running = File::open(prepared.paths.source().join("vpnhide-core")).unwrap();
    assert!(matches!(
        work.observe_owned(&prepared.paths, BOOT_B, &running, prepared.uid)
            .unwrap(),
        ProjectionObservation::ActiveAfterBoot(_)
    ));
    fs::write(
        prepared.paths.root.join("proc/sys/kernel/random/boot_id"),
        BOOT_A,
    )
    .unwrap();
    assert!(
        work.observe_owned(&prepared.paths, BOOT_A, &running, prepared.uid)
            .is_err(),
        "same-boot directory substitution is not a new boot"
    );
    fs::write(
        prepared.paths.root.join("proc/sys/kernel/random/boot_id"),
        BOOT_B,
    )
    .unwrap();
    fs::write(
        prepared.paths.active().join("zygisk/armeabi-v7a.so"),
        b"wrong image",
    )
    .unwrap();
    assert!(
        work.observe_owned(&prepared.paths, BOOT_B, &running, prepared.uid)
            .is_err()
    );
    assert_eq!(dispatched(root.path()), 1);
    assert!(owner.saved.zygisk_authority.is_none());
}

#[test]
fn journal_validation_fences_old_readers_and_retains_work_despite_zero_receipts() {
    let (_root, prepared) = fixture();
    let mut owner = Owner::new(&prepared);
    let work = prepared
        .dispatch(prepared.work(request()).unwrap(), |w| owner.persist(w))
        .unwrap();
    for version in 1..17 {
        let mut old = owner.saved.clone();
        old.version = version;
        journal::write_atomic(&owner.path, &old).unwrap();
        assert!(journal::read(&owner.path).is_err(), "version {version}");
    }
    for variant in 0..9 {
        let mut bad = owner.saved.clone();
        let bad_work = bad.zygisk_projection.as_mut().unwrap();
        match variant {
            0 => bad_work.operation.0 += 1,
            1 => bad_work.request.acknowledged_warning = None,
            2 => bad_work.dispatch_intent = false,
            3 => bad_work.staging = None,
            4 => {
                bad_work
                    .source
                    .files
                    .get_mut("payloads/manifest.json")
                    .unwrap()
                    .identity
                    .sha256 = "0".repeat(64)
            }
            5 => bad_work.manager.discovery_path = "untrusted".into(),
            6 => bad.backend_choice = None,
            7 => bad.operations.clear(),
            _ => {
                bad_work.outcome = PhaseOutcome::FailedKnown;
                bad_work.failure = Some(ProjectionFailure::InterruptedBeforeDispatch);
                bad_work.staging = None;
                bad.backend_choice = None;
                bad.operations[0].record.phases[0].outcome = PhaseOutcome::FailedKnown;
                bad.operations[0].record.settled = Some(PhaseOutcome::FailedKnown);
            }
        }
        journal::write_atomic(&owner.path, &bad).unwrap();
        assert!(journal::read(&owner.path).is_err(), "variant {variant}");
    }
    let mut compact = owner.saved.clone();
    compact.prune();
    assert!(compact.get(work.operation).is_some());
    assert_eq!(compact.zygisk_projection.as_ref(), Some(&work));
    compact.expired_holds.push(ConfigPhase::Native);
    journal::write_atomic(&owner.path, &compact).unwrap();
    assert!(
        owner
            .restart()
            .held_resources()
            .contains(&ConfigPhase::Native)
    );
}

#[test]
fn retained_projection_is_not_forgotten_by_actual_reset_choice_or_replacement_commands() {
    use crate::{CoreState, Publisher};
    use vpnhide_core_rpc::{CoreCommand, CoreReplacementRequest, StateSnapshot, SubmitError};
    for unknown in [false, true] {
        let (root, prepared) = fixture();
        if unknown {
            fs::write(root.path().join("data/adb/mode"), "none").unwrap();
        }
        let mut owner = Owner::new(&prepared);
        let work = prepared
            .dispatch(prepared.work(request()).unwrap(), |w| owner.persist(w))
            .unwrap();
        let state = CoreState::load_with_journal(
            Publisher::new(1, StateSnapshot::default()),
            prepared.paths.persistent.join("config.json"),
            Some(owner.path.clone()),
            None,
        )
        .unwrap();
        state.set_effect_paths(Some(crate::effects::EffectPaths {
            root: root.path().into(),
        }));
        let revision = state.snapshot().backend_choice.unwrap().revision;
        for command in [
            CoreCommand::FullReset,
            CoreCommand::StageBackendChoice {
                base: BaseRef { revision: 0 },
                request: BackendChoiceRequest {
                    expected_revision: revision,
                    choice: BackendChoice::Auto,
                    acknowledged_warning: None,
                },
            },
            CoreCommand::StageCoreReplacement {
                base: BaseRef { revision: 0 },
                request: CoreReplacementRequest {
                    descriptor: b"{}".to_vec(),
                    expected_commit: "a".repeat(40),
                    expected_package_sha256: "b".repeat(64),
                    target_rpc: 33,
                },
            },
        ] {
            let op = state.issue_operation_id().unwrap();
            assert_eq!(
                state.submit_command(op, command.clone()),
                Err(SubmitError::Busy),
                "{command:?}, unknown={unknown}"
            );
            assert!(state.get_operation_record(op).is_none());
            assert_eq!(
                journal::read(&owner.path)
                    .unwrap()
                    .zygisk_projection
                    .as_ref(),
                Some(&work)
            );
        }
        assert!(!prepared.paths.persistent.join("config.json").exists());
        assert_eq!(dispatched(root.path()), 1);
        state.shutdown();
    }
}

#[test]
fn later_physical_readback_cannot_rewrite_a_terminal_unknown_or_its_warning() {
    let (_root, prepared) = fixture();
    let mut owner = Owner::new(&prepared);
    owner.fail = Some((3, false));
    assert!(
        prepared
            .dispatch(prepared.work(request()).unwrap(), |w| owner.persist(w))
            .is_err()
    );
    let mut restored = owner.restart();
    let mut changed = restored.zygisk_projection.clone().unwrap();
    changed.outcome = PhaseOutcome::Confirmed;
    changed.failure = None;
    changed.staging = Some(prepared.verify_staged().unwrap());
    let before = restored.clone();
    assert!(
        journal::backend_choice::record_projection(
            &mut restored,
            BaseRef { revision: 0 },
            &changed
        )
        .is_err()
    );
    assert_eq!(restored, before);
    changed = before.zygisk_projection.clone().unwrap();
    changed.dispatch_intent = false;
    changed.outcome = PhaseOutcome::Running;
    changed.failure = None;
    assert!(
        journal::backend_choice::record_projection(
            &mut restored,
            BaseRef { revision: 0 },
            &changed
        )
        .is_err()
    );
    assert_eq!(restored, before);
}

#[test]
fn runtime_code_accepts_only_the_exact_retained_authority_not_arbitrary_activation() {
    use std::os::unix::fs::MetadataExt;
    let (_root, prepared) = fixture();
    let mut owner = Owner::new(&prepared);
    let work = prepared
        .dispatch(prepared.work(request()).unwrap(), |w| owner.persist(w))
        .unwrap();
    fs::rename(prepared.paths.staged(), prepared.paths.active()).unwrap();
    fs::write(
        prepared.paths.root.join("proc/sys/kernel/random/boot_id"),
        BOOT_B,
    )
    .unwrap();
    let active = prepared.paths.active();
    let meta = fs::metadata(&active).unwrap();
    let wire = vpnhide_protocol::format_config(false, 0, &[]);
    let grant = vpnhide_protocol::zygisk_activation::Authorization {
        boot_id: BOOT_B.into(),
        generation: 9,
        wire: wire.clone(),
    };
    let authority = journal::ZygiskAuthorityRecord {
        operation: vpnhide_core_rpc::OperationId(9),
        intent: journal::NativeIntent {
            backend: NativeBackend::Zygisk,
            wire,
            zygisk_module: Some(active.clone()),
            zygisk_authorization: Some(journal::ZygiskAuthorizationIntent {
                boot_id: BOOT_B.into(),
                generation: 9,
                directory_identity: (meta.dev(), meta.ino()),
            }),
        },
    };
    let auth_path = active.join(vpnhide_protocol::zygisk_activation::FILE_NAME);
    fs::write(&auth_path, grant.encode().unwrap()).unwrap();
    assert!(
        work.observe_owned(&prepared.paths, BOOT_B, &prepared.running, prepared.uid)
            .is_err()
    );
    assert!(matches!(
        work.observe_with_authority_owned(
            &prepared.paths,
            BOOT_B,
            &prepared.running,
            prepared.uid,
            Some(&authority)
        )
        .unwrap(),
        ProjectionObservation::ActiveAfterBoot(_)
    ));
    for variant in 0..5 {
        let mut wrong = authority.clone();
        match variant {
            0 => wrong.operation.0 += 1,
            1 => wrong.intent.zygisk_module = Some(prepared.paths.source()),
            2 => wrong.intent.zygisk_authorization.as_mut().unwrap().boot_id = BOOT_A.into(),
            3 => {
                wrong
                    .intent
                    .zygisk_authorization
                    .as_mut()
                    .unwrap()
                    .directory_identity
                    .1 += 1
            }
            _ => wrong.intent.wire = vpnhide_protocol::format_config(true, 0, &[]),
        }
        assert!(
            work.observe_with_authority_owned(
                &prepared.paths,
                BOOT_B,
                &prepared.running,
                prepared.uid,
                Some(&wrong)
            )
            .is_err(),
            "variant {variant}"
        );
    }
    fs::remove_file(&auth_path).unwrap();
    symlink("module.prop", &auth_path).unwrap();
    assert!(
        work.observe_with_authority_owned(
            &prepared.paths,
            BOOT_B,
            &prepared.running,
            prepared.uid,
            Some(&authority)
        )
        .is_err()
    );
}
