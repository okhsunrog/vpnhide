use super::*;
use crate::maintenance::zygisk_projection::install::tests::{dispatch, fixture, promote};
use std::fs;

fn state(paths: &ProjectionPaths) -> (std::sync::Arc<CoreState>, crate::native_boot::NativeEnv) {
    let state = CoreState::load_with_journal(
        Publisher::new(7, vpnhide_core_rpc::StateSnapshot::default()),
        paths.root.join("data/system/vpnhide_config.json"),
        Some(paths.persistent.join("operations.json")),
        None,
    )
    .unwrap();
    state.set_effect_paths(Some(crate::effects::EffectPaths {
        root: paths.root.clone(),
    }));
    let mut env = crate::native_boot::NativeEnv::from_env(Some(paths.root.join("ctl"))).unwrap();
    env.modules = Some(paths.root.join("data/adb/modules"));
    env.boot_id_path = paths.root.join("proc/sys/kernel/random/boot_id");
    env.legacy_root = None;
    env.kpm_lock = None;
    env.state_file = paths.root.join("native.json");
    state.defer_native_boot(env.clone(), 7);
    (state, env)
}
#[test]
fn install_boot_writer_preserves_native_receipt_and_inherits_without_fabricated_warning() {
    let (_root, paths, before, incoming) = fixture();
    let guard = crate::instance::InstanceGuard::state(
        &paths.root.join("data/system/vpnhide_config.json"),
        &paths.persistent.join("operations.json"),
    )
    .unwrap();
    dispatch(&paths, &incoming, |_| Ok(())).unwrap();
    assert_eq!(
        crate::journal::read(&paths.persistent.join("operations.json")).unwrap(),
        before
    );
    drop(guard);
    promote(&paths, &incoming);
    let (state, env) = state(&paths);
    let canonical = fs::read(paths.root.join("data/system/vpnhide_config.json")).unwrap();
    state
        .adopt_install_handoff_owned(
            &env,
            7,
            Some(File::open(paths.source().join("vpnhide-core")).unwrap()),
            unsafe { libc::geteuid() },
        )
        .unwrap();
    let journal = crate::journal::read(&paths.persistent.join("operations.json")).unwrap();
    assert_eq!(journal.get(OperationId(8)), before.get(OperationId(8)));
    assert_eq!(journal.zygisk_authority, before.zygisk_authority);
    assert_eq!(
        fs::read(paths.root.join("data/system/vpnhide_config.json")).unwrap(),
        canonical
    );
    let work = journal.zygisk_projection.as_ref().unwrap();
    assert_eq!(
        crate::server::backend_choice::offer::upgrade_source(&journal),
        Some(work.descriptor.source_commit.clone())
    );
    state.prepare_backend_offer().unwrap();
    assert!(
        state
            .engine
            .store
            .lock()
            .unwrap()
            .journal
            .backend_offer
            .is_none(),
        "historical consent is not current-boot Native confirmation"
    );
    assert!(work.install_continuity.is_some());
    assert!(work.operation.0 > work.descriptor.operation);
    assert!(journal.backend_choice.as_ref().unwrap().inherited);
    assert!(work.request.acknowledged_warning.is_none());
    assert!(
        work.request.validate().is_err(),
        "a fresh external request must still require warning"
    );
    assert!(
        !paths
            .active()
            .join(vpnhide_protocol::zygisk_activation::FILE_NAME)
            .exists()
    );
    state
        .adopt_install_handoff_owned(
            &env,
            7,
            Some(File::open(paths.source().join("vpnhide-core")).unwrap()),
            unsafe { libc::geteuid() },
        )
        .unwrap();
    assert_eq!(
        crate::journal::read(&paths.persistent.join("operations.json")).unwrap(),
        journal
    );
    for variant in 0..5 {
        let mut bad = journal.clone();
        let work = bad.zygisk_projection.as_mut().unwrap();
        match variant {
            0 => {
                work.install_continuity
                    .as_mut()
                    .unwrap()
                    .intent
                    .authority
                    .operation = OperationId(1)
            }
            1 => work.install_continuity.as_mut().unwrap().adopted_in_boot = work.boot_id.clone(),
            2 => {
                work.install_continuity
                    .as_mut()
                    .unwrap()
                    .intent
                    .previous_manifest = "{}".into()
            }
            3 => work.install_continuity = None,
            _ => bad.backend_choice.as_mut().unwrap().inherited = false,
        }
        crate::journal::write_atomic(&paths.persistent.join("bad.json"), &bad).unwrap();
        assert!(
            crate::journal::read(&paths.persistent.join("bad.json")).is_err(),
            "variant={variant}"
        );
    }
}
#[test]
fn install_boot_refuses_changed_user_state_foreign_grant_and_wrong_running_inode() {
    for variant in 0..4 {
        let (_root, paths, mut before, incoming) = fixture();
        dispatch(&paths, &incoming, |_| Ok(())).unwrap();
        promote(&paths, &incoming);
        match variant {
            0 => {
                let mut config = before
                    .get(OperationId(8))
                    .unwrap()
                    .candidate
                    .clone()
                    .unwrap();
                config.apps.clear();
                crate::config_file::write_atomic(
                    &paths.root.join("data/system/vpnhide_config.json"),
                    &config,
                )
                .unwrap();
            }
            1 => {
                fs::write(
                    paths
                        .active()
                        .join(vpnhide_protocol::zygisk_activation::FILE_NAME),
                    b"foreign",
                )
                .unwrap();
            }
            2 => {
                let entry = before.get_mut(OperationId(8)).unwrap();
                entry.record.settled = Some(PhaseOutcome::Unknown);
                entry.record.phases[0].outcome = PhaseOutcome::Unknown;
                crate::journal::write_atomic(&paths.persistent.join("operations.json"), &before)
                    .unwrap();
            }
            _ => {}
        }
        let (state, env) = state(&paths);
        let raw = fs::read(paths.persistent.join("operations.json")).unwrap();
        let exe = if variant == 3 {
            paths.root.join("old-core-retained/vpnhide-core")
        } else {
            paths.source().join("vpnhide-core")
        };
        assert!(
            state
                .adopt_install_handoff_owned(&env, 7, Some(File::open(exe).unwrap()), unsafe {
                    libc::geteuid()
                })
                .is_err(),
            "variant={variant}"
        );
        assert_eq!(
            fs::read(paths.persistent.join("operations.json")).unwrap(),
            raw
        );
        assert!(
            state
                .engine
                .store
                .lock()
                .unwrap()
                .journal
                .zygisk_projection
                .is_none()
        );
    }
}

#[test]
fn install_boot_journal_failure_is_fenced_and_restart_never_republishes() {
    for after_rename in [false, true] {
        let (_root, paths, _before, incoming) = fixture();
        dispatch(&paths, &incoming, |_| Ok(())).unwrap();
        promote(&paths, &incoming);
        let (initial, env) = state(&paths);
        let intent = fs::read(paths.persistent.join("install-handoff/intent.json")).unwrap();
        initial
            .call_writer(move |_| journal::fail_write(1, after_rename))
            .unwrap();
        assert!(
            initial
                .adopt_install_handoff_owned(
                    &env,
                    7,
                    Some(File::open(paths.source().join("vpnhide-core")).unwrap()),
                    unsafe { libc::geteuid() }
                )
                .is_err()
        );
        assert!(initial.snapshot().storage_fault.is_some());
        assert!(
            !paths
                .active()
                .join(vpnhide_protocol::zygisk_activation::FILE_NAME)
                .exists()
        );
        assert_eq!(
            journal::read(&paths.persistent.join("operations.json"))
                .unwrap()
                .zygisk_projection
                .is_some(),
            after_rename
        );
        drop(initial);
        let (restarted, env) = state(&paths);
        restarted
            .adopt_install_handoff_owned(
                &env,
                7,
                Some(File::open(paths.source().join("vpnhide-core")).unwrap()),
                unsafe { libc::geteuid() },
            )
            .unwrap();
        assert!(
            journal::read(&paths.persistent.join("operations.json"))
                .unwrap()
                .zygisk_projection
                .is_some()
        );
        assert_eq!(
            fs::read(paths.persistent.join("install-handoff/intent.json")).unwrap(),
            intent
        );
        assert!(!paths.staged().exists());
        assert!(
            !paths
                .active()
                .join(vpnhide_protocol::zygisk_activation::FILE_NAME)
                .exists()
        );
    }
}

#[test]
fn install_missing_handoff_never_silently_replaces_last_zygisk_choice_with_auto_kernel() {
    let (_root, paths, _before, incoming) = fixture();
    fs::rename(paths.source(), paths.root.join("old-core-retained")).unwrap();
    fs::rename(incoming, paths.source()).unwrap();
    fs::write(
        paths.root.join("proc/sys/kernel/random/boot_id"),
        "22222222-2222-2222-2222-222222222222",
    )
    .unwrap();
    let (state, env) = state(&paths);
    assert!(state.bind_projection_for_boot(&env, 7).is_err());
    let journal = journal::read(&paths.persistent.join("operations.json")).unwrap();
    assert!(journal.zygisk_exposed_in("22222222-2222-2222-2222-222222222222"));
    assert!(journal.zygisk_projection.is_none());
    assert!(!paths.active().exists());
    assert!(
        state
            .native_handle()
            .unwrap()
            .lock()
            .unwrap()
            .selected()
            .is_none()
    );
}

#[test]
fn install_inherited_choice_can_prepare_independent_withdrawal_with_distinct_operation() {
    use crate::maintenance::zygisk_projection::withdrawal::PreparedWithdrawal;
    let (_root, paths, _, incoming) = fixture();
    dispatch(&paths, &incoming, |_| Ok(())).unwrap();
    promote(&paths, &incoming);
    let (state, env) = state(&paths);
    let uid = unsafe { libc::geteuid() };
    state
        .adopt_install_handoff_owned(
            &env,
            7,
            Some(File::open(paths.source().join("vpnhide-core")).unwrap()),
            uid,
        )
        .unwrap();
    let mut journal = crate::journal::read(&paths.persistent.join("operations.json")).unwrap();
    let inherited = journal.zygisk_projection.clone();
    let original_native = journal.get(OperationId(8)).cloned();
    journal.issued_through += 1;
    let prepared = PreparedWithdrawal::prepare_owned(
        paths.clone(),
        journal.issued_through,
        &journal,
        crate::maintenance::zygisk_projection::install::tests::manager(&paths),
        File::open(paths.source().join("vpnhide-core")).unwrap(),
        uid,
    )
    .unwrap();
    let request = BackendChoiceRequest {
        expected_revision: journal.backend_choice.as_ref().unwrap().revision,
        choice: BackendChoice::Auto,
        acknowledged_warning: None,
    };
    let work = prepared.work(request).unwrap();
    assert!(work.operation.0 > work.descriptor.published.operation);
    crate::journal::backend_choice::record_withdrawal(&mut journal, BaseRef { revision: 0 }, &work)
        .unwrap();
    crate::journal::write_atomic(&paths.persistent.join("withdrawal.json"), &journal).unwrap();
    let readback = crate::journal::read(&paths.persistent.join("withdrawal.json")).unwrap();
    assert_eq!(readback.zygisk_projection, inherited);
    assert_eq!(readback.get(OperationId(8)), original_native.as_ref());
    assert!(
        !paths
            .active()
            .join(vpnhide_protocol::zygisk_activation::FILE_NAME)
            .exists()
    );
}
