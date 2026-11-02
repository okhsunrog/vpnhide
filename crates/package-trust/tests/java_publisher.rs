//! Actual Java-keystore signing against Rust verification, not shared mock crypto.
use std::{fs, path::Path, process::Command};
use vpnhide_package_trust::{TrustError, TrustStore};

fn run(command: &mut Command) {
    let output = command.output().expect("test tool could not start");
    assert!(
        output.status.success(),
        "fixture tool failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn existing_android_key_types_sign_exact_unified_package_for_rust() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let temp = tempfile::tempdir().unwrap();
    let package = temp.path().join("core.zip");
    run(Command::new("python3").current_dir(repo.join("scripts")).args([
        "-c",
        "from test_payload_bundle import BundleTests; import shutil,sys; c=BundleTests(); c.setUp(); c.build(); shutil.copyfile(c.output,sys.argv[1]); c.doCleanups()",
    ]).arg(&package));
    for (index, algorithm) in ["RSA", "secp256r1", "secp384r1"].into_iter().enumerate() {
        let keystore = temp.path().join(format!("test-only-{index}.p12"));
        let public = temp.path().join(format!("trust-{index}.json"));
        let envelope = temp.path().join(format!("signed-{index}.json"));
        // Disposable test fixture only. Production has no key generation path.
        let mut generate = Command::new("keytool");
        generate
            .args([
                "-genkeypair",
                "-storetype",
                "PKCS12",
                "-alias",
                "fixture",
                "-dname",
                "CN=VPNHide Test Only",
                "-validity",
                "1",
                "-storepass",
                "fixture-only-password",
                "-keypass",
                "fixture-only-password",
                "-keystore",
            ])
            .arg(&keystore);
        if algorithm == "RSA" {
            generate.args(["-keyalg", "RSA", "-keysize", "2048"]);
        } else {
            generate.args(["-keyalg", "EC", "-groupname", algorithm]);
        }
        run(&mut generate);
        let signer = || {
            let mut command = Command::new("python3");
            command
                .arg(repo.join("scripts/core_descriptor.py"))
                .env("KEYSTORE_PASSWORD", "fixture-only-password")
                .env("KEY_ALIAS", "fixture");
            command
        };
        run(signer()
            .args(["public", "--keystore"])
            .arg(&keystore)
            .arg("--output")
            .arg(&public));
        run(signer()
            .args(["sign", "--keystore"])
            .arg(&keystore)
            .arg("--trust")
            .arg(&public)
            .arg("--package")
            .arg(&package)
            .args([
                "--commit",
                &"a".repeat(40),
                "--sequence",
                "1001",
                "--output",
            ])
            .arg(&envelope));
        let trust = TrustStore::parse(&fs::read(&public).unwrap()).unwrap();
        let verified = trust.authenticate(&fs::read(&envelope).unwrap()).unwrap();
        verified
            .verify_package(fs::File::open(&package).unwrap())
            .unwrap();
        assert_eq!(verified.descriptor().build_sequence, 1001);
        let mut forged: serde_json::Value =
            serde_json::from_slice(&fs::read(&envelope).unwrap()).unwrap();
        forged["descriptor"] = serde_json::json!(
            forged["descriptor"]
                .as_str()
                .unwrap()
                .replace("1001", "1002")
        );
        assert!(matches!(
            trust.authenticate(&serde_json::to_vec(&forged).unwrap()),
            Err(TrustError::Signature)
        ));
    }
}
