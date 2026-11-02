use std::{collections::BTreeMap, fs, path::Path, process::Command};
use vpnhide_artifact_model::{
    archive::{self, FileIdentity},
    fixtures,
    manifest::KMIS,
};
fn tool(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_vpnhide-tooling"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn producer(root: &Path, name: &str, component: &str, entries: Vec<(&str, Vec<u8>)>) {
    let mut files: BTreeMap<String, Vec<u8>> =
        entries.into_iter().map(|(n, b)| (n.into(), b)).collect();
    let identities: BTreeMap<_, _> = files
        .iter()
        .map(|(n, b)| (n.clone(), FileIdentity::of(b)))
        .collect();
    let meta = serde_json::json!({"schema":1,"component":component,"version":"test","dirty":false,"source_commit":"a".repeat(40),"files":identities});
    files.insert(
        "build-provenance.json".into(),
        serde_json::to_vec(&meta).unwrap(),
    );
    archive::write_archive(&root.join(name), &files).unwrap();
}
#[test]
fn basename_assembly_output_succeeds_and_failed_replacement_keeps_previous_bytes() {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    let golden = fixtures::bundle_files().unwrap();
    producer(
        r,
        "vpnhide-core-base.zip",
        "core",
        vec![
            ("module.prop", b"id=vpnhide_core\n".to_vec()),
            (
                "bin/arm64/vpnhide-core",
                golden["bin/arm64/vpnhide-core"].clone(),
            ),
            (
                "bin/arm/vpnhide-core",
                golden["bin/arm/vpnhide-core"].clone(),
            ),
            ("service.sh", golden["service.sh"].clone()),
            ("sepolicy.rule", golden["sepolicy.rule"].clone()),
        ],
    );
    for kmi in KMIS {
        producer(
            r,
            &format!("vpnhide-kmod-{kmi}.zip"),
            &format!("kmod:{kmi}"),
            vec![(
                "vpnhide_kmod.ko",
                golden[&format!("payloads/kmod/{kmi}/vpnhide_kmod.ko")].clone(),
            )],
        );
    }
    producer(
        r,
        "vpnhide-kpm.zip",
        "kpm",
        vec![(
            "vpnhide.kpm",
            golden["payloads/kpm/arm64/vpnhide.kpm"].clone(),
        )],
    );
    producer(
        r,
        "vpnhide-zygisk.zip",
        "zygisk",
        vec![
            ("zygisk/arm64-v8a.so", golden["zygisk/arm64-v8a.so"].clone()),
            (
                "zygisk/armeabi-v7a.so",
                golden["zygisk/armeabi-v7a.so"].clone(),
            ),
        ],
    );
    let args = [
        "bundle",
        "assemble",
        "--inputs",
        ".",
        "--output",
        "core.zip",
        "--commit",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ];
    let output = tool(r, &args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let previous = fs::read(r.join("core.zip")).unwrap();
    fs::remove_file(r.join("vpnhide-kpm.zip")).unwrap();
    assert!(!tool(r, &args).status.success());
    assert_eq!(fs::read(r.join("core.zip")).unwrap(), previous);
}
#[test]
fn cli_actions_distribution_excludes_internal_artifacts_and_updates_only_core_feed() {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    for name in [
        "vpnhide.apk",
        "vpnhide-core.zip",
        "vpnhide-core.descriptor.json",
        "vpnhide-kpm.zip",
    ] {
        fs::write(r.join(name), b"producer fixture").unwrap();
    }
    assert!(
        tool(
            r,
            &[
                "distribution",
                "select",
                "--root",
                ".",
                "--github-output",
                "actions"
            ]
        )
        .status
        .success()
    );
    assert_eq!(
        fs::read_to_string(r.join("actions")).unwrap(),
        "files<<VPNHIDE_PUBLIC_ASSETS\n./vpnhide.apk\n./vpnhide-core.zip\n./vpnhide-core.descriptor.json\nVPNHIDE_PUBLIC_ASSETS\n"
    );
    fs::write(r.join("VERSION"), b"2.0.0\n").unwrap();
    fs::create_dir(r.join("update-json")).unwrap();
    fs::write(r.join("update-json/update-kpm.json"), b"frozen").unwrap();
    assert!(tool(r, &["distribution", "update-feed"]).status.success());
    assert_eq!(
        fs::read(r.join("update-json/update-kpm.json")).unwrap(),
        b"frozen"
    );
    assert!(
        fs::read_to_string(r.join("update-json/update-core.json"))
            .unwrap()
            .contains("/vpnhide-core.zip")
    );
}
#[test]
fn unavailable_signing_identity_never_generates_a_key_or_replaces_previous_output() {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    fs::write(r.join("previous"), b"prior verified output").unwrap();
    let output = tool(
        r,
        &[
            "signer",
            "public",
            "--keystore",
            "missing.p12",
            "--output",
            "previous",
        ],
    );
    assert!(!output.status.success());
    assert_eq!(
        fs::read(r.join("previous")).unwrap(),
        b"prior verified output"
    );
    assert!(!r.join("missing.p12").exists());
}
#[test]
fn device_reset_default_is_an_offline_plan_without_adb_or_receipt_writes() {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    let output = Command::new(env!("CARGO_BIN_EXE_vpnhide-tooling"))
        .current_dir(r)
        .env("PATH", r)
        .args(["device-reset", "--adb", "missing-adb"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(plan["action"], "core_full_reset");
    assert_eq!(fs::read_dir(r).unwrap().count(), 0);
    let no_target = tool(r, &["device-reset", "--execute"]);
    assert!(!no_target.status.success());
    assert_eq!(fs::read_dir(r).unwrap().count(), 0);
}
