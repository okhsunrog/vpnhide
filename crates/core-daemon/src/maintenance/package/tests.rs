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
    fixture_at(1001)
}
fn fixture_at(sequence: u64) -> Fixture {
    fixture_payload(sequence, b"package")
}
fn fixture_payload(sequence: u64, payload: &[u8]) -> Fixture {
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
        schema: 2,
        module_id: "vpnhide_core".into(),
        source_commit: "a".repeat(40),
        build_sequence: sequence,
        package_sha256: format!("{:x}", Sha256::digest(payload)),
        package_bytes: payload.len() as u64,
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
        payload,
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
fn accepted_floor_survives_reopen_and_serializes_independent_instances() {
    let f = fixture();
    let persistent = f.temp.path().join("persistent");
    let uid = unsafe { libc::geteuid() };
    let store = FloorStore::open_owned(&persistent, uid).unwrap();
    let second = FloorStore::open_owned(&persistent, uid).unwrap();
    let lease = store.lock().unwrap();
    assert_eq!(lease.read().unwrap(), AcceptedFloor::default());
    assert!(matches!(second.lock(), Err(PackageError::Busy)));
    let prepared = f.stage.copy_verified(&f.verified, 1).unwrap();
    assert_eq!(lease.read().unwrap(), AcceptedFloor::default()); // preparation is inert
    let mut reader = prepared.file();
    reader.seek(std::io::SeekFrom::Start(2)).unwrap();
    lease.accept_prepared(&prepared).unwrap();
    assert_eq!(reader.stream_position().unwrap(), 2);
    lease.accept_prepared(&prepared).unwrap();
    drop(lease);
    drop(store);
    let lease = second.lock().unwrap();
    assert_eq!(
        lease.read().unwrap(),
        AcceptedFloor {
            build_sequence: 1001,
            package_sha256: Some(f.verified.descriptor().package_sha256.clone()),
        }
    );
    assert_eq!(
        fs::metadata(persistent.join("maintenance-floor.json"))
            .unwrap()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn changed_private_package_cannot_advance_accepted_floor() {
    let f = fixture();
    let store = FloorStore::open_owned(&f.temp.path().join("persistent"), unsafe {
        libc::geteuid()
    })
    .unwrap();
    let lease = store.lock().unwrap();
    let prepared = f.stage.copy_verified(&f.verified, 1).unwrap();
    fs::write(f.final_file(), b"changed").unwrap(); // same length, different bytes
    assert!(lease.accept_prepared(&prepared).is_err());
    assert_eq!(lease.read().unwrap(), AcceptedFloor::default());
}

#[test]
fn higher_floor_refuses_older_package_and_cross_directory_tokens() {
    let f = fixture();
    let high = fixture_at(1002);
    let uid = unsafe { libc::geteuid() };
    let store = FloorStore::open_owned(&f.temp.path().join("persistent"), uid).unwrap();
    let lease = store.lock().unwrap();
    let foreign = high.stage.copy_verified(&high.verified, 1).unwrap();
    assert!(matches!(
        lease.accept_prepared(&foreign),
        Err(PackageError::UnsafeStorage)
    ));
    let prepared_high = f.stage.copy_verified(&high.verified, 2).unwrap();
    lease.accept_prepared(&prepared_high).unwrap();
    let prepared_old = f.stage.copy_verified(&f.verified, 3).unwrap();
    assert!(matches!(
        lease.accept_prepared(&prepared_old),
        Err(PackageError::Trust(TrustError::Rollback))
    ));
    assert_eq!(lease.read().unwrap().build_sequence, 1002);
}

#[test]
fn equal_sequence_cannot_change_the_authenticated_package_hash() {
    let f = fixture();
    let collision = fixture_payload(1001, b"different package");
    let store = FloorStore::open_owned(&f.temp.path().join("persistent"), unsafe {
        libc::geteuid()
    })
    .unwrap();
    let lease = store.lock().unwrap();
    let first = f.stage.copy_verified(&f.verified, 1).unwrap();
    lease.accept_prepared(&first).unwrap();
    fs::write(
        f.temp.path().join(format!(
            "app/files/core-maintenance/{}.zip",
            collision.verified.descriptor().package_sha256
        )),
        b"different package",
    )
    .unwrap();
    let conflicting = f.stage.copy_verified(&collision.verified, 2).unwrap();
    assert!(matches!(
        lease.accept_prepared(&conflicting),
        Err(PackageError::Trust(TrustError::Rollback))
    ));
    assert_eq!(
        lease.read().unwrap().package_sha256,
        Some(f.verified.descriptor().package_sha256.clone())
    );
}

#[test]
fn exposing_the_parent_after_lock_never_turns_absence_into_an_empty_floor() {
    let f = fixture();
    let persistent = f.temp.path().join("persistent");
    let store = FloorStore::open_owned(&persistent, unsafe { libc::geteuid() }).unwrap();
    let lease = store.lock().unwrap();
    fs::set_permissions(&persistent, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(matches!(lease.read(), Err(PackageError::UnsafeStorage)));
    fs::set_permissions(&persistent, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn floor_corruption_symlinks_fifos_and_bad_modes_never_become_zero() {
    let f = fixture();
    let persistent = f.temp.path().join("persistent");
    let path = persistent.join("maintenance-floor.json");
    let store = FloorStore::open_owned(&persistent, unsafe { libc::geteuid() }).unwrap();
    let lease = store.lock().unwrap();
    for text in [
        "",
        "{",
        r#"{"schema":2,"floor":{"build_sequence":0,"package_sha256":null}}"#,
        r#"{"schema":1,"floor":{"build_sequence":9,"package_sha256":null}}"#,
    ] {
        fs::write(&path, text).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(lease.read().is_err());
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(lease.read(), Err(PackageError::UnsafeStorage)));
    fs::remove_file(&path).unwrap();
    symlink(f.source(), &path).unwrap();
    assert!(lease.read().is_err());
    fs::remove_file(&path).unwrap();
    let cpath = CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
    assert!(matches!(lease.read(), Err(PackageError::UnsafeStorage)));
    fs::remove_file(&path).unwrap();
    assert_eq!(lease.read().unwrap(), AcceptedFloor::default());
}

#[test]
fn incomplete_floor_copy_is_inert_and_only_a_safe_regular_temporary_is_recovered() {
    let f = fixture();
    let persistent = f.temp.path().join("persistent");
    let next = persistent.join("maintenance-floor.next");
    let store = FloorStore::open_owned(&persistent, unsafe { libc::geteuid() }).unwrap();
    let lease = store.lock().unwrap();
    fs::write(&next, b"partial").unwrap();
    fs::set_permissions(&next, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(lease.read().unwrap(), AcceptedFloor::default());
    let prepared = f.stage.copy_verified(&f.verified, 1).unwrap();
    lease.accept_prepared(&prepared).unwrap();
    assert!(!next.exists());
    let high = fixture_at(1002);
    let prepared = f.stage.copy_verified(&high.verified, 2).unwrap();
    symlink(f.source(), &next).unwrap();
    assert!(lease.accept_prepared(&prepared).is_err());
    assert_eq!(lease.read().unwrap().build_sequence, 1001);
}

#[test]
fn post_rename_floor_sync_failure_remains_uncertain_and_reconfirms_same_package() {
    let f = fixture();
    let store = FloorStore::open_owned(&f.temp.path().join("persistent"), unsafe {
        libc::geteuid()
    })
    .unwrap();
    let lease = store.lock().unwrap();
    let prepared = f.stage.copy_verified(&f.verified, 1).unwrap();
    store.fail_next_sync();
    assert!(matches!(
        lease.accept_prepared(&prepared),
        Err(PackageError::FloorDurabilityUnknown(_))
    ));
    assert_eq!(lease.read().unwrap().build_sequence, 1001);
    lease.accept_prepared(&prepared).unwrap();
    assert_eq!(lease.read().unwrap().build_sequence, 1001);
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
        payload_manifest: item.clone(),
        service: item.clone(),
        sepolicy: item.clone(),
        module_prop: item,
    }
}
