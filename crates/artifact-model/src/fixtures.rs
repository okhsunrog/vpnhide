//! Independent synthetic inputs, not production payload builders. Golden bytes
//! originate from the former separately implemented Python test producer.
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Component, Path},
};

pub fn bundle_files() -> io::Result<BTreeMap<String, Vec<u8>>> {
    #[derive(serde::Deserialize)]
    struct Golden {
        origin: String,
        files: BTreeMap<String, String>,
    }
    let golden: Golden = serde_json::from_str(include_str!(
        "../../../fixtures/payloads/bundle-golden.json"
    ))
    .map_err(io::Error::other)?;
    assert!(!golden.origin.is_empty());
    let mut files = BTreeMap::new();
    for (name, hex) in golden.files {
        if Path::new(&name)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
            || name.contains('\\')
            || !hex.len().is_multiple_of(2)
        {
            return Err(io::Error::other("invalid fixture"));
        }
        let bytes: Result<Vec<u8>, _> = hex
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|v| {
                u8::from_str_radix(std::str::from_utf8(v).map_err(io::Error::other)?, 16)
                    .map_err(io::Error::other)
            })
            .collect();
        files.insert(name, bytes?);
    }
    Ok(files)
}
/// Explicit old-layout fixture retained for migration/revocation regressions.
pub fn write_bundle_tree(root: &Path) -> io::Result<()> {
    write_files(root, bundle_files()?)
}
pub fn passive_bundle_files() -> io::Result<BTreeMap<String, Vec<u8>>> {
    let mut files = bundle_files()?;
    files.retain(|name, _| !name.starts_with("zygisk/"));
    files.insert(
        "module.prop".into(),
        b"id=vpnhide_core\nvpnhideControl=2\n".to_vec(),
    );
    // Keep the historical golden unchanged. New replacement fixtures carry an
    // independently encoded versioned GKI contract instead of its old payloads.
    let mut manifest: serde_json::Value = serde_json::from_slice(&files["payloads/manifest.json"])?;
    let mut provenance: serde_json::Value =
        serde_json::from_slice(&files["bundle-provenance.json"])?;
    for payload in manifest["payloads"].as_array_mut().unwrap() {
        if payload["implementation"]["kind"] != "kmod" {
            continue;
        }
        let kmi = payload["implementation"]["kmi"]
            .as_str()
            .unwrap()
            .to_owned();
        let bytes = gki_contract_payload(true);
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        payload["bytes"] = bytes.len().into();
        payload["sha256"] = sha256.clone().into();
        provenance["inputs"][format!("kmod:{kmi}")]["files"]["vpnhide_kmod.ko"] =
            serde_json::json!({"bytes":bytes.len(),"sha256":sha256});
        files.insert(format!("payloads/kmod/{kmi}/vpnhide_kmod.ko"), bytes);
    }
    files.insert(
        "payloads/manifest.json".into(),
        serde_json::to_vec(&manifest)?,
    );
    files.insert(
        "bundle-provenance.json".into(),
        serde_json::to_vec(&provenance)?,
    );
    Ok(files)
}
pub fn write_passive_bundle_tree(root: &Path) -> io::Result<()> {
    write_files(root, passive_bundle_files()?)
}
fn write_files(root: &Path, files: BTreeMap<String, Vec<u8>>) -> io::Result<()> {
    for (name, bytes) in files {
        let path = root.join(name);
        fs::create_dir_all(
            path.parent()
                .ok_or_else(|| io::Error::other("missing fixture parent"))?,
        )?;
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?
            .write_all(&bytes)?;
    }
    Ok(())
}

/// A raw ELF encoder deliberately independent of the parser's section types.
#[derive(Clone)]
pub struct Section {
    pub name: String,
    pub bytes: Vec<u8>,
    pub kind: u32,
    pub flags: u64,
    pub link: u32,
    pub info: u32,
    pub entry_size: u64,
}
impl Section {
    pub fn new(name: &str, bytes: impl Into<Vec<u8>>, kind: u32) -> Self {
        Self {
            name: name.into(),
            bytes: bytes.into(),
            kind,
            flags: 0,
            link: 0,
            info: 0,
            entry_size: 0,
        }
    }
}
fn u32_at(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn u64_at(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
pub fn relocatable(modinfo: &[u8], extra: &[Section]) -> Vec<u8> {
    let mut sections = vec![Section::new(".modinfo", modinfo, 1)];
    sections.extend_from_slice(extra);
    let mut names = b"\0.shstrtab\0".to_vec();
    let mut offsets = Vec::new();
    for section in &sections {
        offsets.push(names.len() as u32);
        names.extend_from_slice(section.name.as_bytes());
        names.push(0);
    }
    let mut bytes = vec![0; 64];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    bytes[16..18].copy_from_slice(&1u16.to_le_bytes());
    bytes[18..20].copy_from_slice(&183u16.to_le_bytes());
    u32_at(&mut bytes, 20, 1);
    let mut headers = vec![[0_u8; 64]; 2];
    u32_at(&mut headers[1], 0, 1);
    u32_at(&mut headers[1], 4, 3);
    u64_at(&mut headers[1], 24, 64);
    u64_at(&mut headers[1], 32, names.len() as u64);
    u64_at(&mut headers[1], 48, 1);
    bytes.extend_from_slice(&names);
    for (section, offset) in sections.iter().zip(offsets) {
        let mut header = [0; 64];
        u32_at(&mut header, 0, offset);
        u32_at(&mut header, 4, section.kind);
        u64_at(&mut header, 8, section.flags);
        u64_at(&mut header, 24, bytes.len() as u64);
        u64_at(&mut header, 32, section.bytes.len() as u64);
        u32_at(&mut header, 40, section.link);
        u32_at(&mut header, 44, section.info);
        u64_at(&mut header, 48, 1);
        u64_at(&mut header, 56, section.entry_size);
        headers.push(header);
        bytes.extend_from_slice(&section.bytes);
    }
    let table = bytes.len() as u64;
    u64_at(&mut bytes, 40, table);
    bytes[58..60].copy_from_slice(&64u16.to_le_bytes());
    bytes[60..62].copy_from_slice(&(headers.len() as u16).to_le_bytes());
    bytes[62..64].copy_from_slice(&1u16.to_le_bytes());
    for header in headers {
        bytes.extend_from_slice(&header);
    }
    bytes
}
pub fn init_contract_payload(preflight: bool) -> Vec<u8> {
    init_contract_with_versions(preflight, None)
}

pub fn gki_contract_payload(preflight: bool) -> Vec<u8> {
    gki_contract_with_versions(
        preflight,
        &[
            ("module_layout", 0x12345678),
            ("param_ops_bool", 0x87654321),
            ("param_ops_uint", 0x00000000),
        ],
    )
}

pub fn gki_contract_with_versions(preflight: bool, records: &[(&str, u32)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (name, crc) in records {
        assert!(name.len() < 56);
        let mut row = [0; 64];
        row[..8].copy_from_slice(&u64::from(*crc).to_le_bytes());
        row[8..8 + name.len()].copy_from_slice(name.as_bytes());
        bytes.extend_from_slice(&row);
    }
    init_contract_with_versions(preflight, Some(bytes))
}

fn init_contract_with_versions(preflight: bool, versions: Option<Vec<u8>>) -> Vec<u8> {
    let mut metadata=b"vpnhide_init_contract=1\0parmtype=preflight_only:bool\0parmtype=filesystem_hiding:bool\0parmtype=probe_mask:uint\0".to_vec();
    if preflight {
        metadata.extend_from_slice(b"vpnhide_preflight=1\0");
    }
    let strings=b"\0param_ops_bool\0param_ops_uint\0__this_module\0preflight_only\0filesystem_hiding\0probe_mask\0";
    let offset = |name: &[u8]| {
        strings
            .windows(name.len() + 1)
            .position(|v| v[..name.len()] == *name && v[name.len()] == 0)
            .unwrap() as u32
    };
    let mut symbols = vec![0_u8; 24];
    for (name, binding, section) in [
        ("param_ops_bool", 16, 0),
        ("param_ops_uint", 16, 0),
        ("__this_module", 17, 9),
        ("", 3, 7),
        ("", 3, 8),
    ] {
        let mut symbol = [0; 24];
        u32_at(
            &mut symbol,
            0,
            if name.is_empty() {
                0
            } else {
                offset(name.as_bytes())
            },
        );
        symbol[4] = binding;
        symbol[6..8].copy_from_slice(&(section as u16).to_le_bytes());
        u64_at(
            &mut symbol,
            16,
            if name == "__this_module" { 64 } else { 0 },
        );
        symbols.extend_from_slice(&symbol);
    }
    for (name, value, size) in [
        ("preflight_only", 0, 1),
        ("filesystem_hiding", 1, 1),
        ("probe_mask", 4, 4),
    ] {
        let mut symbol = [0; 24];
        u32_at(&mut symbol, 0, offset(name.as_bytes()));
        symbol[4] = 1;
        symbol[6..8].copy_from_slice(&8u16.to_le_bytes());
        u64_at(&mut symbol, 8, value);
        u64_at(&mut symbol, 16, size);
        symbols.extend_from_slice(&symbol);
    }
    let names = b"preflight_only\0filesystem_hiding\0probe_mask\0";
    let mut relocations = Vec::new();
    for (index, name) in ["preflight_only", "filesystem_hiding", "probe_mask"]
        .into_iter()
        .enumerate()
    {
        let position = names
            .windows(name.len() + 1)
            .position(|v| v[..name.len()] == *name.as_bytes() && v[name.len()] == 0)
            .unwrap();
        for (field, symbol, addend) in [
            (0, 4, position as u64),
            (8, 3, 0),
            (16, if index < 2 { 1 } else { 2 }, 0),
            (32, 5, [0, 1, 4][index]),
        ] {
            relocations.extend_from_slice(&(index as u64 * 40 + field).to_le_bytes());
            relocations.extend_from_slice(&((symbol << 32) | 257_u64).to_le_bytes());
            relocations.extend_from_slice(&addend.to_le_bytes());
        }
    }
    let mut extra = vec![
        Section::new("__param", vec![0; 120], 1),
        Section::new(".symtab", symbols, 2),
        Section::new(".strtab", strings.as_slice(), 3),
        Section::new(".rela__param", relocations, 4),
        Section::new(".rodata", names.as_slice(), 1),
        Section::new(".data", vec![0; 8], 1),
        Section::new(".gnu.linkonce.this_module", vec![0; 64], 1),
    ];
    extra[1].link = 5;
    extra[1].entry_size = 24;
    extra[3].link = 4;
    extra[3].info = 3;
    extra[3].entry_size = 24;
    extra[4].flags = 2;
    extra[5].flags = 3;
    extra[6].flags = 3;
    if let Some(versions) = versions {
        extra.push(Section::new("__versions", versions, 1));
    }
    relocatable(&metadata, &extra)
}

use crate::manifest::*;
fn payload(implementation: PayloadKind) -> Payload {
    Payload {
        implementation,
        version: "test".into(),
        source_commit: "a".repeat(40),
        sha256: format!("{:x}", Sha256::digest(b"payload")),
        bytes: 7,
        control_version: 2,
        preflight_protocol: None,
        init_contract: None,
    }
}
pub fn manifest() -> Manifest {
    let mut payloads: Vec<_> = KMIS
        .iter()
        .map(|kmi| payload(PayloadKind::Kmod { kmi: (*kmi).into() }))
        .collect();
    payloads.extend([
        payload(PayloadKind::Kpm),
        payload(PayloadKind::Zygisk { abi: Abi::Arm64 }),
        payload(PayloadKind::Zygisk { abi: Abi::Arm32 }),
    ]);
    Manifest {
        schema: 1,
        bundle_commit: "b".repeat(40),
        payloads,
    }
}
