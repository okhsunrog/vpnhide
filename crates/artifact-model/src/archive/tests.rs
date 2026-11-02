use super::*;
use crate::fixtures::{init_contract_payload, relocatable};
use serde_json::{Value, json};
use zip::ZipArchive;

const COMMIT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
// An independent stored-ZIP encoder permits duplicate names, symlinks, encrypted
// flags and forged sizes which the production writer deliberately cannot emit.
fn raw_zip(entries: &[(String, Vec<u8>, u32, u16)]) -> Vec<u8> {
    let mut data = Vec::new();
    let mut central = Vec::new();
    for (name, bytes, mode, flags) in entries {
        let offset = data.len() as u32;
        let checksum = crc32fast::hash(bytes);
        data.extend_from_slice(&0x04034b50u32.to_le_bytes());
        for value in [20u16, *flags, 0, 0, 0] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        for value in [checksum, bytes.len() as u32, bytes.len() as u32] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        data.extend_from_slice(&(name.len() as u16).to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(name.as_bytes());
        data.extend_from_slice(bytes);
        central.extend_from_slice(&0x02014b50u32.to_le_bytes());
        for value in [0x0314u16, 20, *flags, 0, 0, 0] {
            central.extend_from_slice(&value.to_le_bytes());
        }
        for value in [checksum, bytes.len() as u32, bytes.len() as u32] {
            central.extend_from_slice(&value.to_le_bytes());
        }
        for value in [name.len() as u16, 0, 0, 0, 0] {
            central.extend_from_slice(&value.to_le_bytes());
        }
        central.extend_from_slice(&(mode << 16).to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let central_offset = data.len() as u32;
    data.extend_from_slice(&central);
    data.extend_from_slice(&0x06054b50u32.to_le_bytes());
    for value in [0u16, 0, entries.len() as u16, entries.len() as u16] {
        data.extend_from_slice(&value.to_le_bytes());
    }
    data.extend_from_slice(&(central.len() as u32).to_le_bytes());
    data.extend_from_slice(&central_offset.to_le_bytes());
    data.extend_from_slice(&0u16.to_le_bytes());
    data
}
fn architecture(machine: u16) -> Vec<u8> {
    let mut header = vec![0; 64];
    header[..7].copy_from_slice(&[
        0x7f,
        b'E',
        b'L',
        b'F',
        if machine == 183 { 2 } else { 1 },
        1,
        1,
    ]);
    header[16..18].copy_from_slice(&3u16.to_le_bytes());
    header[18..20].copy_from_slice(&machine.to_le_bytes());
    header
}
fn producer(path: &Path, component: &str, files: &[(&str, Vec<u8>)], overrides: Value) {
    let mut meta = json!({"schema":1,"component":component,"source_commit":COMMIT,"dirty":false,"version":"test","files":{}});
    let mut entries = Vec::new();
    for (name, bytes) in files {
        meta["files"][name] =
            json!({"bytes":bytes.len(), "sha256":format!("{:x}",Sha256::digest(bytes))});
        entries.push(((*name).to_string(), bytes.clone(), 0o100644, 0));
    }
    for (key, value) in overrides.as_object().unwrap() {
        meta[key] = value.clone();
    }
    entries.push((
        PROVENANCE.into(),
        serde_json::to_vec(&meta).unwrap(),
        0o100644,
        0,
    ));
    fs::write(path, raw_zip(&entries)).unwrap();
}
struct Bundle {
    root: tempfile::TempDir,
}
impl Bundle {
    fn new() -> Self {
        let result = Self {
            root: tempfile::tempdir().unwrap(),
        };
        result.core(&[
            ("module.prop", b"id=vpnhide_core\n".to_vec()),
            ("bin/arm64/vpnhide-core", architecture(183)),
            ("bin/arm/vpnhide-core", architecture(40)),
            ("service.sh", b"exec core service\n".to_vec()),
            ("sepolicy.rule", b"# synthetic fixture policy\n".to_vec()),
        ]);
        for kmi in KMIS {
            producer(
                &result.path(&format!("vpnhide-kmod-{kmi}.zip")),
                &format!("kmod:{kmi}"),
                &[("vpnhide_kmod.ko", init_contract_payload(true))],
                json!({}),
            );
        }
        result.kpm(json!({}));
        producer(
            &result.path("vpnhide-zygisk.zip"),
            "zygisk",
            &[
                ("zygisk/arm64-v8a.so", architecture(183)),
                ("zygisk/armeabi-v7a.so", architecture(40)),
            ],
            json!({}),
        );
        result
    }
    fn path(&self, name: &str) -> std::path::PathBuf {
        self.root.path().join(name)
    }
    fn core(&self, files: &[(&str, Vec<u8>)]) {
        producer(
            &self.path("vpnhide-core-base.zip"),
            "core",
            files,
            json!({}),
        );
    }
    fn kpm(&self, overrides: Value) {
        producer(
            &self.path("vpnhide-kpm.zip"),
            "kpm",
            &[("vpnhide.kpm", architecture(183))],
            overrides,
        );
    }
    fn build(&self) -> io::Result<()> {
        assemble(self.root.path(), &self.path("unified.zip"), COMMIT)
    }
}
#[test]
fn complete_bundle_retains_exact_golden_payloads_with_inert_autoload_and_no_backend_scripts() {
    let b = Bundle::new();
    b.build().unwrap();
    let files = read_archive(&b.path("unified.zip"), 128).unwrap();
    let m: Manifest = metadata(&files, "payloads/manifest.json").unwrap();
    m.require_complete().unwrap();
    assert_eq!(m.bundle_commit, COMMIT);
    assert_eq!(
        files["module.prop"],
        b"id=vpnhide_core\nvpnhideControl=2\nvpnhideActivation=1\n"
    );
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        assert_eq!(
            files[&format!("zygisk/{abi}.so")],
            files[&format!("payloads/zygisk/{abi}.so")]
        );
    }
    assert!(!files.contains_key("zygisk-activation.txt"));
    assert!(!files.contains_key("targets.txt"));
    assert_eq!(
        files.keys().filter(|n| n.ends_with("module.prop")).count(),
        1
    );
    let golden = tempfile::tempdir().unwrap();
    crate::fixtures::write_bundle_tree(golden.path()).unwrap();
    for p in &m.payloads {
        let path = p.implementation.relative_path().unwrap();
        assert_eq!(files[&path], fs::read(golden.path().join(path)).unwrap());
        assert_eq!(p.control_version, 2);
        assert_eq!(p.source_commit, COMMIT);
        if matches!(p.implementation, PayloadKind::Kmod { .. }) {
            assert_eq!((p.preflight_protocol, p.init_contract), (Some(1), Some(1)));
        }
    }
    let mut zip = ZipArchive::new(File::open(b.path("unified.zip")).unwrap()).unwrap();
    assert_eq!(
        zip.by_name("bin/arm64/vpnhide-core")
            .unwrap()
            .unix_mode()
            .unwrap()
            & 0o777,
        0o755
    );
}
#[test]
fn every_missing_component_preserves_previous_output() {
    let b = Bundle::new();
    let output = b.path("unified.zip");
    fs::write(&output, b"previous good output").unwrap();
    for entry in fs::read_dir(b.root.path()).unwrap() {
        let source = entry.unwrap().path();
        if source == output {
            continue;
        }
        let saved = source.with_extension("saved");
        fs::rename(&source, &saved).unwrap();
        assert!(b.build().is_err());
        assert_eq!(fs::read(&output).unwrap(), b"previous good output");
        fs::rename(saved, source).unwrap();
    }
}
#[test]
fn aggregate_retained_input_budget_is_shared_and_preserves_previous_output() {
    let b = Bundle::new();
    let total = fs::read_dir(b.root.path())
        .unwrap()
        .map(|e| {
            read_archive(&e.unwrap().path(), 64)
                .unwrap()
                .values()
                .map(|v| v.len() as u64)
                .sum::<u64>()
        })
        .sum::<u64>();
    assert!(total > 0 && total < 128 * 1024);
    assemble_with_budget(b.root.path(), &b.path("unified.zip"), COMMIT, total).unwrap();
    let previous = fs::read(b.path("unified.zip")).unwrap();
    for limit in [0, total / 2, total - 1] {
        assert!(
            assemble_with_budget(b.root.path(), &b.path("unified.zip"), COMMIT, limit).is_err()
        );
        assert_eq!(fs::read(b.path("unified.zip")).unwrap(), previous);
    }
}
#[test]
fn provenance_source_dirty_component_hash_and_uncovered_bytes_refuse() {
    let b = Bundle::new();
    for mutation in [
        json!({"source_commit":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}),
        json!({"dirty":true}),
        json!({"component":"other"}),
        json!({"files":{"vpnhide.kpm":{"bytes":64,"sha256":"0".repeat(64)}}}),
        json!({"files":{}}),
        json!({"unknown":true}),
    ] {
        b.kpm(mutation);
        assert!(b.build().is_err());
        assert!(!b.path("unified.zip").exists());
    }
}
#[test]
fn wrong_abi_missing_library_and_payloads_in_base_refuse() {
    let b = Bundle::new();
    for files in [
        vec![("zygisk/arm64-v8a.so", architecture(183))],
        vec![
            ("zygisk/arm64-v8a.so", architecture(183)),
            ("zygisk/armeabi-v7a.so", architecture(183)),
        ],
    ] {
        producer(&b.path("vpnhide-zygisk.zip"), "zygisk", &files, json!({}));
        assert!(b.build().is_err());
    }
    b.core(&[("zygisk/arm64-v8a.so", architecture(183))]);
    assert!(b.build().is_err());
}
#[test]
fn legacy_and_forged_capabilities_preserve_previous_bundle() {
    let b = Bundle::new();
    b.build().unwrap();
    let previous = fs::read(b.path("unified.zip")).unwrap();
    for bytes in [
        relocatable(b"license=GPL\0", &[]),
        relocatable(b"vpnhide_preflight=1\0", &[]),
        init_contract_payload(false),
    ] {
        producer(
            &b.path(&format!("vpnhide-kmod-{}.zip", KMIS[0])),
            &format!("kmod:{}", KMIS[0]),
            &[("vpnhide_kmod.ko", bytes)],
            json!({}),
        );
        assert!(b.build().is_err());
        assert_eq!(fs::read(b.path("unified.zip")).unwrap(), previous);
    }
}
#[test]
fn unsafe_paths_symlinks_encryption_duplicates_and_forged_sizes_refuse_before_read() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("invalid.zip");
    for name in ["../escape", "/absolute", "a//b", "a/./b", "a\\b", "a\nline"] {
        fs::write(&path, raw_zip(&[(name.into(), vec![1], 0o100644, 0)])).unwrap();
        assert!(read_archive(&path, 64).is_err());
    }
    for (mode, flags) in [(0o120777, 0), (0o020600, 0), (0o100644, 1)] {
        fs::write(&path, raw_zip(&[("payload".into(), vec![1], mode, flags)])).unwrap();
        assert!(read_archive(&path, 64).is_err());
    }
    let duplicate = ("same".into(), vec![1], 0o100644, 0);
    fs::write(&path, raw_zip(&[duplicate.clone(), duplicate])).unwrap();
    assert!(read_archive(&path, 64).is_err());
    let mut bytes = raw_zip(&[("large".into(), vec![1], 0o100644, 0)]);
    let at = bytes.windows(4).position(|v| v == b"PK\x01\x02").unwrap();
    bytes[at + 24..at + 28].copy_from_slice(&(129u32 * 1024 * 1024).to_le_bytes());
    fs::write(&path, bytes).unwrap();
    assert!(read_archive(&path, 64).is_err());
}
#[test]
fn excessive_declared_entries_refuse_before_attempting_to_decode_members() {
    // Each invalid member is unsupported/encrypted: a limit error must precede
    // any attempt to open it. The fixture is small, independent of allocation.
    let entries = (0..65)
        .map(|i| (format!("member-{i}"), vec![1], 0o100644, 1))
        .collect::<Vec<_>>();
    let bytes = raw_zip(&entries);
    let error = read_archive_bytes(&bytes, 64).unwrap_err();
    assert_eq!(error.to_string(), "duplicate or excessive archive members");
}
#[test]
fn producer_stamp_uses_actual_bytes_rejects_symlinks_and_invalid_identity() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("payload"), b"exact").unwrap();
    stamp(root.path(), "test", "v1", COMMIT, false).unwrap();
    let p: Provenance =
        serde_json::from_slice(&fs::read(root.path().join(PROVENANCE)).unwrap()).unwrap();
    assert_eq!(p.files["payload"], FileIdentity::of(b"exact"));
    assert!(!p.dirty);
    assert!(stamp(root.path(), "test", "v1", "unknown", false).is_err());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("payload", root.path().join("link")).unwrap();
        assert!(stamp(root.path(), "test", "v1", COMMIT, false).is_err());
    }
}

#[test]
fn descriptor_binds_exact_package_runtime_and_typed_compatibility() {
    let b = Bundle::new();
    b.build().unwrap();
    let p = b.path("unified.zip");
    let d = descriptor(&p, COMMIT, 1001).unwrap();
    assert_eq!(d.package_bytes, fs::metadata(&p).unwrap().len());
    assert_eq!(
        d.package_sha256,
        format!("{:x}", Sha256::digest(fs::read(&p).unwrap()))
    );
    assert_eq!(d.rpc, vpnhide_build_contract::RPC_VERSION);
    assert_eq!(d.journal_max, vpnhide_build_contract::JOURNAL_VERSION);
    assert_eq!(d.kernel_control, vpnhide_protocol::CONTROL_VERSION);
    assert_eq!(d.runtime.service, FileIdentity::of(b"exec core service\n"));
    assert!(descriptor(&p, COMMIT, 0).is_err());
    assert!(descriptor(&p, &"b".repeat(40), 1001).is_err());
    assert!(descriptor(&b.path("vpnhide-core-base.zip"), COMMIT, 1001).is_err());
}
#[test]
fn descriptor_refuses_incomplete_duplicate_forged_payload_and_runtime_mirrors() {
    let b = Bundle::new();
    b.build().unwrap();
    let original = read_archive(&b.path("unified.zip"), 128).unwrap();
    for mutation in [
        "missing",
        "duplicate",
        "bytes",
        "mirror",
        "source",
        "runtime",
        "module-id",
        "unsafe-path",
    ] {
        let mut files = original.clone();
        let mut m: serde_json::Value =
            serde_json::from_slice(&files["payloads/manifest.json"]).unwrap();
        match mutation {
            "missing" => {
                m["payloads"].as_array_mut().unwrap().pop();
            }
            "duplicate" => m["payloads"][1] = m["payloads"][0].clone(),
            "bytes" => {
                files.insert(
                    "payloads/kpm/arm64/vpnhide.kpm".into(),
                    b"different".to_vec(),
                );
            }
            "mirror" => {
                files.insert("zygisk/armeabi-v7a.so".into(), b"different".to_vec());
            }
            "source" => m["payloads"][0]["source_commit"] = json!("b".repeat(40)),
            "runtime" => {
                files.remove("service.sh");
            }
            "module-id" => {
                files.insert(
                    "module.prop".into(),
                    b"id=vpnhide_core\nid=other\n".to_vec(),
                );
            }
            "unsafe-path" => {
                files.insert("../escape".into(), vec![1]);
            }
            _ => unreachable!(),
        }
        files.insert(
            "payloads/manifest.json".into(),
            serde_json::to_vec(&m).unwrap(),
        );
        let entries = files
            .into_iter()
            .map(|(n, b)| (n, b, 0o100644, 0))
            .collect::<Vec<_>>();
        let p = b.path("changed.zip");
        fs::write(&p, raw_zip(&entries)).unwrap();
        assert!(descriptor(&p, COMMIT, 1001).is_err(), "{mutation}");
    }
}
