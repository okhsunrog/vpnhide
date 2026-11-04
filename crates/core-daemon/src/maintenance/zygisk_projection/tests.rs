use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

pub(crate) fn fixture() -> (tempfile::TempDir, PreparedProjection) {
    fixture_libraries(0)
}

fn fixture_libraries(min_bytes: usize) -> (tempfile::TempDir, PreparedProjection) {
    let root = tempfile::tempdir().unwrap();
    let paths = ProjectionPaths {
        root: root.path().into(),
        persistent: root.path().join("persistent"),
    };
    fs::create_dir_all(paths.root.join("proc/sys/kernel/random")).unwrap();
    fs::write(
        paths.root.join("proc/sys/kernel/random/boot_id"),
        "11111111-1111-1111-1111-111111111111",
    )
    .unwrap();
    fs::create_dir(&paths.persistent).unwrap();
    fs::set_permissions(&paths.persistent, fs::Permissions::from_mode(0o700)).unwrap();
    vpnhide_artifact_model::fixtures::write_passive_bundle_tree(&paths.source()).unwrap();
    if min_bytes > 0 {
        use sha2::{Digest, Sha256};
        let manifest_path = paths.source().join("payloads/manifest.json");
        let mut manifest =
            crate::payloads::Manifest::parse(&fs::read(&manifest_path).unwrap()).unwrap();
        for payload in &mut manifest.payloads {
            if !matches!(
                payload.implementation,
                crate::payloads::PayloadKind::Zygisk { .. }
            ) {
                continue;
            }
            let path = paths
                .source()
                .join(payload.implementation.relative_path().unwrap());
            let mut data = fs::read(&path).unwrap();
            data.resize(min_bytes.max(data.len()), 0);
            payload.bytes = data.len() as u64;
            payload.sha256 = format!("{:x}", Sha256::digest(&data));
            fs::write(path, data).unwrap();
        }
        fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    }
    let bytes = fs::read(paths.source().join("bin/arm64/vpnhide-core")).unwrap();
    fs::write(paths.source().join("vpnhide-core"), bytes).unwrap();
    fs::set_permissions(
        paths.source().join("vpnhide-core"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let manager = crate::maintenance::manager::tests::install(&paths.root);
    let running = File::open(paths.source().join("vpnhide-core")).unwrap();
    let prepared =
        PreparedProjection::prepare_owned(paths, 8, manager, running, unsafe { libc::geteuid() })
            .unwrap();
    let archive = prepared.package().try_clone().unwrap();
    use std::os::unix::fs::FileExt;
    let mut bytes = vec![0; archive.metadata().unwrap().len() as usize];
    archive.read_exact_at(&mut bytes, 0).unwrap();
    let members = vpnhide_artifact_model::archive::read_archive_bytes(&bytes, 4).unwrap();
    assert_eq!(members.len(), 4);
    let template = root.path().join("data/adb/projection-template");
    for (name, bytes) in members {
        let path = template.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    fs::write(root.path().join("data/adb/mode"), "projection").unwrap();
    (root, prepared)
}

#[test]
fn preparation_is_inert_and_real_manager_receives_the_sealed_pair() {
    let (root, prepared) = fixture();
    assert!(!prepared.paths.active().exists());
    assert!(!prepared.paths.staged().exists());
    assert!(!root.path().join("data/adb/dispatch-count").exists());
    assert_eq!(
        fs::read_dir(&prepared.paths.persistent).unwrap().count(),
        0,
        "temporary archive is gone before admission"
    );
    fs::write(root.path().join("data/adb/mode"), "projection-forge-zip").unwrap();
    prepared.require_fresh().unwrap();
    let result = prepared.manager.stage(prepared.package()).unwrap();
    assert!(result.output.status.success());
    let staged = prepared.verify_staged().unwrap();
    assert_eq!(
        staged.staged.as_ref().unwrap().descriptor,
        prepared.descriptor
    );
    assert_eq!(
        fs::read_to_string(root.path().join("data/adb/dispatch-count")).unwrap(),
        "1"
    );
    assert!(!prepared.paths.active().exists());
    assert!(
        !prepared
            .paths
            .staged()
            .join(vpnhide_protocol::zygisk_activation::FILE_NAME)
            .exists()
    );
    staged
        .require_unchanged_owned(&prepared.paths, &prepared.descriptor, prepared.uid)
        .unwrap();
}

#[test]
fn exit_zero_missing_abi_extra_code_live_change_and_placeholder_only_never_verify() {
    for mode in [
        "none",
        "projection-missing-abi",
        "projection-extra",
        "projection-live-change",
        "projection-placeholder-only",
    ] {
        let (root, prepared) = fixture();
        fs::write(root.path().join("data/adb/mode"), mode).unwrap();
        let result = prepared.manager.stage(prepared.package()).unwrap();
        assert!(result.output.status.success(), "{mode}");
        assert!(prepared.verify_staged().is_err(), "{mode}");
    }
}

#[test]
fn unowned_targets_changed_source_and_unsafe_members_are_refused() {
    let (_root, prepared) = fixture();
    fs::create_dir_all(prepared.paths.active()).unwrap();
    assert!(prepared.require_fresh().is_err());
    fs::remove_dir(prepared.paths.active()).unwrap();
    fs::write(
        prepared.paths.source().join("payloads/manifest.json"),
        b"{}",
    )
    .unwrap();
    assert!(prepared.require_fresh().is_err());
    for variant in 0..4 {
        let (_root, prepared) = fixture();
        prepared.manager.stage(prepared.package()).unwrap();
        let staged = prepared.verify_staged().unwrap();
        let library = prepared.paths.staged().join("zygisk/armeabi-v7a.so");
        match variant {
            0 => {
                fs::remove_file(&library).unwrap();
                symlink("arm64-v8a.so", &library).unwrap();
            }
            1 => {
                fs::set_permissions(prepared.paths.staged(), fs::Permissions::from_mode(0o777))
                    .unwrap();
            }
            2 => {
                fs::write(prepared.paths.staged().join("disable"), b"").unwrap();
            }
            _ => {
                fs::write(
                    prepared.paths.staged().join("module.prop"),
                    format!(
                        "{}updateJson=https://invalid.example/update.json\n",
                        String::from_utf8(Descriptor::module_prop()).unwrap()
                    ),
                )
                .unwrap();
            }
        }
        assert!(
            staged
                .require_unchanged_owned(&prepared.paths, &prepared.descriptor, prepared.uid)
                .is_err()
        );
    }
}

#[test]
fn disabled_removing_or_legacy_autoload_source_never_prepares_discovery() {
    for marker in ["disable", "remove", "zygisk", ".ksu_hot_install"] {
        let (_root, prepared) = fixture();
        fs::write(prepared.paths.source().join(marker), b"").unwrap();
        assert!(prepared.require_fresh().is_err(), "{marker}");
    }
}

#[test]
fn active_and_staged_manager_layouts_are_separate_positive_inert_evidence() {
    for mode in [
        "projection-active",
        "projection-active-only",
        "projection-placeholder",
    ] {
        let (root, prepared) = fixture();
        fs::write(root.path().join("data/adb/mode"), mode).unwrap();
        assert!(
            prepared
                .manager
                .stage(prepared.package())
                .unwrap()
                .output
                .status
                .success()
        );
        let proof = prepared.verify_staged().unwrap();
        assert_eq!(proof.staged.is_some(), mode != "projection-active-only");
        assert_eq!(
            matches!(proof.active, Some(ActiveProjection::Placeholder { .. })),
            mode == "projection-placeholder"
        );
        assert!(
            !prepared
                .paths
                .active()
                .join(vpnhide_protocol::zygisk_activation::FILE_NAME)
                .exists()
        );
        proof
            .require_unchanged_owned(&prepared.paths, &prepared.descriptor, prepared.uid)
            .unwrap();
    }
}

#[test]
fn actual_library_sizes_above_metadata_budget_are_verified_not_treated_as_scripts() {
    let (_root, prepared) = fixture_libraries(96 * 1024);
    prepared.manager.stage(prepared.package()).unwrap();
    let proof = prepared.verify_staged().unwrap();
    assert_eq!(
        proof.staged.as_ref().unwrap().files["zygisk/armeabi-v7a.so"]
            .identity
            .bytes,
        96 * 1024
    );
    fs::write(
        prepared.paths.staged().join("service.sh"),
        vec![0; 96 * 1024],
    )
    .unwrap();
    assert!(prepared.verify_staged().is_err());
}

#[test]
fn foreign_duplicate_and_unsafe_source_metadata_never_becomes_projection_authority() {
    for prop in [
        "id=foreign\n",
        "id=vpnhide_core\nid=foreign\n",
        "id=vpnhide_core\\\n",
    ] {
        let (root, prepared) = fixture();
        fs::write(prepared.paths.source().join("module.prop"), prop).unwrap();
        assert!(prepared.require_fresh().is_err(), "{prop}");
        assert!(!root.path().join("data/adb/dispatch-count").exists());
    }
}

mod work;
