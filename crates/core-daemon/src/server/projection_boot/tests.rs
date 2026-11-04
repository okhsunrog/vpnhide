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
