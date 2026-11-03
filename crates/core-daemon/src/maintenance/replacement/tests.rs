use super::*;
use ring::{
    rand::SystemRandom,
    signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair},
};
use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt};
use vpnhide_package_trust::{DOMAIN, Envelope, PublicKey, TrustStore, VerifiedDescriptor};

fn sign(d: &vpnhide_package_trust::Descriptor) -> (Vec<u8>, VerifiedDescriptor) {
    let rng = SystemRandom::new();
    let key = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
    let key =
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, key.as_ref(), &rng).unwrap();
    let hex = |v: &[u8]| v.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let trust = TrustStore {
        schema: 1,
        keys: vec![PublicKey::P256Sha256 {
            point: hex(key.public_key().as_ref()),
        }],
    };
    let descriptor = serde_json::to_string(d).unwrap();
    let mut message = DOMAIN.to_vec();
    message.extend(descriptor.as_bytes());
    let envelope = serde_json::to_vec(&Envelope {
        descriptor,
        signature: hex(key.sign(&rng, &message).unwrap().as_ref()),
    })
    .unwrap();
    let verified = trust.authenticate(&envelope).unwrap();
    (envelope, verified)
}
fn image(root: &Path, files: &BTreeMap<String, Vec<u8>>) {
    fs::create_dir_all(root).unwrap();
    let manifest: vpnhide_artifact_model::manifest::Manifest =
        serde_json::from_slice(&files["payloads/manifest.json"]).unwrap();
    let allowed = vpnhide_artifact_model::core_layout::CoreLayout::Installed(&manifest)
        .allowed_files()
        .unwrap();
    for name in allowed {
        if name == "update" {
            continue;
        }
        let bytes = if name == "vpnhide-core" {
            &files["bin/arm64/vpnhide-core"]
        } else {
            let Some(bytes) = files.get(&name) else {
                continue;
            };
            bytes
        };
        let path = root.join(&name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        if name == "vpnhide-core" || name == "service.sh" {
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}
pub(crate) fn prepared(paths: ReplacementPaths, op: OperationId) -> PreparedReplacement {
    let uid = unsafe { libc::geteuid() };
    fs::create_dir_all(&paths.persistent).unwrap();
    fs::set_permissions(&paths.persistent, fs::Permissions::from_mode(0o700)).unwrap();
    fs::create_dir_all(paths.app.join("files/core-maintenance")).unwrap();
    fs::create_dir_all(paths.root.join("proc/sys/kernel/random")).unwrap();
    fs::write(
        paths.root.join("proc/sys/kernel/random/boot_id"),
        "11111111-1111-1111-1111-111111111111",
    )
    .unwrap();
    let files = vpnhide_artifact_model::fixtures::passive_bundle_files().unwrap();
    let commit: serde_json::Value =
        serde_json::from_slice(&files["payloads/manifest.json"]).unwrap();
    let commit = commit["bundle_commit"].as_str().unwrap();
    let zip = paths.root.join("replacement.zip");
    vpnhide_artifact_model::archive::write_archive(&zip, &files).unwrap();
    let d = vpnhide_artifact_model::archive::descriptor(&zip, commit, 1001).unwrap();
    let (envelope, verified) = sign(&d);
    image(&paths.active(), &files);
    image(&paths.root.join("data/adb/replacement-template"), &files);
    let manager = super::super::manager::tests::install(&paths.root);
    let before =
        ImageFence::inspect(&paths.active(), None, crate::payloads::Abi::Arm64, uid).unwrap();
    let running = File::open(paths.active().join("vpnhide-core")).unwrap();
    let stage = PackageStaging::open_owned(&paths.persistent, &paths.app, uid, uid).unwrap();
    fs::copy(
        zip,
        paths
            .app
            .join(format!("files/core-maintenance/{}.zip", d.package_sha256)),
    )
    .unwrap();
    let package = stage.copy_verified(&verified, op.0).unwrap();
    package.verify_bundle().unwrap();
    let work = ReplacementWork {
        request: CoreReplacementRequest {
            descriptor: envelope,
            expected_commit: d.source_commit.clone(),
            expected_package_sha256: d.package_sha256.clone(),
            target_rpc: d.rpc,
        },
        status: CoreMaintenanceStatus {
            operation: op,
            target_commit: d.source_commit,
            stage: CoreMaintenanceStage::Accepted,
            failure: None,
        },
        boot_id: paths.boot().unwrap(),
        before,
        manager: manager.identity.clone(),
        legacy: LegacyInventory::capture_owned(&paths.root, uid).unwrap(),
        staged: None,
        retired: false,
        dispatch_intent: false,
    };
    PreparedReplacement {
        _permit: PreparationPermit::acquire(Arc::new(AtomicBool::new(false))).unwrap(),
        owner_uid: uid,
        running,
        package,
        floor: FloorStore::open_owned(&paths.persistent, uid).unwrap(),
        manager,
        paths,
        work,
    }
}
fn fixture() -> (tempfile::TempDir, PreparedReplacement) {
    let root = tempfile::tempdir().unwrap();
    let p = prepared(
        ReplacementPaths {
            root: root.path().into(),
            persistent: root.path().join("persistent"),
            app: root.path().join("app"),
        },
        OperationId(1),
    );
    (root, p)
}
#[test]
fn prepared_package_dispatch_and_staged_readback_are_separate_controls() {
    let (_root, p) = fixture();
    p.require_fresh().unwrap();
    p.floor.lock().unwrap().accept_prepared(&p.package).unwrap();
    let result = p.manager.stage(p.package.file()).unwrap();
    assert!(result.output.status.success());
    let staged = p.verify_staged().unwrap();
    p.live_unchanged().unwrap();
    p.retire(&staged).unwrap();
    assert!(staged.verified_layout());
    assert!(p.work.valid(1));
}
#[test]
fn real_manager_exit_zero_without_image_and_live_code_mutation_never_verify() {
    for mode in ["none", "partial", "live-change"] {
        let (_root, p) = fixture();
        fs::write(p.paths.root.join("data/adb/mode"), mode).unwrap();
        assert!(
            p.manager
                .stage(p.package.file())
                .unwrap()
                .output
                .status
                .success()
        );
        if mode == "live-change" {
            assert!(p.live_unchanged().is_err());
        } else {
            assert!(p.verify_staged().is_err());
        }
    }
}
#[test]
fn generation_proof_requires_real_running_inode_and_preboot_retirement() {
    use super::reconcile::GenerationProof;
    let (_root, mut p) = fixture();
    let legacy = p.paths.root.join("data/adb/modules/vpnhide_kmod");
    fs::create_dir(&legacy).unwrap();
    fs::write(
        legacy.join("module.prop"),
        "id=vpnhide_kmod\nauthor=okhsunrog\n",
    )
    .unwrap();
    fs::write(legacy.join("payload.ko"), b"old payload").unwrap();
    p.work.legacy = LegacyInventory::capture_owned(&p.paths.root, p.owner_uid).unwrap();
    p.floor.lock().unwrap().accept_prepared(&p.package).unwrap();
    p.manager.stage(p.package.file()).unwrap();
    let staged = p.verify_staged().unwrap();
    p.retire(&staged).unwrap();
    p.work.dispatch_intent = true;
    p.work.staged = Some(staged);
    p.work.retired = true;
    p.work.status.stage = CoreMaintenanceStage::PendingRestart;
    assert_eq!(
        p.work.same_boot(&p.paths, &p.running, p.owner_uid).unwrap(),
        GenerationProof::OldRunning
    );
    let old = p.running.try_clone().unwrap();
    fs::remove_dir_all(p.paths.active()).unwrap();
    fs::rename(p.paths.staged(), p.paths.active()).unwrap();
    let running = File::open(p.paths.active().join("vpnhide-core")).unwrap();
    let floor = p.floor.lock().unwrap().read().unwrap();
    assert!(
        p.work
            .new_boot(&p.paths, &old, p.owner_uid, p.package.verified(), &floor)
            .is_err()
    );
    assert_eq!(
        p.work
            .new_boot(
                &p.paths,
                &running,
                p.owner_uid,
                p.package.verified(),
                &floor
            )
            .unwrap(),
        GenerationProof::NewRunning
    );
    p.work.retired = false;
    assert!(
        p.work
            .new_boot(
                &p.paths,
                &running,
                p.owner_uid,
                p.package.verified(),
                &floor
            )
            .is_err()
    );
}
#[test]
fn recovered_predispatch_proof_does_not_replay_manager() {
    let (_root, mut p) = fixture();
    p.work.fence_restart();
    assert_eq!(
        p.work.same_boot(&p.paths, &p.running, p.owner_uid).unwrap(),
        super::reconcile::GenerationProof::NoDispatch
    );
    assert!(!p.paths.root.join("data/adb/dispatch-count").exists());
    fs::create_dir_all(p.paths.staged()).unwrap();
    assert!(p.work.same_boot(&p.paths, &p.running, p.owner_uid).is_err());
}
