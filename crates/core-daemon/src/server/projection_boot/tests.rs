use super::*;
use crate::maintenance::zygisk_projection::tests::fixture;
use std::{fs::File, os::unix::fs::symlink};
use vpnhide_core_rpc::{BackendChoiceRequest, StateSnapshot, ZygiskChoiceWarning};

const BOOT_A: &str = "11111111-1111-1111-1111-111111111111";
const BOOT_B: &str = "22222222-2222-2222-2222-222222222222";
fn setup(
    prepared: bool,
) -> (
    tempfile::TempDir,
    Arc<CoreState>,
    crate::native_boot::NativeEnv,
    ProjectionPaths,
    File,
) {
    let (root, image) = fixture();
    let paths = ProjectionPaths {
        root: root.path().into(),
        persistent: root.path().join("persistent"),
    };
    let path = paths.persistent.join("journal.json");
    if prepared {
        let request = BackendChoiceRequest {
            expected_revision: 0,
            choice: BackendChoice::Fixed {
                backend: NativeBackend::Zygisk,
            },
            acknowledged_warning: Some(ZygiskChoiceWarning::ProcessInjectionV1),
        };
        let mut journal = Journal {
            issued_through: image.descriptor.operation,
            ..Default::default()
        };
        image
            .dispatch(image.work(request).unwrap(), |work| {
                journal::backend_choice::record_projection(
                    &mut journal,
                    BaseRef { revision: 0 },
                    work,
                )?;
                journal::write_atomic(&path, &journal)
            })
            .unwrap();
    }
    let state = CoreState::load_with_journal(
        Publisher::new(7, StateSnapshot::default()),
        paths.persistent.join("config.json"),
        Some(path),
        None,
    )
    .unwrap();
    state.set_effect_paths(Some(crate::effects::EffectPaths {
        root: root.path().into(),
    }));
    let mut env = crate::native_boot::NativeEnv::from_env(Some(root.path().join("ctl"))).unwrap();
    env.modules = Some(root.path().join("data/adb/modules"));
    env.boot_id_path = root.path().join("proc/sys/kernel/random/boot_id");
    env.kpm_lock = None;
    env.zygisk_module = None;
    env.legacy_root = None;
    env.apatch_kpms = root.path().join("apatch");
    env.state_file = root.path().join("native.json");
    state.defer_native_boot(env.clone(), 7);
    state
        .loader
        .configure_test_context(paths.source(), BOOT_A.into(), 7);
    let running = File::open(paths.source().join("vpnhide-core")).unwrap();
    (root, state, env, paths, running)
}

#[test]
fn ordinary_kernel_boot_has_no_projection_work_or_extra_exposure() {
    let (_root, state, env, paths, _) = setup(false);
    state.bind_projection_for_boot(&env, 7).unwrap();
    let j = journal::read(&paths.persistent.join("journal.json")).unwrap();
    assert_eq!(j.issued_through, 0);
    assert!(j.native_startup.is_none());
    assert!(j.zygisk_exposure_boots.is_empty());
    state.shutdown();
}

#[test]
fn unowned_disabled_and_broken_link_residue_is_fenced_without_dispatch_or_budget() {
    for variant in 0..3 {
        let (root, state, env, paths, _) = setup(false);
        if variant == 2 {
            symlink("missing-generation", paths.active()).unwrap();
        } else {
            fs::create_dir(paths.active()).unwrap();
            if variant == 1 {
                fs::write(paths.active().join("disable"), b"").unwrap();
            }
        }
        assert!(state.bind_projection_for_boot(&env, 7).is_err());
        let saved = fs::read(paths.persistent.join("journal.json")).unwrap();
        let j = journal::read(&paths.persistent.join("journal.json")).unwrap();
        assert!(j.zygisk_exposed_in(BOOT_A));
        assert_eq!(j.issued_through, 0);
        assert!(j.native_startup.is_none());
        assert!(j.native_load_budget.is_none());
        assert!(j.zygisk_authority.is_none());
        assert!(!root.path().join("data/adb/dispatch-count").exists());
        assert!(matches!(
            state.native_boot_status().unwrap().outcome,
            vpnhide_core_rpc::NativeBootOutcome::Unknown { .. }
        ));
        assert!(state.bind_projection_for_boot(&env, 7).is_err());
        assert_eq!(
            fs::read(paths.persistent.join("journal.json")).unwrap(),
            saved,
            "idempotent safety marker"
        );
        state.shutdown();
    }
}

#[test]
fn owned_inert_same_boot_staging_never_becomes_runtime_or_authority() {
    let (root, state, env, paths, running) = setup(true);
    state
        .inspect_projection_for_boot_owned(&env, Some(&running), unsafe { libc::geteuid() })
        .unwrap();
    let j = journal::read(&paths.persistent.join("journal.json")).unwrap();
    assert_eq!(j.issued_through, 8);
    assert!(j.zygisk_authority.is_none());
    assert!(j.zygisk_exposure_boots.is_empty());
    assert!(!paths.active().exists());
    assert_eq!(
        fs::read_to_string(root.path().join("data/adb/dispatch-count")).unwrap(),
        "1"
    );
    state.shutdown();
}

#[test]
fn new_boot_routes_only_exact_active_code_then_remains_unavailable_without_consumption() {
    let (root, state, env, paths, running) = setup(true);
    fs::write(&env.boot_id_path, BOOT_B).unwrap();
    state
        .loader
        .configure_test_context(paths.source(), BOOT_B.into(), 7);
    assert!(
        state
            .inspect_projection_for_boot_owned(&env, Some(&running), unsafe { libc::geteuid() })
            .is_err()
    );
    fs::rename(paths.staged(), paths.active()).unwrap();
    state
        .inspect_projection_for_boot_owned(&env, Some(&running), unsafe { libc::geteuid() })
        .unwrap();
    assert!(state.loader.owns_projection(env.modules.as_ref().unwrap()));
    let catalog = crate::payloads::Catalog::open(&paths.source()).unwrap();
    assert!(
        !state
            .loader
            .available(&catalog, crate::payloads::Abi::Arm64, true)
    );
    let j = journal::read(&paths.persistent.join("journal.json")).unwrap();
    assert_eq!(j.issued_through, 8);
    assert!(j.zygisk_authority.is_none());
    assert!(j.native_startup.is_none());
    assert!(j.zygisk_exposure_boots.is_empty());
    assert_eq!(
        fs::read_to_string(root.path().join("data/adb/dispatch-count")).unwrap(),
        "1"
    );
    fs::write(paths.active().join("zygisk/armeabi-v7a.so"), b"wrong").unwrap();
    assert!(
        state
            .inspect_projection_for_boot_owned(&env, Some(&running), unsafe { libc::geteuid() })
            .is_err()
    );
    state.shutdown();
}

fn ready_bootstrap() -> (
    tempfile::TempDir,
    Arc<CoreState>,
    crate::native_boot::NativeEnv,
    ProjectionPaths,
) {
    let (root, state, env, paths, running) = setup(true);
    fs::write(&env.boot_id_path, BOOT_B).unwrap();
    state
        .loader
        .configure_test_context(paths.source(), BOOT_B.into(), 7);
    fs::rename(paths.staged(), paths.active()).unwrap();
    state
        .inspect_projection_for_boot_owned(&env, Some(&running), unsafe { libc::geteuid() })
        .unwrap();
    (root, state, env, paths)
}
fn bootstrap(state: &CoreState, env: &crate::native_boot::NativeEnv) -> Result<(), String> {
    state.prepare_projection_bootstrap_owned(env, 7, unsafe { libc::geteuid() })
}
#[test]
fn empty_bootstrap_has_its_own_identity_and_never_confirms_policy_or_loader_availability() {
    let (_root, state, env, paths) = ready_bootstrap();
    bootstrap(&state, &env).unwrap();
    let journal = journal::read(&paths.persistent.join("journal.json")).unwrap();
    let work = journal.zygisk_bootstrap.as_ref().unwrap();
    assert_eq!(work.outcome, PhaseOutcome::Confirmed);
    assert_eq!(work.authority.operation, OperationId(9));
    assert_eq!(journal.issued_through, 9);
    assert!(journal.zygisk_authority.is_none());
    assert!(journal.native_startup.is_none());
    assert!(journal.get(OperationId(9)).is_none());
    let wire = vpnhide_protocol::parse_config(work.authority.intent.wire.as_bytes()).unwrap();
    assert_eq!(wire.default_mask, 0);
    assert!(wire.targets.is_empty());
    assert!(journal.zygisk_exposed_in(BOOT_B));
    let target = crate::zygisk::ZygiskTarget::open(&paths.active()).unwrap();
    target.check_authorization(&work.authority).unwrap();
    let bytes = fs::read(
        paths
            .active()
            .join(vpnhide_protocol::zygisk_activation::FILE_NAME),
    )
    .unwrap();
    bootstrap(&state, &env).unwrap();
    assert_eq!(
        journal::read(&paths.persistent.join("journal.json")).unwrap(),
        journal
    );
    assert_eq!(
        fs::read(
            paths
                .active()
                .join(vpnhide_protocol::zygisk_activation::FILE_NAME)
        )
        .unwrap(),
        bytes
    );
    assert!(!state.loader.available(
        &crate::payloads::Catalog::open(&paths.source()).unwrap(),
        crate::payloads::Abi::Arm64,
        false
    ));
    assert!(
        state
            .native_handle()
            .unwrap()
            .lock()
            .unwrap()
            .selected()
            .is_none()
    );
    state.shutdown();
}
#[test]
fn bootstrap_noops_on_ordinary_or_requested_boot_and_refuses_unknown_authorization_or_kernel() {
    for prepared in [false, true] {
        let (_root, state, env, paths, _) = setup(prepared);
        bootstrap(&state, &env).unwrap();
        assert!(
            journal::read(&paths.persistent.join("journal.json"))
                .unwrap()
                .zygisk_bootstrap
                .is_none()
        );
        assert!(
            !paths
                .active()
                .join(vpnhide_protocol::zygisk_activation::FILE_NAME)
                .exists()
        );
        state.shutdown();
    }
    for kernel in [false, true] {
        let (_root, state, env, paths) = ready_bootstrap();
        let path = paths
            .active()
            .join(vpnhide_protocol::zygisk_activation::FILE_NAME);
        if kernel {
            fs::write(
                &env.ctl,
                vpnhide_protocol::format_status(&vpnhide_protocol::Status {
                    backend: vpnhide_protocol::hook_ids::Backend::Builtin as u32,
                    kver: 0,
                    hooks: 1,
                    error: 0,
                }),
            )
            .unwrap();
        } else {
            fs::write(&path, b"unowned grant").unwrap();
        }
        assert!(bootstrap(&state, &env).is_err());
        assert!(
            journal::read(&paths.persistent.join("journal.json"))
                .unwrap()
                .zygisk_bootstrap
                .is_none()
        );
        assert_eq!(
            journal::read(&paths.persistent.join("journal.json"))
                .unwrap()
                .issued_through,
            8
        );
        if !kernel {
            assert_eq!(fs::read(path).unwrap(), b"unowned grant");
        }
        state.shutdown();
    }
}
#[test]
fn bootstrap_write_failures_and_restart_never_republish_uncertain_authority() {
    for number in 1..=3 {
        for after in [false, true] {
            let (_root, state, env, paths) = ready_bootstrap();
            state
                .call_writer(move |_| journal::fail_write(number, after))
                .unwrap();
            assert!(bootstrap(&state, &env).is_err());
            let authorization = paths
                .active()
                .join(vpnhide_protocol::zygisk_activation::FILE_NAME);
            let bytes = fs::read(&authorization).ok();
            assert_eq!(bytes.is_some(), number == 3);
            let mut recovered = journal::read(&paths.persistent.join("journal.json")).unwrap();
            recovered.fence_unresolved();
            match recovered.zygisk_bootstrap.as_ref() {
                None => assert!(number == 1 && !after),
                Some(work) => {
                    let expected = match (number, after) {
                        (1, true) | (2, false) => PhaseOutcome::FailedKnown,
                        (2, true) | (3, false) => PhaseOutcome::Unknown,
                        (3, true) => PhaseOutcome::Confirmed,
                        _ => unreachable!(),
                    };
                    assert_eq!(work.outcome, expected);
                    assert_eq!(
                        recovered.held_resources().contains(&ConfigPhase::Native),
                        expected == PhaseOutcome::Unknown
                    );
                }
            }
            let retained = recovered.zygisk_bootstrap.clone();
            recovered.retain_settled = 0;
            recovered.prune();
            assert_eq!(recovered.zygisk_bootstrap, retained);
            let path = paths.persistent.join("journal.json");
            // Persist restart fencing before clearing the simulated worker's
            // storage freeze. Recovery never calls the publication adapter.
            state
                .call_writer(move |state| {
                    journal::write_atomic(&path, &recovered).unwrap();
                    state.publish_progress(&recovered);
                    state.publisher.update(|snapshot| {
                        snapshot.storage_fault = None;
                    });
                })
                .unwrap();
            if number == 1 && !after {
                state.shutdown();
                continue;
            }
            if number == 3 && after {
                bootstrap(&state, &env).unwrap();
            } else {
                assert!(bootstrap(&state, &env).is_err());
            }
            assert_eq!(fs::read(&authorization).ok(), bytes);
            assert!(
                journal::read(&paths.persistent.join("journal.json"))
                    .unwrap()
                    .zygisk_authority
                    .is_none()
            );
            state.shutdown();
        }
    }
}

#[test]
fn bootstrap_restart_and_next_boot_qualify_exact_grants_without_policy_confirmation() {
    let (root, state, mut env, paths) = ready_bootstrap();
    bootstrap(&state, &env).unwrap();
    let authorization = paths
        .active()
        .join(vpnhide_protocol::zygisk_activation::FILE_NAME);
    let first = fs::read(&authorization).unwrap();
    state.shutdown();
    drop(state);
    for boot in [BOOT_B, "33333333-3333-3333-3333-333333333333"] {
        fs::write(&env.boot_id_path, boot).unwrap();
        let resumed = CoreState::load_with_journal(
            Publisher::new(7, StateSnapshot::default()),
            paths.persistent.join("config.json"),
            Some(paths.persistent.join("journal.json")),
            None,
        )
        .unwrap();
        resumed.set_effect_paths(Some(crate::effects::EffectPaths {
            root: root.path().into(),
        }));
        resumed.defer_native_boot(env.clone(), 7);
        resumed
            .loader
            .configure_test_context(paths.source(), boot.into(), 7);
        let running = File::open(paths.source().join("vpnhide-core")).unwrap();
        resumed
            .inspect_projection_for_boot_owned(&env, Some(&running), unsafe { libc::geteuid() })
            .unwrap();
        bootstrap(&resumed, &env).unwrap();
        let saved = journal::read(&paths.persistent.join("journal.json")).unwrap();
        let work = saved.zygisk_bootstrap.as_ref().unwrap();
        assert!(saved.zygisk_authority.is_none());
        assert!(saved.native_startup.is_none());
        if boot == BOOT_B {
            assert_eq!(saved.issued_through, 9);
            assert_eq!(fs::read(&authorization).unwrap(), first);
        } else {
            assert_eq!(saved.issued_through, 10);
            assert_eq!(
                work.previous_authority.as_ref().unwrap().operation,
                OperationId(9)
            );
            assert!(saved.zygisk_exposed_in(BOOT_B));
            assert!(saved.zygisk_exposed_in(boot));
        }
        env.loader = None;
        resumed.shutdown();
    }
}

#[test]
fn bootstrap_never_overwrites_a_current_user_policy() {
    let (_root, state, env, paths) = ready_bootstrap();
    bootstrap(&state, &env).unwrap();
    let mut saved = journal::read(&paths.persistent.join("journal.json")).unwrap();
    let mut user = saved.zygisk_bootstrap.as_ref().unwrap().authority.clone();
    user.operation = OperationId(10);
    user.intent
        .zygisk_authorization
        .as_mut()
        .unwrap()
        .generation = 10;
    user.intent.wire = vpnhide_protocol::format_config(
        false,
        0,
        &[vpnhide_protocol::Target {
            uid: 10123,
            hookmask: vpnhide_protocol::hook_ids::ZYGISK_HOOK_MASK,
        }],
    );
    let target = crate::zygisk::ZygiskTarget::open(&paths.active()).unwrap();
    target
        .publish_authorized(&vpnhide_protocol::zygisk_activation::Authorization {
            boot_id: BOOT_B.into(),
            generation: 10,
            wire: user.intent.wire.clone(),
        })
        .unwrap();
    saved.issued_through = 10;
    saved.zygisk_authority = Some(user.clone());
    state
        .call_writer(move |state| {
            state.persist_transition_journal(
                &paths.persistent.join("journal.json"),
                &saved,
                OperationId(10),
            )
        })
        .unwrap()
        .unwrap();
    let authorization = env
        .zygisk_module
        .as_ref()
        .map(|p| p.join(vpnhide_protocol::zygisk_activation::FILE_NAME))
        .unwrap_or_else(|| {
            env.modules
                .as_ref()
                .unwrap()
                .join(MANAGED_MODULE_ID)
                .join(vpnhide_protocol::zygisk_activation::FILE_NAME)
        });
    let bytes = fs::read(&authorization).unwrap();
    bootstrap(&state, &env).unwrap();
    assert_eq!(fs::read(&authorization).unwrap(), bytes);
    let saved = state.engine.store.lock().unwrap().journal.clone();
    assert_eq!(
        saved.zygisk_bootstrap.as_ref().unwrap().authority.operation,
        OperationId(9)
    );
    assert_eq!(saved.issued_through, 10);
    assert_eq!(saved.zygisk_authority, Some(user));
    state.shutdown();
}

#[test]
fn bootstrap_purpose_and_schema_cannot_be_forged_into_user_policy() {
    let (_root, state, env, paths) = ready_bootstrap();
    bootstrap(&state, &env).unwrap();
    let original = journal::read(&paths.persistent.join("journal.json")).unwrap();
    let path = paths.persistent.join("adversarial.json");
    for wire in [
        vpnhide_protocol::format_config(true, 0, &[]),
        vpnhide_protocol::format_config(false, 1, &[]),
        vpnhide_protocol::format_config(
            false,
            0,
            &[vpnhide_protocol::Target {
                uid: 10123,
                hookmask: 1,
            }],
        ),
    ] {
        let mut changed = original.clone();
        let work = changed.zygisk_bootstrap.as_mut().unwrap();
        work.binding.wire = wire.clone();
        work.authority.intent.wire = wire;
        journal::write_atomic(&path, &changed).unwrap();
        assert!(journal::read(&path).is_err());
    }
    let mut old = original.clone();
    old.version = 18;
    journal::write_atomic(&path, &old).unwrap();
    assert!(journal::read(&path).is_err());
    let mut old = original;
    old.version = 18;
    old.zygisk_bootstrap = None;
    journal::write_atomic(&path, &old).unwrap();
    let recovered = journal::read(&path).unwrap();
    assert_eq!(recovered.version, vpnhide_build_contract::JOURNAL_VERSION);
    assert_eq!(recovered.issued_through, 9);
    assert!(recovered.zygisk_bootstrap.is_none());
    assert!(recovered.zygisk_exposed_in(BOOT_B));
    state.shutdown();
}

#[test]
fn an_exact_bootstrap_owner_can_be_revoked_without_losing_exposure_or_republishing() {
    for dispatch in [false, true] {
        let (_root, state, env, paths) = ready_bootstrap();
        bootstrap(&state, &env).unwrap();
        let active = paths.active();
        let persistent = paths.persistent.join("journal.json");
        let authority = state
            .engine
            .store
            .lock()
            .unwrap()
            .journal
            .zygisk_bootstrap
            .as_ref()
            .unwrap()
            .authority
            .clone();
        let expected = authority.clone();
        let result = state
            .call_writer(move |state| {
                let mut current = state.engine.store.lock().unwrap().journal.clone();
                assert_eq!(
                    crate::native_boot::transition::revocable_authority(&current, &active).unwrap(),
                    Some(expected.clone())
                );
                crate::native_boot::transition::revoke(
                    &mut current,
                    &expected,
                    None,
                    BOOT_B,
                    &mut |journal| {
                        state.persist_transition_journal(&persistent, journal, OperationId(9))
                    },
                    || dispatch,
                )
            })
            .unwrap();
        assert_eq!(result.is_ok(), dispatch);
        let recovered = journal::read(&paths.persistent.join("journal.json")).unwrap();
        assert_eq!(
            recovered.zygisk_bootstrap.as_ref().unwrap().authority,
            authority
        );
        assert!(recovered.zygisk_exposed_in(BOOT_B));
        assert_eq!(
            recovered.held_resources().contains(&ConfigPhase::Native),
            !dispatch
        );
        assert_eq!(
            paths
                .active()
                .join(vpnhide_protocol::zygisk_activation::FILE_NAME)
                .exists(),
            !dispatch
        );
        assert!(bootstrap(&state, &env).is_err());
        assert_eq!(
            journal::read(&paths.persistent.join("journal.json"))
                .unwrap()
                .issued_through,
            9
        );
        state.shutdown();
    }
}

#[test]
fn a_stale_bootstrap_epoch_spends_no_identity_and_cannot_replace_current_failure() {
    let (_root, state, env, paths) = ready_bootstrap();
    let previous = state.native_boot_status().unwrap();
    assert!(
        state
            .prepare_projection_bootstrap_owned(&env, 6, unsafe { libc::geteuid() })
            .is_err()
    );
    assert_eq!(state.native_boot_status().unwrap(), previous);
    let saved = journal::read(&paths.persistent.join("journal.json")).unwrap();
    assert_eq!(saved.issued_through, 8);
    assert!(saved.zygisk_bootstrap.is_none());
    assert!(
        !paths
            .active()
            .join(vpnhide_protocol::zygisk_activation::FILE_NAME)
            .exists()
    );
    state.shutdown();
}

#[test]
fn unavailable_boot_identity_never_commits_an_invalid_bootstrap_or_spends_a_generation() {
    for boot in [Some(""), Some("unknown"), None] {
        let (_root, state, env, paths) = ready_bootstrap();
        let path = paths.persistent.join("journal.json");
        let before = fs::read(&path).unwrap();
        match boot {
            Some(boot) => fs::write(&env.boot_id_path, boot).unwrap(),
            None => fs::remove_file(&env.boot_id_path).unwrap(),
        }
        assert!(bootstrap(&state, &env).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(journal::read(&path).unwrap().issued_through, 8);
        assert!(
            !paths
                .active()
                .join(vpnhide_protocol::zygisk_activation::FILE_NAME)
                .exists()
        );
        state.shutdown();
    }
}
