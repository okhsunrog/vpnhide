use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

pub(crate) fn fixture(grant: bool) -> (tempfile::TempDir, PreparedWithdrawal) {
    let (root, env) = crate::maintenance::zygisk_projection::tests::active_binding_fixture();
    let paths = ProjectionPaths {
        root: root.path().into(),
        persistent: root.path().join("persistent"),
    };
    let mut journal = crate::journal::read(&paths.persistent.join("journal.json")).unwrap();
    if grant {
        let authority = crate::journal::ZygiskAuthorityRecord {
            operation: vpnhide_core_rpc::OperationId(9),
            intent: crate::journal::NativeIntent {
                backend: vpnhide_core_rpc::NativeBackend::Zygisk,
                wire: vpnhide_protocol::format_config(
                    false,
                    0,
                    &[vpnhide_protocol::Target {
                        uid: 10123,
                        hookmask: vpnhide_protocol::hook_ids::ZYGISK_HOOK_MASK,
                    }],
                ),
                zygisk_module: Some(paths.active()),
                zygisk_authorization: Some(crate::journal::ZygiskAuthorizationIntent {
                    boot_id: paths.boot().unwrap(),
                    generation: 9,
                    directory_identity: env
                        .bundle_identity
                        .as_ref()
                        .unwrap()
                        .zygisk_recipient
                        .as_ref()
                        .unwrap()
                        .image
                        .directory,
                }),
            },
        };
        crate::zygisk::ZygiskTarget::open(&paths.active())
            .unwrap()
            .publish_authorized(&vpnhide_protocol::zygisk_activation::Authorization {
                boot_id: paths.boot().unwrap(),
                generation: 9,
                wire: authority.intent.wire.clone(),
            })
            .unwrap();
        journal.zygisk_authority = Some(authority);
    }
    journal.issued_through = 10;
    crate::journal::write_atomic(&paths.persistent.join("journal.json"), &journal).unwrap();
    let uid = unsafe { libc::geteuid() };
    let prepared = PreparedWithdrawal::prepare_owned(
        paths,
        10,
        &journal,
        Manager::discover_owned(root.path(), uid).unwrap(),
        File::open(
            root.path()
                .join("data/adb/modules/vpnhide_core/vpnhide-core"),
        )
        .unwrap(),
        uid,
    )
    .unwrap();
    let template = root.path().join("data/adb/projection-template");
    fs::remove_dir_all(&template).unwrap();
    fs::create_dir(&template).unwrap();
    for (name, bytes) in prepared.descriptor.files().unwrap() {
        fs::write(template.join(name), bytes).unwrap();
    }
    (root, prepared)
}
#[test]
fn withdrawal_package_is_inert_and_staging_preserves_active_code_and_exact_grant() {
    for grant in [false, true] {
        let (root, prepared) = fixture(grant);
        assert!(!prepared.paths.staged().exists());
        assert_eq!(fs::read_dir(&prepared.paths.persistent).unwrap().count(), 1);
        let before = fs::read(
            prepared
                .paths
                .active()
                .join(vpnhide_protocol::zygisk_activation::FILE_NAME),
        )
        .ok();
        let file = prepared.package();
        let mut raw = vec![0; file.metadata().unwrap().len() as usize];
        use std::os::unix::fs::FileExt;
        file.read_exact_at(&mut raw, 0).unwrap();
        let members = vpnhide_artifact_model::archive::read_archive_bytes(&raw, 2).unwrap();
        assert_eq!(
            members.keys().map(String::as_str).collect::<Vec<_>>(),
            ["module.prop", WITHDRAWAL_FILE_NAME]
        );
        let output = prepared.manager.stage(file).unwrap();
        assert!(output.output.status.success());
        let staged = prepared.verify_staged().unwrap();
        assert_eq!(staged.descriptor, prepared.descriptor);
        assert_eq!(
            fs::read(
                prepared
                    .paths
                    .active()
                    .join(vpnhide_protocol::zygisk_activation::FILE_NAME)
            )
            .ok(),
            before
        );
        assert!(
            prepared
                .paths
                .active()
                .join("zygisk/armeabi-v7a.so")
                .exists()
        );
        assert!(prepared.paths.active().join("zygisk/arm64-v8a.so").exists());
        assert!(!prepared.paths.staged().join("zygisk").exists());
        assert_eq!(
            fs::read_to_string(root.path().join("data/adb/dispatch-count")).unwrap(),
            "2"
        );
    }
}
#[test]
fn missing_stage_early_active_replacement_or_extra_executable_never_confirms_withdrawal() {
    for mode in ["none", "projection-active-only", "projection-extra"] {
        let (root, prepared) = fixture(true);
        fs::write(root.path().join("data/adb/mode"), mode).unwrap();
        let _ = prepared.manager.stage(prepared.package()).unwrap();
        assert!(prepared.verify_staged().is_err());
    }
}
#[test]
fn tombstone_rejects_even_empty_library_directories_grants_links_and_unsafe_modes() {
    for variant in 0..4 {
        let (_root, prepared) = fixture(false);
        prepared.manager.stage(prepared.package()).unwrap();
        let staged = prepared.paths.staged();
        match variant {
            0 => fs::create_dir(staged.join("zygisk")).unwrap(),
            1 => fs::write(
                staged.join(vpnhide_protocol::zygisk_activation::FILE_NAME),
                b"grant",
            )
            .unwrap(),
            2 => {
                fs::remove_file(staged.join(WITHDRAWAL_FILE_NAME)).unwrap();
                symlink("elsewhere", staged.join(WITHDRAWAL_FILE_NAME)).unwrap();
            }
            _ => fs::set_permissions(
                staged.join(WITHDRAWAL_FILE_NAME),
                fs::Permissions::from_mode(0o666),
            )
            .unwrap(),
        }
        assert!(prepared.verify_staged().is_err());
    }
}
#[test]
fn active_grant_or_code_changes_and_replaced_source_stop_prepared_withdrawal() {
    for variant in 0..3 {
        let (_root, prepared) = fixture(true);
        match variant {
            0 => fs::remove_file(
                prepared
                    .paths
                    .active()
                    .join(vpnhide_protocol::zygisk_activation::FILE_NAME),
            )
            .unwrap(),
            1 => fs::write(
                prepared.paths.active().join("zygisk/armeabi-v7a.so"),
                b"wrong",
            )
            .unwrap(),
            _ => fs::write(prepared.paths.source().join("vpnhide-core"), b"wrong").unwrap(),
        }
        assert!(prepared.require_fresh().is_err());
    }
}

fn withdrawal_request() -> vpnhide_core_rpc::BackendChoiceRequest {
    vpnhide_core_rpc::BackendChoiceRequest {
        expected_revision: 1,
        choice: vpnhide_core_rpc::BackendChoice::Auto,
        acknowledged_warning: None,
    }
}

#[test]
fn durable_withdrawal_preserves_published_history_and_active_grant() {
    use crate::journal::{self, backend_choice::record_withdrawal};
    use vpnhide_core_rpc::{BaseRef, PhaseOutcome};
    let (_root, prepared) = fixture(true);
    let path = prepared.paths.persistent.join("journal.json");
    let mut saved = journal::read(&path).unwrap();
    saved.retain_settled = 0;
    let before = saved.clone();
    let work = prepared
        .dispatch(prepared.work(withdrawal_request()).unwrap(), |work| {
            record_withdrawal(&mut saved, BaseRef { revision: 0 }, work)?;
            journal::write_atomic(&path, &saved)?;
            assert_eq!(journal::read(&path)?.zygisk_withdrawal.as_ref(), Some(work));
            Ok(())
        })
        .unwrap();
    assert_eq!(work.outcome, PhaseOutcome::Confirmed);
    assert_eq!(saved.zygisk_projection, before.zygisk_projection);
    assert_eq!(saved.zygisk_authority, before.zygisk_authority);
    assert_eq!(saved.zygisk_exposure_boots, before.zygisk_exposure_boots);
    assert_eq!(saved.backend_choice.as_ref().unwrap().revision, 2);
    assert!(
        saved
            .get(before.zygisk_projection.as_ref().unwrap().operation)
            .is_some()
    );
    assert!(saved.get(work.operation).is_some());
    prepared.verify_staged().unwrap();
    for backend in [
        vpnhide_core_rpc::NativeBackend::Builtin,
        vpnhide_core_rpc::NativeBackend::Kmod,
        vpnhide_core_rpc::NativeBackend::Kpm,
    ] {
        for action in [
            crate::native_boot::prepared::TransitionAction::Load(backend),
            crate::native_boot::prepared::TransitionAction::Adopt(backend),
        ] {
            for boot in [
                prepared.boot.clone(),
                "22222222-2222-2222-2222-222222222222".into(),
            ] {
                let intent = crate::native_boot::prepared::TransitionIntent {
                    selection_evidence: None,
                    boot_id: boot,
                    filesystem_hiding: false,
                    action,
                    payload: None,
                    bundle: None,
                    retire_zygisk: None,
                    retire_directory_identity: None,
                };
                assert!(
                    saved
                        .check_native_safety(&intent)
                        .unwrap_err()
                        .contains("negative verification")
                );
            }
        }
    }
    assert!(
        prepared
            .dispatch(work.clone(), |_| panic!("no replay"))
            .is_err()
    );
    // An unrelated choice cannot piggyback on a confirmed withdrawal.
    let mut bad = saved.clone();
    bad.backend_choice.as_mut().unwrap().request.choice = vpnhide_core_rpc::BackendChoice::Fixed {
        backend: vpnhide_core_rpc::NativeBackend::Kmod,
    };
    journal::write_atomic(&path, &bad).unwrap();
    assert!(journal::read(&path).is_err());
    for version in [17, 18, 19] {
        let mut bad = saved.clone();
        bad.version = version;
        journal::write_atomic(&path, &bad).unwrap();
        assert!(journal::read(&path).is_err());
    }
}

#[test]
fn withdrawal_faults_preserve_old_choice_and_never_replay_manager() {
    use crate::journal::{self, backend_choice::record_withdrawal};
    use vpnhide_core_rpc::{BaseRef, ConfigPhase, PhaseOutcome};
    for boundary in 1..=3 {
        for after in [false, true] {
            let (root, prepared) = fixture(true);
            let path = prepared.paths.persistent.join("journal.json");
            let mut saved = journal::read(&path).unwrap();
            let original = saved.clone();
            let mut writes = 0;
            assert!(
                prepared
                    .dispatch(prepared.work(withdrawal_request()).unwrap(), |work| {
                        writes += 1;
                        if writes == boundary && !after {
                            return Err(io::Error::other("before sync"));
                        }
                        let mut next = saved.clone();
                        record_withdrawal(&mut next, BaseRef { revision: 0 }, work)?;
                        journal::write_atomic(&path, &next)?;
                        saved = next;
                        if writes == boundary && after {
                            return Err(io::Error::other("after sync"));
                        }
                        Ok(())
                    })
                    .is_err()
            );
            let dispatched =
                || fs::read_to_string(root.path().join("data/adb/dispatch-count")).unwrap();
            assert_eq!(dispatched(), if boundary == 3 { "2" } else { "1" });
            let mut restored = journal::read(&path).unwrap();
            journal::backend_choice::reconcile_unfinished(&mut restored);
            restored.fence_unresolved();
            assert_eq!(restored.zygisk_projection, original.zygisk_projection);
            assert_eq!(restored.zygisk_authority, original.zygisk_authority);
            if boundary == 1 && !after {
                assert!(restored.zygisk_withdrawal.is_none());
                continue;
            }
            let expected = match (boundary, after) {
                (1, true) | (2, false) => PhaseOutcome::FailedKnown,
                (3, true) => PhaseOutcome::Confirmed,
                _ => PhaseOutcome::Unknown,
            };
            let work = restored.zygisk_withdrawal.as_ref().unwrap();
            assert_eq!(work.outcome, expected, "boundary={boundary} after={after}");
            assert_eq!(
                restored.get(work.operation).unwrap().record.settled,
                Some(expected)
            );
            assert_eq!(
                restored
                    .held_resources()
                    .contains(&ConfigPhase::Maintenance),
                expected == PhaseOutcome::Unknown
            );
            assert_eq!(
                restored.backend_choice.as_ref().unwrap().revision,
                if expected == PhaseOutcome::Confirmed {
                    2
                } else {
                    1
                }
            );
            journal::write_atomic(&path, &restored).unwrap();
            assert_eq!(
                journal::read(&path).unwrap().zygisk_withdrawal,
                restored.zygisk_withdrawal
            );
            assert!(
                prepared
                    .dispatch(work.clone(), |_| panic!("no replay after restart"))
                    .is_err()
            );
            if expected == PhaseOutcome::Unknown {
                let before = restored.clone();
                let fresh = prepared.work(withdrawal_request()).unwrap();
                assert!(record_withdrawal(&mut restored, BaseRef { revision: 0 }, &fresh).is_err());
                assert_eq!(restored, before);
            }
            assert_eq!(dispatched(), if boundary == 3 { "2" } else { "1" });
        }
    }
}

#[test]
fn boot_negative_proof_requires_exact_tombstone_and_preserves_same_boot_code() {
    use super::observation::WithdrawalObservation;
    use crate::journal::{self, backend_choice::record_withdrawal};
    use vpnhide_core_rpc::BaseRef;
    let (_root, prepared) = fixture(true);
    let path = prepared.paths.persistent.join("journal.json");
    let mut saved = journal::read(&path).unwrap();
    let work = prepared
        .dispatch(prepared.work(withdrawal_request()).unwrap(), |work| {
            record_withdrawal(&mut saved, BaseRef { revision: 0 }, work)?;
            journal::write_atomic(&path, &saved)
        })
        .unwrap();
    assert!(matches!(
        work.observe_owned(&prepared.paths, &prepared.running, prepared.uid)
            .unwrap(),
        WithdrawalObservation::RetainedActive(_)
    ));
    // Even an early manager replacement cannot certify that existing hooks exited.
    let old = prepared.paths.root.join("old-active");
    fs::rename(prepared.paths.active(), &old).unwrap();
    fs::rename(prepared.paths.staged(), prepared.paths.active()).unwrap();
    assert!(
        work.observe_owned(&prepared.paths, &prepared.running, prepared.uid)
            .is_err()
    );
    let boot = "22222222-2222-2222-2222-222222222222";
    fs::write(
        prepared.paths.root.join("proc/sys/kernel/random/boot_id"),
        boot,
    )
    .unwrap();
    let WithdrawalObservation::Withdrawn(proof) = work
        .observe_owned(&prepared.paths, &prepared.running, prepared.uid)
        .unwrap()
    else {
        panic!("new boot should prove withdrawal");
    };
    proof
        .recheck_owned(&prepared.running, prepared.uid)
        .unwrap();
    let intent = crate::native_boot::prepared::TransitionIntent {
        selection_evidence: None,
        boot_id: boot.into(),
        filesystem_hiding: false,
        action: crate::native_boot::prepared::TransitionAction::Adopt(
            vpnhide_core_rpc::NativeBackend::Builtin,
        ),
        payload: None,
        bundle: None,
        retire_zygisk: None,
        retire_directory_identity: None,
    };
    saved
        .check_native_safety_with_withdrawal(&intent, Some(&proof))
        .unwrap();
    saved.note_zygisk_exposure(boot);
    assert!(
        saved
            .check_native_safety_with_withdrawal(&intent, Some(&proof))
            .is_err()
    );
    for name in [
        "zygisk",
        vpnhide_protocol::zygisk_activation::FILE_NAME,
        "service.sh",
    ] {
        fs::write(prepared.paths.active().join(name), b"residue").unwrap();
        assert!(
            proof
                .recheck_owned(&prepared.running, prepared.uid)
                .is_err()
        );
        fs::remove_file(prepared.paths.active().join(name)).unwrap();
    }
    fs::rename(prepared.paths.active(), prepared.paths.staged()).unwrap();
    assert!(
        proof
            .recheck_owned(&prepared.running, prepared.uid)
            .is_err(),
        "absence alone is not proof"
    );
    assert!(
        work.observe_owned(&prepared.paths, &prepared.running, prepared.uid)
            .is_err()
    );
}

#[test]
fn unknown_withdrawal_never_becomes_new_boot_permission_from_later_bytes() {
    use crate::journal::{self, backend_choice::record_withdrawal};
    use vpnhide_core_rpc::{BaseRef, PhaseOutcome};
    let (root, prepared) = fixture(true);
    fs::write(root.path().join("data/adb/mode"), "none").unwrap();
    let path = prepared.paths.persistent.join("journal.json");
    let mut saved = journal::read(&path).unwrap();
    let work = prepared
        .dispatch(prepared.work(withdrawal_request()).unwrap(), |work| {
            record_withdrawal(&mut saved, BaseRef { revision: 0 }, work)?;
            journal::write_atomic(&path, &saved)
        })
        .unwrap();
    assert_eq!(work.outcome, PhaseOutcome::Unknown);
    assert!(matches!(
        work.observe_owned(&prepared.paths, &prepared.running, prepared.uid)
            .unwrap(),
        super::observation::WithdrawalObservation::RetainedActive(_)
    ));
    fs::write(root.path().join("data/adb/mode"), "projection").unwrap();
    prepared.manager.stage(prepared.package()).unwrap();
    let old = root.path().join("old-active");
    fs::rename(prepared.paths.active(), old).unwrap();
    fs::rename(prepared.paths.staged(), prepared.paths.active()).unwrap();
    fs::write(
        root.path().join("proc/sys/kernel/random/boot_id"),
        "22222222-2222-2222-2222-222222222222",
    )
    .unwrap();
    assert!(
        work.observe_owned(&prepared.paths, &prepared.running, prepared.uid)
            .is_err()
    );
    assert_eq!(
        saved.zygisk_withdrawal.as_ref().unwrap().outcome,
        PhaseOutcome::Unknown
    );
}

#[test]
fn later_choice_has_a_scoped_receipt_without_rewriting_withdrawal_or_allowing_zygisk() {
    use crate::journal::{
        self,
        backend_choice::{BackendChoiceRecord, record_withdrawal},
    };
    use vpnhide_core_rpc::{
        BackendChoice, BaseRef, ConfigEdit, ConfigPhase, NativeBackend, OperationId,
        OperationRecord, PhaseOutcome, PhaseRecord,
    };
    let (_root, prepared) = fixture(true);
    let path = prepared.paths.persistent.join("journal.json");
    let mut saved = journal::read(&path).unwrap();
    let work = prepared
        .dispatch(prepared.work(withdrawal_request()).unwrap(), |work| {
            record_withdrawal(&mut saved, BaseRef { revision: 0 }, work)?;
            journal::write_atomic(&path, &saved)
        })
        .unwrap();
    fs::rename(
        prepared.paths.active(),
        prepared.paths.root.join("old-active"),
    )
    .unwrap();
    fs::rename(prepared.paths.staged(), prepared.paths.active()).unwrap();
    let boot = "22222222-2222-2222-2222-222222222222";
    fs::write(
        prepared.paths.root.join("proc/sys/kernel/random/boot_id"),
        boot,
    )
    .unwrap();
    let super::observation::WithdrawalObservation::Withdrawn(proof) = work
        .observe_owned(&prepared.paths, &prepared.running, prepared.uid)
        .unwrap()
    else {
        panic!("negative proof required")
    };
    let choice = BackendChoiceRecord {
        inherited: false,
        revision: 3,
        operation: OperationId(11),
        requested_in_boot: boot.into(),
        request: vpnhide_core_rpc::BackendChoiceRequest {
            expected_revision: 2,
            choice: BackendChoice::Fixed {
                backend: NativeBackend::Kmod,
            },
            acknowledged_warning: None,
        },
    };
    saved.withdrawal_followup = Some(super::followup::WithdrawalFollowup {
        choice: choice.clone(),
        withdrawal: work.operation,
        source: proof.source.clone(),
        image: proof.image.clone(),
    });
    saved.backend_choice = Some(choice.clone());
    saved.upsert(crate::journal::JournalEntry {
        admission: Some(crate::journal::Admission {
            from_revision: 0,
            commit_revision: 0,
            writes: vec![],
        }),
        command: Some(crate::journal::CommandIntent::StageBackendChoice {
            request: choice.request,
        }),
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
            op: choice.operation,
            phases: vec![PhaseRecord {
                phase: ConfigPhase::Maintenance,
                outcome: PhaseOutcome::Confirmed,
            }],
            settled: Some(PhaseOutcome::Confirmed),
            ports: None,
        },
    });
    saved.retain_settled = 0;
    saved.prune();
    journal::write_atomic(&path, &saved).unwrap();
    assert_eq!(
        journal::read(&path).unwrap().zygisk_withdrawal,
        Some(work.clone())
    );
    assert!(saved.get(OperationId(11)).is_some());
    for variant in 0..5 {
        let mut bad = saved.clone();
        match variant {
            0 => {
                bad.withdrawal_followup
                    .as_mut()
                    .unwrap()
                    .source
                    .files
                    .get_mut("vpnhide-core")
                    .unwrap()
                    .identity
                    .sha256 = "e".repeat(64)
            }
            1 => {
                bad.withdrawal_followup
                    .as_mut()
                    .unwrap()
                    .choice
                    .requested_in_boot = work.boot_id.clone()
            }
            2 => {
                bad.withdrawal_followup
                    .as_mut()
                    .unwrap()
                    .choice
                    .request
                    .choice = BackendChoice::Fixed {
                    backend: NativeBackend::Zygisk,
                }
            }
            3 => bad.withdrawal_followup.as_mut().unwrap().withdrawal = OperationId(9),
            _ => bad.version = 20,
        }
        journal::write_atomic(&path, &bad).unwrap();
        assert!(journal::read(&path).is_err(), "invalid variant {variant}");
    }
}

#[test]
fn explicit_reentry_archives_completed_cycle_and_preserves_tombstone_until_new_boot() {
    use crate::journal::{
        self,
        backend_choice::{record_projection, record_withdrawal},
    };
    use vpnhide_core_rpc::{
        BackendChoice, BaseRef, NativeBackend, PhaseOutcome, ZygiskChoiceWarning,
    };
    let (root, withdrawal) = fixture(true);
    let paths = withdrawal.paths.clone();
    let path = paths.persistent.join("journal.json");
    let mut saved = journal::read(&path).unwrap();
    let old_publication = saved.zygisk_projection.clone();
    let old = withdrawal
        .dispatch(withdrawal.work(withdrawal_request()).unwrap(), |work| {
            record_withdrawal(&mut saved, BaseRef { revision: 0 }, work)?;
            journal::write_atomic(&path, &saved)
        })
        .unwrap();
    fs::rename(paths.active(), root.path().join("old-active")).unwrap();
    fs::rename(paths.staged(), paths.active()).unwrap();
    let boot = "22222222-2222-2222-2222-222222222222";
    fs::write(root.path().join("proc/sys/kernel/random/boot_id"), boot).unwrap();
    let super::observation::WithdrawalObservation::Withdrawn(proof) = old
        .observe_owned(&paths, &withdrawal.running, withdrawal.uid)
        .unwrap()
    else {
        panic!("negative proof required")
    };
    let prepared = PreparedProjection::prepare_owned_kind(
        paths.clone(),
        20,
        Manager::discover_owned(root.path(), withdrawal.uid).unwrap(),
        File::open(paths.source().join("vpnhide-core")).unwrap(),
        withdrawal.uid,
        Some(proof.image.clone()),
    )
    .unwrap();
    use std::os::unix::fs::FileExt;
    let mut raw = vec![0; prepared.package().metadata().unwrap().len() as usize];
    prepared.package().read_exact_at(&mut raw, 0).unwrap();
    let template = root.path().join("data/adb/projection-template");
    fs::remove_dir_all(&template).unwrap();
    fs::create_dir(&template).unwrap();
    for (name, bytes) in vpnhide_artifact_model::archive::read_archive_bytes(&raw, 4).unwrap() {
        let file = template.join(name);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, bytes).unwrap();
    }
    saved.issued_through = 20;
    saved.retain_settled = 0;
    let mut boundaries = Vec::new();
    let work = prepared
        .dispatch(
            prepared
                .work(vpnhide_core_rpc::BackendChoiceRequest {
                    expected_revision: 2,
                    choice: BackendChoice::Fixed {
                        backend: NativeBackend::Zygisk,
                    },
                    acknowledged_warning: Some(ZygiskChoiceWarning::ProcessInjectionV1),
                })
                .unwrap(),
            |work| {
                record_projection(&mut saved, BaseRef { revision: 0 }, work)?;
                journal::write_atomic(&path, &saved)?;
                journal::read(&path)?;
                boundaries.push(saved.clone());
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(boundaries.len(), 3);
    for (index, mut interrupted) in boundaries.into_iter().enumerate() {
        interrupted.fence_unresolved();
        journal::write_atomic(&path, &interrupted).unwrap();
        let restored = journal::read(&path).unwrap();
        if index == 0 {
            assert_eq!(restored.zygisk_projection, old_publication);
            assert_eq!(restored.zygisk_withdrawal, Some(old.clone()));
            assert!(restored.projection_history.is_empty());
            assert_eq!(
                interrupted
                    .get(vpnhide_core_rpc::OperationId(20))
                    .unwrap()
                    .record
                    .settled,
                Some(PhaseOutcome::FailedKnown)
            );
            assert!(
                restored.is_retired(vpnhide_core_rpc::OperationId(20)),
                "zero retention may expire a known no-effect receipt"
            );
            assert!(!restored.has_unknown());
        } else if index == 1 {
            assert_eq!(
                restored.zygisk_projection.as_ref().unwrap().outcome,
                PhaseOutcome::Unknown
            );
            assert_eq!(restored.projection_history.len(), 1);
            assert!(restored.has_unknown());
        } else {
            assert_eq!(
                restored.zygisk_projection.as_ref().unwrap().outcome,
                PhaseOutcome::Confirmed
            );
        }
    }
    let staged_proof = super::observation::WithdrawalBootProof {
        pending_reentry: work.staging.as_ref().unwrap().staged.clone().map(Box::new),
        ..proof.clone()
    };
    staged_proof
        .recheck_owned(&withdrawal.running, withdrawal.uid)
        .unwrap();
    fs::write(paths.staged().join("extra.so"), b"unowned").unwrap();
    assert!(
        staged_proof
            .recheck_owned(&withdrawal.running, withdrawal.uid)
            .is_err()
    );
    fs::remove_file(paths.staged().join("extra.so")).unwrap();
    assert_eq!(work.outcome, PhaseOutcome::Confirmed);
    assert_eq!(saved.projection_history.len(), 1);
    assert_eq!(
        Some(saved.projection_history[0].projection.clone()),
        old_publication
    );
    assert_eq!(saved.projection_history[0].withdrawal, old);
    assert!(saved.zygisk_withdrawal.is_none());
    assert_eq!(
        WithdrawalImage::inspect_owned(&paths.active(), &proof.image.descriptor, withdrawal.uid)
            .unwrap(),
        proof.image
    );
    assert!(!paths.active().join("zygisk").exists());
    assert!(saved.get(old.operation).is_some());
    assert!(saved.get(old_publication.unwrap().operation).is_some());
    for variant in 0..3 {
        let mut bad = saved.clone();
        match variant {
            0 => bad.projection_history.clear(),
            1 => {
                bad.projection_history[0]
                    .source
                    .files
                    .get_mut("vpnhide-core")
                    .unwrap()
                    .identity
                    .sha256 = "f".repeat(64)
            }
            _ => bad.projection_history[0].boot_id = old.boot_id.clone(),
        }
        journal::write_atomic(&path, &bad).unwrap();
        assert!(journal::read(&path).is_err(), "bad history {variant}");
    }
    fs::rename(paths.active(), root.path().join("old-tombstone")).unwrap();
    fs::rename(paths.staged(), paths.active()).unwrap();
    let boot_c = "33333333-3333-3333-3333-333333333333";
    fs::write(root.path().join("proc/sys/kernel/random/boot_id"), boot_c).unwrap();
    assert!(matches!(
        work.observe_owned(&paths, boot_c, &withdrawal.running, withdrawal.uid)
            .unwrap(),
        super::super::work::ProjectionObservation::ActiveAfterBoot(_)
    ));
    assert!(prepared.dispatch(work, |_| panic!("no replay")).is_err());
}
