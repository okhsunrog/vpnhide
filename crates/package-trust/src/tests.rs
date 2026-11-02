use super::*;
use ring::{
    rand::SystemRandom,
    signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair},
};
use sha2::{Digest, Sha256};

fn fixture() -> (TrustStore, EcdsaKeyPair, Descriptor) {
    let rng = SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
    let pair =
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng).unwrap();
    let trust = TrustStore {
        schema: 1,
        keys: vec![PublicKey::P256Sha256 {
            point: hex(pair.public_key().as_ref()),
        }],
    };
    let descriptor = Descriptor {
        schema: 2,
        module_id: "vpnhide_core".into(),
        source_commit: "a".repeat(40),
        build_sequence: 42,
        package_sha256: format!("{:x}", Sha256::digest(b"package")),
        package_bytes: 7,
        rpc: 22,
        journal_min: 1,
        journal_max: 11,
        kernel_control: 2,
        runtime: vpnhide_package_trust_runtime(),
    };
    (trust, pair, descriptor)
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn envelope(pair: &EcdsaKeyPair, descriptor: &Descriptor) -> Vec<u8> {
    signed(pair, serde_json::to_string(descriptor).unwrap())
}
fn signed(pair: &EcdsaKeyPair, descriptor: String) -> Vec<u8> {
    let mut message = DOMAIN.to_vec();
    message.extend_from_slice(descriptor.as_bytes());
    let signature = pair.sign(&SystemRandom::new(), &message).unwrap();
    serde_json::to_vec(&Envelope {
        descriptor,
        signature: hex(signature.as_ref()),
    })
    .unwrap()
}
fn target(d: &Descriptor) -> Target {
    Target {
        source_commit: d.source_commit.clone(),
        package_sha256: d.package_sha256.clone(),
        rpc: d.rpc,
        current_journal: 11,
    }
}
#[test]
fn authenticated_exact_package_and_idempotent_floor() {
    let (trust, pair, d) = fixture();
    let verified = trust.authenticate(&envelope(&pair, &d)).unwrap();
    verified.require_build_at_least(42).unwrap();
    assert_eq!(
        verified.require_build_at_least(43),
        Err(TrustError::Rollback)
    );
    verified
        .admit(&target(&d), &AcceptedFloor::default())
        .unwrap();
    verified
        .admit(
            &target(&d),
            &AcceptedFloor {
                build_sequence: 42,
                package_sha256: Some(d.package_sha256.clone()),
            },
        )
        .unwrap();
    verified.verify_package(&b"package"[..]).unwrap();
    assert_eq!(
        verified.verify_package(&b"packagf"[..]),
        Err(TrustError::PackageHash)
    );
    for data in [&b"packag"[..], &b"packages"[..]] {
        assert_eq!(verified.verify_package(data), Err(TrustError::PackageSize));
    }
}
#[test]
fn signature_cannot_select_replacement_key_or_normalize_descriptor_bytes() {
    let (trust, pair, d) = fixture();
    let (_, other, _) = fixture();
    assert!(matches!(
        trust.authenticate(&envelope(&other, &d)),
        Err(TrustError::Signature)
    ));
    let mut value: Envelope = serde_json::from_slice(&envelope(&pair, &d)).unwrap();
    value.descriptor.push(' ');
    assert!(matches!(
        trust.authenticate(&serde_json::to_vec(&value).unwrap()),
        Err(TrustError::Signature)
    ));
    let mut body: serde_json::Value = serde_json::from_slice(&envelope(&pair, &d)).unwrap();
    body["key"] = serde_json::json!(trust.keys[0]);
    assert!(matches!(
        trust.authenticate(&serde_json::to_vec(&body).unwrap()),
        Err(TrustError::Malformed)
    ));
}
#[test]
fn admission_refuses_replay_wrong_target_and_incompatible_journal_rpc() {
    let (trust, pair, d) = fixture();
    let v = trust.authenticate(&envelope(&pair, &d)).unwrap();
    for floor in [
        AcceptedFloor {
            build_sequence: 43,
            package_sha256: Some(d.package_sha256.clone()),
        },
        AcceptedFloor {
            build_sequence: 42,
            package_sha256: Some("f".repeat(64)),
        },
    ] {
        assert_eq!(v.admit(&target(&d), &floor), Err(TrustError::Rollback));
    }
    for field in [0, 1] {
        let mut t = target(&d);
        if field == 0 {
            t.source_commit = "b".repeat(40);
        } else {
            t.package_sha256 = "b".repeat(64);
        }
        assert_eq!(
            v.admit(&t, &AcceptedFloor::default()),
            Err(TrustError::WrongTarget)
        );
    }
    for field in [0, 1] {
        let mut t = target(&d);
        if field == 0 {
            t.rpc += 1;
        } else {
            t.current_journal = 12;
        }
        assert_eq!(
            v.admit(&t, &AcceptedFloor::default()),
            Err(TrustError::Compatibility)
        );
    }
}
#[test]
fn bounded_strict_parse_and_signed_invalid_metadata_fail_closed() {
    let (trust, pair, d) = fixture();
    assert!(matches!(
        TrustStore::parse(br#"{"schema":1,"keys":[]}"#),
        Err(TrustError::Unconfigured)
    ));
    assert!(matches!(
        trust.authenticate(&vec![0; MAX_ENVELOPE_BYTES + 1]),
        Err(TrustError::Malformed)
    ));
    for field in [
        "schema",
        "module_id",
        "source_commit",
        "build_sequence",
        "package_bytes",
        "rpc",
        "kernel_control",
        "journal_min",
    ] {
        let mut value = serde_json::to_value(&d).unwrap();
        value[field] = if ["module_id", "source_commit"].contains(&field) {
            serde_json::json!("../other")
        } else {
            serde_json::json!(0)
        };
        assert!(
            matches!(
                trust.authenticate(&signed(&pair, value.to_string())),
                Err(TrustError::Malformed)
            ),
            "{field}"
        );
    }
    let duplicate = serde_json::to_string(&d)
        .unwrap()
        .replacen('{', "{\"schema\":1,", 1);
    assert!(matches!(
        trust.authenticate(&signed(&pair, duplicate)),
        Err(TrustError::Malformed)
    ));
    let mut older = d;
    older.schema = 1;
    assert!(matches!(
        trust.authenticate(&envelope(&pair, &older)),
        Err(TrustError::Malformed)
    ));
}

fn vpnhide_package_trust_runtime() -> RuntimeIdentity {
    let item = FileIdentity {
        bytes: 7,
        sha256: format!("{:x}", Sha256::digest(b"package")),
    };
    RuntimeIdentity {
        daemon_arm64: item.clone(),
        daemon_arm: item.clone(),
        payload_manifest: item.clone(),
        service: item.clone(),
        sepolicy: item.clone(),
        module_prop: item,
    }
}
