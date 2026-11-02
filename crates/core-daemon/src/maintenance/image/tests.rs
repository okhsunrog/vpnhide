use super::*;
use ring::{
    rand::SystemRandom,
    signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair},
};
use std::{
    fs,
    io::Seek,
    os::unix::fs::{PermissionsExt, symlink},
};
use vpnhide_package_trust::{DOMAIN, Descriptor, Envelope, PublicKey, RuntimeIdentity, TrustStore};

struct Fixture {
    root: tempfile::TempDir,
    descriptor: VerifiedDescriptor,
    uid: u32,
}
fn identity(path: &Path) -> FileIdentity {
    let bytes = fs::read(path).unwrap();
    FileIdentity {
        bytes: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(&bytes)),
    }
}
fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    vpnhide_artifact_model::fixtures::write_bundle_tree(root.path()).unwrap();
    let runtime = RuntimeIdentity {
        daemon_arm64: identity(&root.path().join("bin/arm64/vpnhide-core")),
        daemon_arm: identity(&root.path().join("bin/arm/vpnhide-core")),
        payload_manifest: identity(&root.path().join("payloads/manifest.json")),
        service: identity(&root.path().join("service.sh")),
        sepolicy: identity(&root.path().join("sepolicy.rule")),
        module_prop: identity(&root.path().join("module.prop")),
    };
    fs::rename(
        root.path().join("bin/arm64/vpnhide-core"),
        root.path().join("vpnhide-core"),
    )
    .unwrap();
    fs::remove_dir_all(root.path().join("bin")).unwrap();
    fs::set_permissions(
        root.path().join("service.sh"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    fs::set_permissions(
        root.path().join("vpnhide-core"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let d = Descriptor {
        schema: 2,
        module_id: "vpnhide_core".into(),
        source_commit: "a".repeat(40),
        build_sequence: 1001,
        package_sha256: "f".repeat(64),
        package_bytes: 1,
        rpc: 22,
        journal_min: 1,
        journal_max: 11,
        kernel_control: 2,
        runtime,
    };
    let rng = SystemRandom::new();
    let pair = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
    let pair =
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pair.as_ref(), &rng).unwrap();
    let hex = |b: &[u8]| b.iter().map(|v| format!("{v:02x}")).collect::<String>();
    let trust = TrustStore {
        schema: 1,
        keys: vec![PublicKey::P256Sha256 {
            point: hex(pair.public_key().as_ref()),
        }],
    };
    let descriptor = serde_json::to_string(&d).unwrap();
    let mut message = DOMAIN.to_vec();
    message.extend_from_slice(descriptor.as_bytes());
    let signature = pair.sign(&rng, &message).unwrap();
    let descriptor = trust
        .authenticate(
            &serde_json::to_vec(&Envelope {
                descriptor,
                signature: hex(signature.as_ref()),
            })
            .unwrap(),
        )
        .unwrap();
    Fixture {
        root,
        descriptor,
        uid: unsafe { libc::geteuid() },
    }
}
impl Fixture {
    fn inspect(&self) -> io::Result<ImageFence> {
        ImageFence::inspect(
            self.root.path(),
            Some(&self.descriptor),
            Abi::Arm64,
            self.uid,
        )
    }
}
#[test]
fn exact_post_customize_image_accepts_and_live_property_update_is_not_code_change() {
    let f = fixture();
    let fence = f.inspect().unwrap();
    fs::write(
        f.root.path().join("module.prop"),
        b"id=vpnhide_core\nversion=new-but-still-staged\n",
    )
    .unwrap();
    fs::write(f.root.path().join("update"), b"").unwrap();
    fence.require_unchanged_owned(f.root.path(), f.uid).unwrap();
    assert_eq!(fence.files.len(), 16); // daemon, manifest, ten payloads, two autoload mirrors, boot entry/policy
}
#[test]
fn signed_wrong_abi_and_missing_or_mutated_autoload_copy_refuse_staging() {
    let f = fixture();
    assert!(ImageFence::inspect(f.root.path(), Some(&f.descriptor), Abi::Arm32, f.uid).is_err());
    let mirror = f.root.path().join("zygisk/armeabi-v7a.so");
    fs::write(&mirror, b"different library").unwrap();
    assert!(f.inspect().is_err());
    fs::remove_file(mirror).unwrap();
    assert!(f.inspect().is_err());
}
#[test]
fn same_bytes_at_new_executable_inode_break_live_fence_and_do_not_update_running_proof() {
    let f = fixture();
    let fence = f.inspect().unwrap();
    let mut running = File::open(f.root.path().join("vpnhide-core")).unwrap();
    running.seek(std::io::SeekFrom::Start(9)).unwrap();
    fence.require_running_owned(&running, f.uid).unwrap();
    fence.require_running_owned(&running, f.uid).unwrap();
    assert_eq!(running.stream_position().unwrap(), 9);
    let replacement = f.root.path().join("new-executable");
    fs::copy(f.root.path().join("vpnhide-core"), &replacement).unwrap();
    fs::rename(replacement, f.root.path().join("vpnhide-core")).unwrap();
    assert!(fence.require_unchanged_owned(f.root.path(), f.uid).is_err());
    assert!(fence.require_running_owned(&running, f.uid).is_err());
}
#[test]
fn signed_manifest_hash_cannot_be_replaced_by_self_consistent_catalog() {
    let f = fixture();
    let path = f.root.path().join("payloads/manifest.json");
    let mut raw = fs::read(&path).unwrap();
    raw.push(b' ');
    fs::write(&path, raw).unwrap();
    assert!(Catalog::open(f.root.path()).is_ok());
    assert!(f.inspect().is_err());
}
#[test]
fn active_payload_mutation_and_symlink_are_not_accepted_as_unchanged_generation() {
    let f = fixture();
    let fence = f.inspect().unwrap();
    let path = f.root.path().join("payloads/kpm/arm64/vpnhide.kpm");
    let original = fs::read(&path).unwrap();
    fs::write(&path, b"partial replacement").unwrap();
    assert!(fence.require_unchanged_owned(f.root.path(), f.uid).is_err());
    fs::write(&path, &original).unwrap();
    let other = f.root.path().join("other-payload");
    fs::write(&other, original).unwrap();
    fs::remove_file(&path).unwrap();
    symlink(other, &path).unwrap();
    assert!(f.inspect().is_err());
}

#[test]
fn missing_boot_entry_or_policy_cannot_authorize_retirement_of_old_modules() {
    for member in ["service.sh", "sepolicy.rule"] {
        let f = fixture();
        let fence = f.inspect().unwrap();
        fs::remove_file(f.root.path().join(member)).unwrap();
        assert!(f.inspect().is_err());
        assert!(fence.require_unchanged_owned(f.root.path(), f.uid).is_err());
    }
}

#[test]
fn disabled_removed_or_hot_install_staging_is_not_a_ready_replacement() {
    for marker in ["disable", "remove", ".ksu_hot_install"] {
        let f = fixture();
        fs::write(f.root.path().join(marker), b"").unwrap();
        assert!(f.inspect().is_err());
    }
}
