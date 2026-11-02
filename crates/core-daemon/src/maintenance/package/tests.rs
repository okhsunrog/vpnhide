use super::*;
use ring::{
    rand::SystemRandom,
    signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair},
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};
use vpnhide_package_trust::{DOMAIN, Envelope, PublicKey};

struct Fixture {
    temp: tempfile::TempDir,
    stage: PackageStaging,
    verified: VerifiedDescriptor,
}
fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let persistent = temp.path().join("persistent");
    let app = temp.path().join("app");
    fs::create_dir(&persistent).unwrap();
    fs::set_permissions(&persistent, fs::Permissions::from_mode(0o700)).unwrap();
    fs::create_dir_all(app.join("files/core-maintenance")).unwrap();
    let uid = unsafe { libc::geteuid() };
    let stage = PackageStaging::open_owned(&persistent, &app, uid, uid).unwrap();
    let rng = SystemRandom::new();
    let key = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
    let key =
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, key.as_ref(), &rng).unwrap();
    let hex = |b: &[u8]| b.iter().map(|v| format!("{v:02x}")).collect::<String>();
    let trust = TrustStore {
        schema: 1,
        keys: vec![PublicKey::P256Sha256 {
            point: hex(key.public_key().as_ref()),
        }],
    };
    let descriptor = serde_json::to_string(&Descriptor {
        schema: 1,
        module_id: "vpnhide_core".into(),
        source_commit: "a".repeat(40),
        build_sequence: 1001,
        package_sha256: format!("{:x}", Sha256::digest(b"package")),
        package_bytes: 7,
        rpc: 22,
        journal_min: 1,
        journal_max: 11,
        kernel_control: 2,
        runtime: vpnhide_package_trust_runtime(),
    })
    .unwrap();
    let mut message = DOMAIN.to_vec();
    message.extend_from_slice(descriptor.as_bytes());
    let signature = key.sign(&rng, &message).unwrap();
    let verified = trust
        .authenticate(
            &serde_json::to_vec(&Envelope {
                descriptor,
                signature: hex(signature.as_ref()),
            })
            .unwrap(),
        )
        .unwrap();
    fs::write(
        app.join(format!(
            "files/core-maintenance/{}.zip",
            verified.descriptor().package_sha256
        )),
        b"package",
    )
    .unwrap();
    Fixture {
        temp,
        stage,
        verified,
    }
}
impl Fixture {
    fn source(&self) -> std::path::PathBuf {
        self.temp.path().join(format!(
            "app/files/core-maintenance/{}.zip",
            self.verified.descriptor().package_sha256
        ))
    }
    fn final_file(&self) -> std::path::PathBuf {
        self.temp.path().join(format!(
            "persistent/maintenance-packages/{}.zip",
            self.verified.descriptor().package_sha256
        ))
    }
    fn count_files(&self) -> usize {
        fs::read_dir(self.temp.path().join("persistent/maintenance-packages"))
            .unwrap()
            .filter(|v| v.as_ref().is_ok_and(|e| e.file_name() != "staging.lock"))
            .count()
    }
}
#[test]
fn authenticated_copy_is_private_idempotent_and_independent_of_app_mutation() {
    let f = fixture();
    let mut prepared = f.stage.copy_verified(&f.verified, 1).unwrap();
    fs::write(f.source(), b"changed").unwrap();
    let mut bytes = Vec::new();
    prepared.file.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"package");
    assert_eq!(prepared.file.metadata().unwrap().mode() & 0o777, 0o600);
    let again = f.stage.copy_verified(&f.verified, 2).unwrap();
    assert_eq!(
        again.file.metadata().unwrap().ino(),
        prepared.file.metadata().unwrap().ino()
    );
    assert_eq!(f.count_files(), 1);
}
#[test]
fn tampered_or_oversized_source_never_enters_private_packages() {
    let f = fixture();
    for bytes in [&b"wrong!!"[..], &b"package-extra"[..]] {
        fs::write(f.source(), bytes).unwrap();
        assert!(f.stage.copy_verified(&f.verified, 2).is_err());
        assert!(!f.final_file().exists());
        assert_eq!(f.count_files(), 0);
    }
}
#[test]
fn source_symlink_and_hardlink_refused_without_following_or_cleanup_of_target() {
    let f = fixture();
    let target = f.temp.path().join("other");
    fs::write(&target, b"package").unwrap();
    fs::remove_file(f.source()).unwrap();
    symlink(&target, f.source()).unwrap();
    assert!(f.stage.copy_verified(&f.verified, 1).is_err());
    fs::remove_file(f.source()).unwrap();
    fs::hard_link(&target, f.source()).unwrap();
    assert!(matches!(
        f.stage.copy_verified(&f.verified, 2),
        Err(PackageError::SourceChanged)
    ));
    assert_eq!(fs::read(target).unwrap(), b"package");
    assert_eq!(f.count_files(), 0);
}
#[test]
fn final_symlink_cannot_be_accepted_or_overwritten() {
    let f = fixture();
    let target = f.temp.path().join("other");
    fs::write(&target, b"package").unwrap();
    symlink(&target, f.final_file()).unwrap();
    assert!(f.stage.copy_verified(&f.verified, 1).is_err());
    assert!(f.final_file().is_symlink());
    assert_eq!(fs::read(target).unwrap(), b"package");
}
#[test]
fn existing_corrupt_private_file_stays_an_error_and_is_not_replaced() {
    let f = fixture();
    fs::write(f.final_file(), b"corrupt").unwrap();
    fs::set_permissions(f.final_file(), fs::Permissions::from_mode(0o600)).unwrap();
    assert!(matches!(
        f.stage.copy_verified(&f.verified, 1),
        Err(PackageError::Trust(TrustError::PackageHash))
    ));
    assert_eq!(fs::read(f.final_file()).unwrap(), b"corrupt");
    assert_eq!(f.count_files(), 1);
}
#[test]
fn exposed_private_directory_and_changed_app_owner_fail_closed() {
    let f = fixture();
    fs::set_permissions(
        f.final_file().parent().unwrap(),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert!(matches!(
        f.stage.copy_verified(&f.verified, 1),
        Err(PackageError::UnsafeStorage)
    ));
    let uid = unsafe { libc::geteuid() };
    assert!(matches!(
        PackageStaging::open_owned(
            &f.temp.path().join("persistent"),
            &f.temp.path().join("app"),
            uid.wrapping_add(1),
            uid
        ),
        Err(PackageError::UnsafeStorage)
    ));
}

#[test]
fn interrupted_inert_copy_is_recovered_only_under_exclusive_lease() {
    let f = fixture();
    let copy = f.final_file().with_file_name("copy-99");
    fs::write(&copy, b"interrupted").unwrap();
    fs::set_permissions(&copy, fs::Permissions::from_mode(0o600)).unwrap();
    let lease = f.stage.lease().unwrap();
    assert!(matches!(
        f.stage.copy_verified(&f.verified, 1),
        Err(PackageError::Busy)
    ));
    assert!(copy.exists());
    drop(lease);
    f.stage.copy_verified(&f.verified, 1).unwrap();
    assert!(!copy.exists());
    assert_eq!(f.count_files(), 1);
}
#[test]
fn storage_bound_preserves_previous_plan_packages_without_silent_gc() {
    let f = fixture();
    for id in ["b".repeat(64), "c".repeat(64)] {
        let path = f.final_file().with_file_name(format!("{id}.zip"));
        fs::write(&path, b"another pending plan").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert!(matches!(
        f.stage.copy_verified(&f.verified, 1),
        Err(PackageError::StorageFull)
    ));
    assert!(!f.final_file().exists());
    assert_eq!(f.count_files(), 2);
}
#[test]
fn fifo_source_and_private_destination_do_not_block_or_become_packages() {
    use std::os::unix::ffi::OsStrExt;
    let f = fixture();
    fs::remove_file(f.source()).unwrap();
    let name = CString::new(f.source().as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(f.stage.copy_verified(&f.verified, 1).is_err());
    fs::remove_file(f.source()).unwrap();
    fs::write(f.source(), b"package").unwrap();
    let name = CString::new(f.final_file().as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(f.stage.copy_verified(&f.verified, 2).is_err());
    assert_eq!(f.count_files(), 1);
}

fn vpnhide_package_trust_runtime() -> vpnhide_package_trust::RuntimeIdentity {
    let item = vpnhide_package_trust::FileIdentity {
        bytes: 7,
        sha256: format!("{:x}", Sha256::digest(b"package")),
    };
    vpnhide_package_trust::RuntimeIdentity {
        daemon_arm64: item.clone(),
        daemon_arm: item.clone(),
        payload_manifest: item,
    }
}
