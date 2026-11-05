use super::*;
use vpnhide_core_rpc::{
    BackendChoice, BackendChoiceRequest, BaseRef, NativeBackend, ZygiskChoiceWarning,
};

fn request() -> BackendChoiceRequest {
    BackendChoiceRequest {
        expected_revision: 0,
        choice: BackendChoice::Fixed {
            backend: NativeBackend::Zygisk,
        },
        acknowledged_warning: Some(ZygiskChoiceWarning::ProcessInjectionV1),
    }
}
pub(crate) fn fixture() -> (tempfile::TempDir, PreparedProjection, PathBuf) {
    let (root, original) = super::super::tests::fixture();
    let paths = original.paths.clone();
    drop(original);
    let old = root.path().join("data/adb/modules/vpnhide_zygisk");
    fs::create_dir_all(old.join("zygisk")).unwrap();
    fs::write(
        old.join("module.prop"),
        "id=vpnhide_zygisk\nauthor=okhsunrog\nvpnhideControl=2\n",
    )
    .unwrap();
    fs::write(
        old.join("service.sh"),
        b"old standalone service; never executed by Core",
    )
    .unwrap();
    fs::write(
        old.join("zygisk/arm64-v8a.so"),
        b"old opaque legacy code; never new payload input",
    )
    .unwrap();
    fs::create_dir_all(root.path().join("data/adb/vpnhide_zygisk")).unwrap();
    fs::write(
        root.path().join("data/adb/vpnhide_zygisk/targets.txt"),
        b"user policy retained",
    )
    .unwrap();
    let prepared = PreparedProjection::prepare_owned(
        paths.clone(),
        8,
        crate::maintenance::manager::tests::discover(root.path()),
        File::open(paths.source().join("vpnhide-core")).unwrap(),
        unsafe { libc::geteuid() },
    )
    .unwrap();
    (root, prepared, old)
}

#[test]
fn warned_adoption_verifies_new_pair_before_deferred_retirement_and_retains_old_policy() {
    let (root, prepared, old) = fixture();
    let old_code = fs::read(old.join("zygisk/arm64-v8a.so")).unwrap();
    let path = prepared.paths.persistent.join("journal.json");
    let mut journal = crate::journal::Journal {
        issued_through: 8,
        retain_settled: 0,
        ..Default::default()
    };
    let mut boundaries = Vec::new();
    let work = prepared
        .dispatch(prepared.work(request()).unwrap(), |work| {
            let adoption = work.legacy_adoption.as_ref().unwrap();
            if !adoption.retirement_intent {
                assert!(!old.join("remove").exists());
            }
            if adoption.retirement_intent {
                assert_eq!(
                    work.staging.as_ref(),
                    Some(&prepared.verify_staged().unwrap())
                );
            }
            if adoption.retired {
                assert!(old.join("disable").is_file() && old.join("remove").is_file());
            }
            crate::journal::backend_choice::record_projection(
                &mut journal,
                BaseRef { revision: 0 },
                work,
            )?;
            crate::journal::write_atomic(&path, &journal)?;
            crate::journal::read(&path)?;
            boundaries.push(journal.clone());
            Ok(())
        })
        .unwrap();
    assert_eq!(boundaries.len(), 4);
    assert_eq!(work.outcome, PhaseOutcome::Confirmed);
    assert_eq!(fs::read(old.join("zygisk/arm64-v8a.so")).unwrap(), old_code);
    assert_eq!(
        fs::read(root.path().join("data/adb/vpnhide_zygisk/targets.txt")).unwrap(),
        b"user policy retained"
    );
    assert!(
        !prepared
            .paths
            .staged()
            .join(vpnhide_protocol::zygisk_activation::FILE_NAME)
            .exists()
    );
    assert!(journal.zygisk_exposed_in(&prepared.boot_id));
    let new_code = fs::read(prepared.paths.staged().join("zygisk/arm64-v8a.so")).unwrap();
    assert_ne!(new_code, old_code);
    assert_eq!(
        crate::journal::read(&path).unwrap().zygisk_projection,
        Some(work.clone())
    );
    for (index, mut interrupted) in boundaries.into_iter().enumerate() {
        interrupted.fence_unresolved();
        crate::journal::write_atomic(&path, &interrupted).unwrap();
        let restored = crate::journal::read(&path).unwrap();
        assert_eq!(
            restored.zygisk_projection.as_ref().unwrap().outcome,
            match index {
                0 => PhaseOutcome::FailedKnown,
                3 => PhaseOutcome::Confirmed,
                _ => PhaseOutcome::Unknown,
            }
        );
    }
    fs::rename(prepared.paths.staged(), prepared.paths.active()).unwrap();
    let boot = "22222222-2222-2222-2222-222222222222";
    fs::write(root.path().join("proc/sys/kernel/random/boot_id"), boot).unwrap();
    assert!(matches!(
        work.observe_owned(&prepared.paths, boot, &prepared.running, prepared.uid)
            .unwrap(),
        work::ProjectionObservation::ActiveAfterBoot(_)
    ));
    fs::remove_file(old.join("remove")).unwrap();
    assert!(
        work.observe_owned(&prepared.paths, boot, &prepared.running, prepared.uid)
            .is_err()
    );
}

#[test]
fn interruption_never_replays_manager_or_marks_old_module_before_retirement_intent_is_synced() {
    for boundary in 1..=4 {
        for after in [false, true] {
            let (root, prepared, old) = fixture();
            let mut journal = crate::journal::Journal {
                issued_through: 8,
                ..Default::default()
            };
            let path = prepared.paths.persistent.join("journal.json");
            crate::journal::write_atomic(&path, &journal).unwrap();
            let mut writes = 0;
            assert!(
                prepared
                    .dispatch(prepared.work(request()).unwrap(), |work| {
                        writes += 1;
                        if writes == boundary && !after {
                            return Err(io::Error::other("before sync"));
                        }
                        crate::journal::backend_choice::record_projection(
                            &mut journal,
                            BaseRef { revision: 0 },
                            work,
                        )?;
                        crate::journal::write_atomic(&path, &journal)?;
                        if writes == boundary && after {
                            return Err(io::Error::other("after sync"));
                        }
                        Ok(())
                    })
                    .is_err()
            );
            assert_eq!(old.join("remove").exists(), boundary == 4);
            let count =
                fs::read_to_string(root.path().join("data/adb/dispatch-count")).unwrap_or_default();
            assert_eq!(count, if boundary >= 3 { "1" } else { "" });
            let mut restored = crate::journal::read(&path).unwrap();
            restored.fence_unresolved();
            crate::journal::write_atomic(&path, &restored).unwrap();
            crate::journal::read(&path).unwrap();
            if let Some(work) = &restored.zygisk_projection {
                assert!(
                    prepared
                        .dispatch(work.clone(), |_| panic!("no replay"))
                        .is_err()
                );
                assert_eq!(
                    work.outcome,
                    if boundary == 4 && after {
                        PhaseOutcome::Confirmed
                    } else if !work.dispatch_intent {
                        PhaseOutcome::FailedKnown
                    } else {
                        PhaseOutcome::Unknown
                    }
                );
            }
        }
    }
}

#[test]
fn changed_foreign_or_new_legacy_source_stops_preparation_without_retirement() {
    let (_root, prepared, old) = fixture();
    fs::write(old.join("service.sh"), b"changed").unwrap();
    assert!(prepared.require_fresh().is_err());
    assert!(!old.join("disable").exists());
    let (_root, prepared, old) = fixture();
    fs::write(
        old.join("module.prop"),
        "id=vpnhide_zygisk\nauthor=foreign\n",
    )
    .unwrap();
    assert!(prepared.require_fresh().is_err());
    assert!(!old.join("remove").exists());
    let (root, prepared) = super::super::tests::fixture();
    let old = root.path().join("data/adb/modules/vpnhide_zygisk");
    fs::create_dir(&old).unwrap();
    fs::write(
        old.join("module.prop"),
        "id=vpnhide_zygisk\nauthor=okhsunrog\n",
    )
    .unwrap();
    assert!(prepared.require_fresh().is_err());
    assert!(!old.join("remove").exists());
}
