use super::*;
use crate::journal;
use crate::maintenance::replacement::{PreparedReplacement, ReplacementPaths};
use std::os::unix::fs::PermissionsExt;
use vpnhide_core_rpc::BaseRef;
use vpnhide_core_rpc::{BackendChoice, BackendChoiceRequest, NativeBackend, ZygiskChoiceWarning};

pub(crate) fn prepared() -> (tempfile::TempDir, PreparedReplacement, Journal) {
    let root = tempfile::tempdir().unwrap();
    let mut replacement = crate::maintenance::replacement::tests::prepared(
        ReplacementPaths {
            root: root.path().into(),
            persistent: root.path().join("persistent"),
            app: root.path().join("app"),
        },
        OperationId(10),
    );
    let paths = ProjectionPaths {
        root: root.path().into(),
        persistent: root.path().join("persistent"),
    };
    let uid = unsafe { libc::geteuid() };
    let original = PreparedProjection::prepare_owned(
        paths.clone(),
        8,
        Manager::discover_owned(root.path(), uid).unwrap(),
        File::open(paths.source().join("vpnhide-core")).unwrap(),
        uid,
    )
    .unwrap();
    let files = replacement.package.verified_files().unwrap();
    let libs: Vec<_> = original
        .descriptor
        .payloads
        .iter()
        .map(|payload| files[&payload.implementation.relative_path().unwrap()].clone())
        .collect();
    let template = root.path().join("data/adb/projection-template");
    for (name, bytes) in original.descriptor.files(&libs[0], &libs[1]).unwrap() {
        let path = template.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    fs::write(root.path().join("data/adb/mode"), "projection").unwrap();
    let mut journal = Journal {
        issued_through: 10,
        ..Default::default()
    };
    original
        .dispatch(
            original
                .work(BackendChoiceRequest {
                    expected_revision: 0,
                    choice: BackendChoice::Fixed {
                        backend: NativeBackend::Zygisk,
                    },
                    acknowledged_warning: Some(ZygiskChoiceWarning::ProcessInjectionV1),
                })
                .unwrap(),
            |work| {
                journal::backend_choice::record_projection(
                    &mut journal,
                    BaseRef { revision: 0 },
                    work,
                )?;
                journal::write_atomic(&paths.persistent.join("journal.json"), &journal)
            },
        )
        .unwrap();
    fs::rename(paths.staged(), paths.active()).unwrap();
    let target = crate::zygisk::ZygiskTarget::open(&paths.active()).unwrap();
    let wire = vpnhide_protocol::format_config(false, 0, &[]);
    let authority = ZygiskAuthorityRecord {
        operation: OperationId(9),
        intent: journal::NativeIntent {
            backend: NativeBackend::Zygisk,
            wire: wire.clone(),
            zygisk_module: Some(paths.active()),
            zygisk_authorization: Some(journal::ZygiskAuthorizationIntent {
                boot_id: paths.boot().unwrap(),
                generation: 9,
                directory_identity: target.identity().unwrap(),
            }),
        },
    };
    target
        .publish_authorized(&vpnhide_protocol::zygisk_activation::Authorization {
            boot_id: paths.boot().unwrap(),
            generation: 9,
            wire,
        })
        .unwrap();
    journal.zygisk_authority = Some(authority);
    journal::write_atomic(&paths.persistent.join("journal.json"), &journal).unwrap();
    let previous = PreparedWithdrawal::prepare_owned(
        paths.clone(),
        10,
        &journal,
        Manager::discover_owned(root.path(), uid).unwrap(),
        File::open(paths.source().join("vpnhide-core")).unwrap(),
        uid,
    )
    .unwrap();
    let projection = PreparedProjectionUpdate::prepare_from_previous(
        paths.clone(),
        OperationId(10),
        &journal,
        &replacement.package,
        previous,
        uid,
    )
    .unwrap();
    fs::remove_dir_all(&template).unwrap();
    for (name, bytes) in projection
        .work
        .descriptor
        .files(&libs[0], &libs[1])
        .unwrap()
    {
        let path = template.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    replacement.work.projection = Some(projection.work.clone());
    replacement.projection = Some(projection);
    fs::write(root.path().join("data/adb/mode"), "combined").unwrap();
    (root, replacement, journal)
}

#[test]
fn authenticated_combined_update_stages_both_images_without_touching_old_grant() {
    let (root, replacement, journal) = prepared();
    let paths = &replacement.paths;
    let old_grant = fs::read(
        paths
            .root
            .join("data/adb/modules")
            .join(MANAGED_MODULE_ID)
            .join(vpnhide_protocol::zygisk_activation::FILE_NAME),
    )
    .unwrap();
    let projection = replacement.projection.as_ref().unwrap();
    assert!(projection.work.valid(&replacement.work));
    assert!(projection.work.linked(&journal));
    replacement.require_fresh().unwrap();
    replacement
        .manager
        .stage(replacement.package.file())
        .unwrap();
    let source = replacement.verify_staged().unwrap();
    projection.require_fresh().unwrap();
    replacement.manager.stage(projection.package()).unwrap();
    let staged = projection.verify_staged().unwrap();
    replacement.live_unchanged().unwrap();
    assert_eq!(
        fs::read(
            paths
                .root
                .join("data/adb/modules")
                .join(MANAGED_MODULE_ID)
                .join(vpnhide_protocol::zygisk_activation::FILE_NAME)
        )
        .unwrap(),
        old_grant
    );
    let mut work = replacement.work.clone();
    work.dispatch_intent = true;
    work.staged = Some(source);
    work.retired = true;
    work.status.stage = CoreMaintenanceStage::PendingRestart;
    work.projection.as_mut().unwrap().dispatch_intent = true;
    work.projection.as_mut().unwrap().staged = Some(staged);
    assert!(work.valid(10));
    // Simulate the manager's next-boot promotion of both exact trees.
    let active = paths.root.join("data/adb/modules").join(MANAGED_MODULE_ID);
    let staged = paths
        .root
        .join("data/adb/modules_update")
        .join(MANAGED_MODULE_ID);
    fs::rename(&active, root.path().join("old-projection")).unwrap();
    fs::rename(staged, &active).unwrap();
    work.projection
        .as_ref()
        .unwrap()
        .require_new_owned(root.path(), unsafe { libc::geteuid() })
        .unwrap();
    fs::write(
        active.join(vpnhide_protocol::zygisk_activation::FILE_NAME),
        b"unexpected grant",
    )
    .unwrap();
    assert!(
        work.projection
            .as_ref()
            .unwrap()
            .require_new_owned(root.path(), unsafe { libc::geteuid() })
            .is_err()
    );
    // Missing one staged half can never qualify PendingRestart.
    work.projection.as_mut().unwrap().staged = None;
    assert!(!work.valid(10));
}

#[test]
fn companion_bytes_are_reauthenticated_and_unsafe_staging_never_confirms() {
    for mode in ["combined-missing-projection", "projection-extra"] {
        let (root, replacement, _) = prepared();
        fs::write(root.path().join("data/adb/mode"), mode).unwrap();
        let projection = replacement.projection.as_ref().unwrap();
        replacement.manager.stage(projection.package()).unwrap();
        assert!(projection.verify_staged().is_err());
    }
    let (root, replacement, journal) = prepared();
    let paths = ProjectionPaths {
        root: root.path().into(),
        persistent: root.path().join("persistent"),
    };
    fs::set_permissions(&paths.persistent, fs::Permissions::from_mode(0o700)).unwrap();
    let private_fd = format!("/proc/self/fd/{}", replacement.package.file().as_raw_fd());
    fs::write(private_fd, b"corrupted authenticated source").unwrap();
    let uid = unsafe { libc::geteuid() };
    let previous = PreparedWithdrawal::prepare_owned(
        paths.clone(),
        10,
        &journal,
        Manager::discover_owned(root.path(), uid).unwrap(),
        File::open(paths.source().join("vpnhide-core")).unwrap(),
        uid,
    )
    .unwrap();
    assert!(
        PreparedProjectionUpdate::prepare_from_previous(
            paths,
            OperationId(10),
            &journal,
            &replacement.package,
            previous,
            uid
        )
        .is_err()
    );
}
