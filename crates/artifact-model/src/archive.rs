//! Bounded passive producer archives. No extraction of archive-selected paths,
//! runtime loading, publisher authentication or Android dependencies.
use crate::{elf, invalid, manifest::*};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::Path,
};
use zip::{ZipReadOptions, ZipWriter, unstable::read::ZipIterable, write::SimpleFileOptions};

pub const PROVENANCE: &str = "build-provenance.json";
pub use crate::package::FileIdentity;
impl FileIdentity {
    pub fn of(bytes: &[u8]) -> Self {
        Self {
            bytes: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(bytes)),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub schema: u32,
    pub source_commit: String,
    pub dirty: bool,
    pub component: String,
    pub version: String,
    pub files: BTreeMap<String, FileIdentity>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleProvenance {
    pub schema: u32,
    pub source_commit: String,
    pub inputs: BTreeMap<String, Provenance>,
}
pub fn safe_name(name: &str) -> bool {
    !name.contains(['\\', '\0'])
        && !name.chars().any(char::is_control)
        && !name.split('/').any(|s| matches!(s, "" | "." | ".."))
}
pub fn read_regular(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(invalid("not a bounded regular file"));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(invalid("file grew beyond budget"));
    }
    Ok(bytes)
}
pub fn read_archive(path: &Path, max_members: usize) -> io::Result<BTreeMap<String, Vec<u8>>> {
    // Pin the input and bound compressed bytes as well as expanded content.
    let input = read_regular(path, MAX_BUNDLE_BYTES)?;
    read_archive_bytes(&input, max_members)
}
pub fn read_archive_bytes(
    input: &[u8],
    max_members: usize,
) -> io::Result<BTreeMap<String, Vec<u8>>> {
    decode_archive(input, max_members, MAX_BUNDLE_BYTES)
}
fn decode_archive(
    input: &[u8],
    max_members: usize,
    expanded_limit: u64,
) -> io::Result<BTreeMap<String, Vec<u8>>> {
    if input.len() as u64 > MAX_BUNDLE_BYTES {
        return Err(invalid("compressed archive exceeds budget"));
    }
    // Read the footer and walk entries without building an index first. A
    // post-ZipArchive::new count check would occur after unbounded allocation.
    let mut archive = ZipIterable::try_new(io::Cursor::new(input), zip::read::Config::default())
        .map_err(invalid)?
        .enforce_header_consistency(true);
    if archive.len() > max_members {
        return Err(invalid("duplicate or excessive archive members"));
    }
    let entries = archive.files().map_err(invalid)?;
    let mut files = BTreeMap::new();
    let mut total = 0u64;
    while let Some(entry) = entries.next() {
        let entry = entry.map_err(invalid)?;
        let name = std::str::from_utf8(entry.name_raw())
            .map_err(invalid)?
            .to_owned();
        if !safe_name(&name)
            || files.contains_key(&name)
            || entry.is_dir()
            || entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 != 0 && mode & 0o170000 != 0o100000)
            || entry.encrypted()
            || entry.size() > MAX_PAYLOAD_BYTES
        {
            return Err(invalid("unsafe archive member"));
        }
        total = total
            .checked_add(entry.size())
            .ok_or_else(|| invalid("size overflow"))?;
        if total > expanded_limit {
            return Err(invalid("expanded archive exceeds budget"));
        }
        let expected = entry.size();
        let mut member = entry
            .with_reader(entries.reader_mut(), ZipReadOptions::default())
            .map_err(invalid)?;
        let mut bytes = Vec::new();
        Read::by_ref(&mut member)
            .take(expected + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 != expected {
            return Err(invalid("archive member size mismatch"));
        }
        files.insert(name, bytes);
    }
    Ok(files)
}
pub fn metadata<T: for<'a> Deserialize<'a>>(
    files: &BTreeMap<String, Vec<u8>>,
    name: &str,
) -> io::Result<T> {
    let bytes = files
        .get(name)
        .ok_or_else(|| invalid(format!("missing {name}")))?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(invalid("metadata exceeds budget"));
    }
    serde_json::from_slice(bytes).map_err(invalid)
}
fn exact_commit(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
impl Provenance {
    fn validate(
        &self,
        component: &str,
        commit: &str,
        files: &BTreeMap<String, Vec<u8>>,
    ) -> io::Result<()> {
        if self.schema != 1
            || self.dirty
            || self.source_commit != commit
            || !exact_commit(commit)
            || self.component != component
            || self.version.is_empty()
            || self.version.len() > 128
            || self.version.chars().any(char::is_control)
            || self.files.len() + 1 != files.len()
        {
            return Err(invalid("artifact provenance mismatch or dirty build"));
        }
        for (name, bytes) in files {
            if name != PROVENANCE && self.files.get(name) != Some(&FileIdentity::of(bytes)) {
                return Err(invalid("artifact provenance does not cover exact bytes"));
            }
        }
        Ok(())
    }
}
pub struct Artifact {
    pub provenance: Provenance,
    files: BTreeMap<String, Vec<u8>>,
}
impl Artifact {
    pub fn open(path: &Path, component: &str, commit: &str) -> io::Result<Self> {
        Self::open_with_budget(path, component, commit, MAX_BUNDLE_BYTES)
    }
    fn open_with_budget(
        path: &Path,
        component: &str,
        commit: &str,
        remaining: u64,
    ) -> io::Result<Self> {
        let bytes = read_regular(path, MAX_BUNDLE_BYTES)?;
        let files = decode_archive(&bytes, 64, remaining)?;
        let provenance: Provenance = metadata(&files, PROVENANCE)?;
        provenance.validate(component, commit, &files)?;
        Ok(Self { provenance, files })
    }
    fn member(&self, name: &str, machine: u16) -> io::Result<Vec<u8>> {
        let bytes = self
            .files
            .get(name)
            .ok_or_else(|| invalid(format!("missing {name}")))?;
        check_architecture(bytes, machine)?;
        Ok(bytes.clone())
    }
}
pub fn check_architecture(bytes: &[u8], machine: u16) -> io::Result<()> {
    let class = if machine == 183 { 2 } else { 1 };
    if bytes.len() < 20
        || bytes[..6] != [0x7f, b'E', b'L', b'F', class, 1]
        || bytes[18..20] != machine.to_le_bytes()
    {
        return Err(invalid("wrong ELF architecture"));
    }
    Ok(())
}
pub fn write_archive(output: &Path, files: &BTreeMap<String, Vec<u8>>) -> io::Result<()> {
    let total: u64 = files.values().map(|b| b.len() as u64).sum();
    if total > MAX_BUNDLE_BYTES || files.len() > 128 {
        return Err(invalid("assembled archive exceeds budget"));
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let parent_directory = File::open(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    {
        let mut writer = ZipWriter::new(temporary.as_file_mut());
        for (name, bytes) in files {
            if !safe_name(name) || bytes.len() as u64 > MAX_PAYLOAD_BYTES {
                return Err(invalid("unsafe output member"));
            }
            let executable = name.starts_with("bin/")
                || name.ends_with(".sh")
                || name.ends_with("update-binary");
            writer
                .start_file(
                    name,
                    SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Deflated)
                        .unix_permissions(if executable { 0o755 } else { 0o644 }),
                )
                .map_err(invalid)?;
            writer.write_all(bytes)?;
        }
        writer.finish().map_err(invalid)?;
    }
    if temporary.as_file().metadata()?.len() > MAX_BUNDLE_BYTES {
        return Err(invalid("compressed output exceeds budget"));
    }
    temporary.as_file().sync_all()?;
    temporary.persist(output).map_err(|e| e.error)?;
    parent_directory.sync_all()?;
    Ok(())
}
pub fn assemble(inputs: &Path, output: &Path, commit: &str) -> io::Result<()> {
    assemble_with_budget(inputs, output, commit, MAX_BUNDLE_BYTES)
}
fn assemble_with_budget(
    inputs: &Path,
    output: &Path,
    commit: &str,
    retained_limit: u64,
) -> io::Result<()> {
    if !exact_commit(commit) {
        return Err(invalid("expected exact source commit"));
    }
    let mut specs = vec![("core".to_string(), "vpnhide-core-base.zip".to_string())];
    specs.extend(
        KMIS.iter()
            .map(|kmi| (format!("kmod:{kmi}"), format!("vpnhide-kmod-{kmi}.zip"))),
    );
    specs.extend([
        ("kpm".into(), "vpnhide-kpm.zip".into()),
        ("zygisk".into(), "vpnhide-zygisk.zip".into()),
    ]);
    let mut artifacts = BTreeMap::new();
    let mut retained = 0u64;
    for (component, name) in specs {
        let artifact = Artifact::open_with_budget(
            &inputs.join(name),
            &component,
            commit,
            retained_limit - retained,
        )?;
        retained += artifact.files.values().map(|b| b.len() as u64).sum::<u64>();
        artifacts.insert(component, artifact);
    }
    let core = &artifacts["core"];
    core.member("bin/arm64/vpnhide-core", 183)?;
    core.member("bin/arm/vpnhide-core", 40)?;
    let mut files = BTreeMap::new();
    for (name, bytes) in &core.files {
        if name.starts_with("zygisk/") || name.starts_with("payloads/") {
            return Err(invalid("base Core contains payloads"));
        }
        if name != PROVENANCE {
            files.insert(name.clone(), bytes.clone());
        }
    }
    let properties = std::str::from_utf8(
        files
            .get("module.prop")
            .ok_or_else(|| invalid("missing module identity"))?,
    )
    .map_err(invalid)?;
    if !properties.lines().any(|line| line == "id=vpnhide_core")
        || properties.lines().any(|line| {
            line.trim().starts_with("vpnhideControl=")
                || line.trim().starts_with("vpnhideActivation=")
        })
    {
        return Err(invalid(
            "unexpected Core identity or ambiguous activation metadata",
        ));
    }
    files.insert(
        "module.prop".into(),
        format!(
            "{}\nvpnhideControl=2\nvpnhideActivation=1\n",
            properties.trim_end_matches('\n')
        )
        .into_bytes(),
    );
    let mut identities: Vec<PayloadKind> = KMIS
        .iter()
        .map(|kmi| PayloadKind::Kmod { kmi: (*kmi).into() })
        .collect();
    identities.extend([
        PayloadKind::Kpm,
        PayloadKind::Zygisk { abi: Abi::Arm64 },
        PayloadKind::Zygisk { abi: Abi::Arm32 },
    ]);
    let mut manifest = Manifest {
        schema: MANIFEST_VERSION,
        bundle_commit: commit.into(),
        payloads: Vec::new(),
    };
    for implementation in identities {
        let (component, source, machine) = match &implementation {
            PayloadKind::Kmod { kmi } => (format!("kmod:{kmi}"), "vpnhide_kmod.ko".into(), 183),
            PayloadKind::Kpm => ("kpm".into(), "vpnhide.kpm".into(), 183),
            PayloadKind::Zygisk { abi } => (
                "zygisk".into(),
                format!(
                    "zygisk/{}.so",
                    if *abi == Abi::Arm64 {
                        "arm64-v8a"
                    } else {
                        "armeabi-v7a"
                    }
                ),
                if *abi == Abi::Arm64 { 183 } else { 40 },
            ),
        };
        let artifact = &artifacts[&component];
        let bytes = artifact.member(&source, machine)?;
        let (preflight_protocol, init_contract) =
            if matches!(implementation, PayloadKind::Kmod { .. }) {
                let caps = elf::capabilities(&bytes)?;
                if caps.preflight_protocol != Some(1) || caps.init_contract != Some(1) {
                    return Err(invalid(
                        "kmod lacks verified preflight and closed init contract",
                    ));
                }
                (caps.preflight_protocol, caps.init_contract)
            } else {
                (None, None)
            };
        let proof = FileIdentity::of(&bytes);
        if matches!(implementation, PayloadKind::Zygisk { .. }) {
            files.insert(source, bytes.clone());
        }
        files.insert(implementation.relative_path()?, bytes);
        manifest.payloads.push(Payload {
            implementation,
            version: artifact.provenance.version.clone(),
            source_commit: commit.into(),
            sha256: proof.sha256,
            bytes: proof.bytes,
            control_version: 2,
            preflight_protocol,
            init_contract,
        });
    }
    manifest.require_complete()?;
    files.insert(
        "payloads/manifest.json".into(),
        serde_json::to_vec(&manifest).map_err(invalid)?,
    );
    files.insert(
        "bundle-provenance.json".into(),
        serde_json::to_vec(&BundleProvenance {
            schema: 1,
            source_commit: commit.into(),
            inputs: artifacts
                .into_iter()
                .map(|(c, a)| (c, a.provenance))
                .collect(),
        })
        .map_err(invalid)?,
    );
    write_archive(output, &files)
}

/// Publisher-side consistency check. The caller must still sign these exact
/// bytes using its existing identity; runtime authentication remains mandatory.
pub fn descriptor(
    path: &Path,
    commit: &str,
    sequence: u64,
) -> io::Result<crate::package::Descriptor> {
    use crate::package::{Descriptor, RuntimeIdentity};
    if !exact_commit(commit) || sequence == 0 {
        return Err(invalid("invalid source or build sequence"));
    }
    let package = read_regular(path, MAX_BUNDLE_BYTES)?;
    let files = read_archive_bytes(&package, 128)?;
    let manifest: Manifest = metadata(&files, "payloads/manifest.json")?;
    manifest.require_complete()?;
    let provenance: BundleProvenance = metadata(&files, "bundle-provenance.json")?;
    let expected: std::collections::BTreeSet<_> =
        ["core".to_string(), "kpm".into(), "zygisk".into()]
            .into_iter()
            .chain(KMIS.iter().map(|kmi| format!("kmod:{kmi}")))
            .collect();
    if provenance.schema != 1
        || provenance.source_commit != commit
        || manifest.bundle_commit != commit
        || provenance
            .inputs
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            != expected
    {
        return Err(invalid("not a complete same-commit unified Core bundle"));
    }
    for (component, p) in &provenance.inputs {
        if p.schema != 1
            || p.dirty
            || p.source_commit != commit
            || &p.component != component
            || p.version.is_empty()
            || p.version.len() > 128
            || p.version.chars().any(char::is_control)
        {
            return Err(invalid("dirty or mixed package input"));
        }
    }
    for payload in &manifest.payloads {
        let name = payload.implementation.relative_path()?;
        let bytes = files
            .get(&name)
            .ok_or_else(|| invalid("missing payload member"))?;
        let identity = FileIdentity::of(bytes);
        if payload.source_commit != commit
            || identity.bytes != payload.bytes
            || identity.sha256 != payload.sha256
        {
            return Err(invalid("payload provenance or bytes differ"));
        }
        if matches!(payload.implementation, PayloadKind::Kmod { .. }) {
            let caps = elf::capabilities(bytes)?;
            if payload.preflight_protocol != Some(1)
                || payload.init_contract != Some(1)
                || caps.preflight_protocol != payload.preflight_protocol
                || caps.init_contract != payload.init_contract
            {
                return Err(invalid("payload capability differs from final ELF"));
            }
        }
        if let PayloadKind::Zygisk { abi } = payload.implementation {
            let source = format!(
                "zygisk/{}.so",
                if abi == Abi::Arm64 {
                    "arm64-v8a"
                } else {
                    "armeabi-v7a"
                }
            );
            if files.get(&source) != Some(bytes) {
                return Err(invalid("autoload mirror differs from verified payload"));
            }
        }
    }
    check_architecture(
        files
            .get("bin/arm64/vpnhide-core")
            .ok_or_else(|| invalid("missing arm64 daemon"))?,
        183,
    )?;
    check_architecture(
        files
            .get("bin/arm/vpnhide-core")
            .ok_or_else(|| invalid("missing arm daemon"))?,
        40,
    )?;
    let properties = std::str::from_utf8(
        files
            .get("module.prop")
            .ok_or_else(|| invalid("missing module identity"))?,
    )
    .map_err(invalid)?;
    if properties
        .lines()
        .filter(|l| l.starts_with("id="))
        .collect::<Vec<_>>()
        != ["id=vpnhide_core"]
    {
        return Err(invalid("wrong or ambiguous module identity"));
    }
    let identity = |name: &str, limit: u64| -> io::Result<FileIdentity> {
        let bytes = files
            .get(name)
            .ok_or_else(|| invalid(format!("missing {name}")))?;
        if bytes.is_empty() || bytes.len() as u64 > limit {
            return Err(invalid("runtime member exceeds limit"));
        }
        Ok(FileIdentity::of(bytes))
    };
    let result = Descriptor {
        schema: 2,
        module_id: "vpnhide_core".into(),
        source_commit: commit.into(),
        build_sequence: sequence,
        package_sha256: format!("{:x}", Sha256::digest(&package)),
        package_bytes: package.len() as u64,
        rpc: vpnhide_build_contract::RPC_VERSION,
        journal_min: vpnhide_build_contract::JOURNAL_MIN_READ,
        journal_max: vpnhide_build_contract::JOURNAL_VERSION,
        kernel_control: vpnhide_protocol::CONTROL_VERSION,
        runtime: RuntimeIdentity {
            daemon_arm64: identity("bin/arm64/vpnhide-core", MAX_PAYLOAD_BYTES)?,
            daemon_arm: identity("bin/arm/vpnhide-core", MAX_PAYLOAD_BYTES)?,
            payload_manifest: identity("payloads/manifest.json", MAX_MANIFEST_BYTES)?,
            service: identity("service.sh", MAX_MANIFEST_BYTES)?,
            sepolicy: identity("sepolicy.rule", MAX_MANIFEST_BYTES)?,
            module_prop: identity("module.prop", MAX_MANIFEST_BYTES)?,
        },
    };
    result.validate().map_err(invalid)?;
    Ok(result)
}

pub fn stamp(
    staging: &Path,
    component: &str,
    version: &str,
    commit: &str,
    dirty: bool,
) -> io::Result<()> {
    if !exact_commit(commit)
        || component.is_empty()
        || component.len() > 128
        || component.chars().any(char::is_control)
        || version.is_empty()
        || version.len() > 128
        || version.chars().any(char::is_control)
        || !fs::symlink_metadata(staging)?.is_dir()
    {
        return Err(invalid("invalid producer identity or staging directory"));
    }
    fn collect(
        root: &Path,
        dir: &Path,
        files: &mut BTreeMap<String, FileIdentity>,
        total: &mut u64,
        depth: usize,
    ) -> io::Result<()> {
        if depth > 16 {
            return Err(invalid("staging depth exceeds budget"));
        }
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let kind = entry.file_type()?;
            if kind.is_dir() {
                collect(root, &path, files, total, depth + 1)?;
            } else if kind.is_file() {
                let name = path
                    .strip_prefix(root)
                    .map_err(invalid)?
                    .to_str()
                    .ok_or_else(|| invalid("non-UTF8 path"))?
                    .to_string();
                if name == PROVENANCE {
                    continue;
                }
                if !safe_name(&name) || files.len() >= 63 {
                    return Err(invalid("unsafe or excessive staging files"));
                }
                let bytes = read_regular(&path, MAX_PAYLOAD_BYTES)?;
                *total += bytes.len() as u64;
                if *total > MAX_BUNDLE_BYTES {
                    return Err(invalid("staging exceeds budget"));
                }
                files.insert(name, FileIdentity::of(&bytes));
            } else {
                return Err(invalid("non-regular staging member"));
            }
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    collect(staging, staging, &mut files, &mut 0, 0)?;
    let bytes = serde_json::to_vec(&Provenance {
        schema: 1,
        source_commit: commit.into(),
        dirty,
        component: component.into(),
        version: version.into(),
        files,
    })
    .map_err(invalid)?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(invalid("producer metadata exceeds budget"));
    }
    let mut temporary = tempfile::NamedTempFile::new_in(staging)?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(staging.join(PROVENANCE))
        .map_err(|e| e.error)?;
    Ok(())
}

#[cfg(test)]
mod tests;
