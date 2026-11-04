//! Bounded final-ELF capability inspection, independent of a producer's claim.
//! A marker alone does not prove early-init semantics; the acceptance gate is
//! separate. Never discover a capability by trying a parameter on an old ko.

use std::{collections::BTreeMap, io};

use super::{MAX_PAYLOAD_BYTES, invalid};

/// Bounded shared-library identity check used by projection preparation and its
/// recipient reader. This is not a dynamic-linker or installed-hook proof.
pub fn shared_object_header(
    data: &[u8],
    file_bytes: u64,
    abi: crate::manifest::Abi,
) -> io::Result<()> {
    let (class, machine, minimum) = match abi {
        crate::manifest::Abi::Arm64 => (2, 183u16, 64),
        crate::manifest::Abi::Arm32 => (1, 40u16, 52),
    };
    if data.len() < 20
        || file_bytes < minimum
        || file_bytes > MAX_PAYLOAD_BYTES
        || &data[..4] != b"\x7fELF"
        || data[4] != class
        || data[5] != 1
        || data[6] != 1
        || u16::from_le_bytes([data[16], data[17]]) != 3
        || u16::from_le_bytes([data[18], data[19]]) != machine
    {
        return Err(invalid(
            "incompatible shared library class/type/architecture",
        ));
    }
    Ok(())
}

fn bytes(data: &[u8], offset: u64, len: u64) -> io::Result<&[u8]> {
    let end = offset
        .checked_add(len)
        .ok_or_else(|| invalid("ELF range overflow"))?;
    let start = usize::try_from(offset).map_err(invalid)?;
    let end = usize::try_from(end).map_err(invalid)?;
    data.get(start..end)
        .ok_or_else(|| invalid("ELF range outside file"))
}
fn number(data: &[u8], offset: u64, len: u64) -> io::Result<u64> {
    let mut result = [0; 8];
    let value = bytes(data, offset, len)?;
    result[..value.len()].copy_from_slice(value);
    Ok(u64::from_le_bytes(result))
}
fn string(data: &[u8], offset: u64) -> io::Result<&str> {
    let data = data
        .get(usize::try_from(offset).map_err(invalid)?..)
        .ok_or_else(|| invalid("ELF string offset"))?;
    let data = &data[..data.len().min(257)];
    let end = data
        .iter()
        .position(|b| *b == 0)
        .ok_or_else(|| invalid("unterminated ELF string"))?;
    let value = std::str::from_utf8(&data[..end]).map_err(invalid)?;
    if !value.is_ascii() {
        return Err(invalid("non-ASCII ELF metadata"));
    }
    Ok(value)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct KmodCapabilities {
    pub preflight_protocol: Option<u32>,
    pub init_contract: Option<u32>,
}
struct Section<'a> {
    index: u64,
    header: &'a [u8],
    kind: u64,
    size: u64,
    content: &'a [u8],
}

/// Passive metadata of an AArch64 relocatable module, independent of any
/// VPN Hide capability marker. These values do not prove it is currently loaded.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleMetadata {
    pub name: String,
    pub vermagic: String,
    pub srcversion: Option<String>,
    pub build_id: Option<Vec<u8>>,
}

fn modinfo<'a>(sections: &BTreeMap<&str, Section<'a>>) -> io::Result<Vec<&'a str>> {
    let Some(metadata) = sections.get(".modinfo") else {
        return Ok(Vec::new());
    };
    if metadata.kind != 1 || metadata.size > 64 * 1024 || metadata.content.last() != Some(&0) {
        return Err(invalid("invalid module metadata"));
    }
    metadata
        .content
        .split(|b| *b == 0)
        .filter(|r| !r.is_empty())
        .map(|record| {
            let record = std::str::from_utf8(record).map_err(invalid)?;
            if !record.is_ascii() {
                return Err(invalid("non-ASCII module metadata"));
            }
            Ok(record)
        })
        .collect()
}

fn unique_metadata<'a>(records: &[&'a str], key: &str, max: usize) -> io::Result<Option<&'a str>> {
    let mut values = records.iter().filter_map(|record| record.strip_prefix(key));
    let value = values.next();
    if values.next().is_some()
        || value.is_some_and(|v| {
            v.is_empty() || v.len() > max || v.bytes().any(|b| b.is_ascii_control())
        })
    {
        return Err(invalid("duplicate or invalid module identity metadata"));
    }
    Ok(value)
}

/// Parse the literal GNU build-ID note, also usable on a bounded sysfs note read.
/// Reject duplicate/malformed notes instead of selecting the first identifier.
pub fn gnu_build_id(note: &[u8]) -> io::Result<Vec<u8>> {
    if note.len() > 4096 {
        return Err(invalid("module build-ID note too large"));
    }
    let mut offset = 0usize;
    let mut found = None;
    while offset < note.len() {
        let names = usize::try_from(number(note, offset as u64, 4)?).map_err(invalid)?;
        let desc = usize::try_from(number(note, offset as u64 + 4, 4)?).map_err(invalid)?;
        let kind = number(note, offset as u64 + 8, 4)?;
        let align = |n: usize| -> io::Result<usize> {
            n.checked_add(3)
                .map(|n| n & !3)
                .ok_or_else(|| invalid("note range overflow"))
        };
        let start = offset
            .checked_add(12)
            .ok_or_else(|| invalid("note range overflow"))?;
        let descriptor = start
            .checked_add(align(names)?)
            .ok_or_else(|| invalid("note range overflow"))?;
        let end = descriptor
            .checked_add(align(desc)?)
            .ok_or_else(|| invalid("note range overflow"))?;
        if end > note.len() {
            return Err(invalid("truncated module build-ID note"));
        }
        let name = bytes(note, start as u64, names as u64)?;
        let value = bytes(note, descriptor as u64, desc as u64)?;
        if name == b"GNU\0" && kind == 3 {
            if !(4..=64).contains(&desc) || found.is_some() {
                return Err(invalid("invalid or duplicate GNU build ID"));
            }
            found = Some(value.to_vec());
        }
        offset = end;
    }
    found.ok_or_else(|| invalid("missing GNU build ID"))
}

pub fn module_metadata(data: &[u8]) -> io::Result<ModuleMetadata> {
    let sections = sections(data)?;
    let records = modinfo(&sections)?;
    let name =
        unique_metadata(&records, "name=", 63)?.ok_or_else(|| invalid("missing module name"))?;
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    {
        return Err(invalid("invalid module name"));
    }
    let vermagic = unique_metadata(&records, "vermagic=", 1024)?
        .ok_or_else(|| invalid("missing module vermagic"))?;
    let srcversion = unique_metadata(&records, "srcversion=", 64)?;
    if srcversion.is_some_and(|v| !v.bytes().all(|b| b.is_ascii_hexdigit())) {
        return Err(invalid("invalid module source version"));
    }
    let build_id = sections
        .get(".note.gnu.build-id")
        .map(|note| {
            if note.kind != 7 {
                return Err(invalid("invalid module build-ID section"));
            }
            gnu_build_id(note.content)
        })
        .transpose()?;
    Ok(ModuleMetadata {
        name: name.into(),
        vermagic: vermagic.into(),
        srcversion: srcversion.map(str::to_owned),
        build_id,
    })
}

#[cfg(test)]
mod module_metadata_tests {
    use super::*;
    use crate::fixtures::{Section, relocatable};

    fn note() -> Vec<u8> {
        // Independent literal ELF note: namesz=4, descsz=5, NT_GNU_BUILD_ID=3.
        vec![
            4, 0, 0, 0, 5, 0, 0, 0, 3, 0, 0, 0, b'G', b'N', b'U', 0, 1, 2, 3, 4, 5, 0, 0, 0,
        ]
    }
    fn module(records: &[u8]) -> Vec<u8> {
        relocatable(records, &[Section::new(".note.gnu.build-id", note(), 7)])
    }

    #[test]
    fn reads_literal_standard_module_identity_without_vpnhide_markers() {
        let bytes = module(b"name=ksu_next\0vermagic=5.15.123-android13 SMP preempt mod_unload modversions aarch64\0srcversion=012ABC\0license=GPL\0");
        let metadata = module_metadata(&bytes).unwrap();
        assert_eq!(metadata.name, "ksu_next");
        assert_eq!(
            metadata.vermagic,
            "5.15.123-android13 SMP preempt mod_unload modversions aarch64"
        );
        assert_eq!(metadata.srcversion.as_deref(), Some("012ABC"));
        assert_eq!(metadata.build_id.as_deref(), Some(&[1, 2, 3, 4, 5][..]));
        assert_eq!(capabilities(&bytes).unwrap(), KmodCapabilities::default());
    }

    #[test]
    fn identities_reject_duplicates_controls_and_absent_fields() {
        for records in [
            &b"name=x\0vermagic=6.1.1\0name=x\0"[..],
            &b"name=x\0vermagic=6.1.1\0vermagic=6.1.1\0"[..],
            &b"name=x\0vermagic=6.1.1\0srcversion=A\0srcversion=A\0"[..],
            &b"name=x\0vermagic=6.1.1\0srcversion=not-hex\0"[..],
            &b"name=../x\0vermagic=6.1.1\0"[..],
            &b"name=x\0vermagic=6.1.1\n\0"[..],
            &b"name=\0vermagic=6.1.1\0"[..],
            &b"name=x\0"[..],
            &b"vermagic=6.1.1\0"[..],
            &b"name=x\0vermagic=6.1.1"[..],
        ] {
            assert!(module_metadata(&module(records)).is_err(), "{records:?}");
        }
        let bytes = module(b"name=x\0vermagic=6.1.1\0");
        for size in 0..bytes.len() {
            assert!(module_metadata(&bytes[..size]).is_err());
        }
        let missing_notes =
            module_metadata(&relocatable(b"name=x\0vermagic=6.1.1\0", &[])).unwrap();
        assert!(missing_notes.build_id.is_none());
        assert!(missing_notes.srcversion.is_none());
    }

    #[test]
    fn build_id_checks_complete_notes_padding_and_uniqueness() {
        let valid = note();
        assert_eq!(gnu_build_id(&valid).unwrap(), [1, 2, 3, 4, 5]);
        for size in 0..valid.len() {
            assert!(gnu_build_id(&valid[..size]).is_err());
        }
        let mut duplicate = valid.clone();
        duplicate.extend_from_slice(&valid);
        assert!(gnu_build_id(&duplicate).is_err());
        let mut wrong_name = valid.clone();
        wrong_name[12] = b'X';
        assert!(gnu_build_id(&wrong_name).is_err());
        let mut overflow = valid.clone();
        overflow[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(gnu_build_id(&overflow).is_err());
        let mut invalid_section = Section::new(".note.gnu.build-id", valid, 1);
        assert!(
            module_metadata(&relocatable(
                b"name=x\0vermagic=6.1.1\0",
                &[invalid_section.clone()]
            ))
            .is_err()
        );
        invalid_section.kind = 7;
        invalid_section.bytes.extend_from_slice(&[0]);
        assert!(
            module_metadata(&relocatable(
                b"name=x\0vermagic=6.1.1\0",
                &[invalid_section]
            ))
            .is_err()
        );
    }
}
fn sections(data: &[u8]) -> io::Result<BTreeMap<&str, Section<'_>>> {
    if data.len() < 64
        || data.len() as u64 > MAX_PAYLOAD_BYTES
        || data.get(..7) != Some(b"\x7fELF\x02\x01\x01")
        || number(data, 16, 2)? != 1
        || number(data, 18, 2)? != 183
        || number(data, 20, 4)? != 1
    {
        return Err(invalid("expected bounded AArch64 ELF64 relocatable"));
    }
    let offset = number(data, 40, 8)?;
    let count = number(data, 60, 2)?;
    let names_index = number(data, 62, 2)?;
    if number(data, 58, 2)? != 64
        || !(1..=4096).contains(&count)
        || names_index == 0
        || names_index >= count
    {
        return Err(invalid("unsupported ELF section table"));
    }
    let table = bytes(data, offset, count * 64)?;
    let names_header = bytes(table, names_index * 64, 64)?;
    let names_size = number(names_header, 32, 8)?;
    if number(names_header, 4, 4)? != 3 || names_size > 1024 * 1024 {
        return Err(invalid("invalid section names"));
    }
    let names = bytes(data, number(names_header, 24, 8)?, names_size)?;
    let mut result = BTreeMap::new();
    for index in 1..count {
        let header = bytes(table, index * 64, 64)?;
        let name = string(names, number(header, 0, 4)?)?;
        let kind = number(header, 4, 4)?;
        let size = number(header, 32, 8)?;
        let content = if kind == 8 {
            &[]
        } else {
            bytes(data, number(header, 24, 8)?, size)?
        };
        if result
            .insert(
                name,
                Section {
                    index,
                    header,
                    kind,
                    size,
                    content,
                },
            )
            .is_some()
        {
            return Err(invalid("duplicate ELF section"));
        }
    }
    Ok(result)
}
fn header_index<'a>(
    sections: &'a BTreeMap<&str, Section<'a>>,
    index: u64,
) -> io::Result<&'a Section<'a>> {
    sections
        .values()
        .find(|s| s.index == index)
        .ok_or_else(|| invalid("invalid linked section"))
}
/// Contract1 allows only the project's three standard scalar kernel parameter
/// setters. Own callbacks/constructors could execute before module_init and must
/// never inherit an early-rejection capability merely from a metadata marker.
fn standard_parameters(sections: &BTreeMap<&str, Section<'_>>) -> io::Result<()> {
    let params = sections
        .get("__param")
        .ok_or_else(|| invalid("missing kernel parameters"))?;
    let symbols = sections
        .get(".symtab")
        .ok_or_else(|| invalid("missing symbol table"))?;
    let relocations = sections
        .get(".rela__param")
        .ok_or_else(|| invalid("missing parameter relocations"))?;
    let strings = header_index(sections, number(symbols.header, 40, 4)?)?;
    if params.kind != 1
        || params.size != 120
        || symbols.kind != 2
        || number(symbols.header, 56, 8)? != 24
        || !symbols.size.is_multiple_of(24)
        || symbols.size / 24 > 262_144
        || strings.kind != 3
        || strings.size > 16 * 1024 * 1024
        || relocations.kind != 4
        || relocations.size != 12 * 24
        || number(relocations.header, 56, 8)? != 24
        || number(relocations.header, 40, 4)? != symbols.index
        || number(relocations.header, 44, 4)? != params.index
    {
        return Err(invalid("unsupported scalar parameter layout"));
    }
    for section in sections.values() {
        if matches!(section.kind, 4 | 9)
            && section.index != relocations.index
            && number(section.header, 44, 4)? == params.index
        {
            return Err(invalid("multiple parameter relocation sources"));
        }
    }
    let mut offsets = std::collections::BTreeSet::new();
    let mut names = std::collections::BTreeSet::new();
    let mut ops = Vec::new();
    let mut rows = std::collections::BTreeMap::new();
    for relocation in relocations.content.as_chunks::<24>().0 {
        let offset = number(relocation, 0, 8)?;
        let info = number(relocation, 8, 8)?;
        let addend = number(relocation, 16, 8)?;
        if offset >= params.size
            || !matches!(offset % 40, 0 | 8 | 16 | 32)
            || !offsets.insert(offset)
            || info as u32 != 257
            || bytes(params.content, offset, 8)?.iter().any(|&b| b != 0)
        {
            return Err(invalid("invalid parameter relocation"));
        }
        let symbol = bytes(
            symbols.content,
            (info >> 32)
                .checked_mul(24)
                .ok_or_else(|| invalid("symbol overflow"))?,
            24,
        )?;
        let name = string(strings.content, number(symbol, 0, 4)?)?;
        let section_index = number(symbol, 6, 2)?;
        let row = rows.entry(offset / 40).or_insert((None, None, None));
        match offset % 40 {
            16 => {
                if section_index != 0
                    || number(symbol, 4, 1)? >> 4 != 1
                    || addend != 0
                    || !matches!(name, "param_ops_bool" | "param_ops_uint")
                {
                    return Err(invalid("own or unrecognized parameter callback"));
                }
                ops.push(name);
                row.1 = Some(name);
            }
            8 => {
                if name != "__this_module"
                    || addend != 0
                    || number(symbol, 4, 1)? != 17
                    || number(symbol, 8, 8)? != 0
                    || sections
                        .get(".gnu.linkonce.this_module")
                        .is_none_or(|s| s.index != section_index)
                    || number(symbol, 16, 8)? == 0
                    || number(symbol, 16, 8)? > header_index(sections, section_index)?.size
                {
                    return Err(invalid("invalid parameter module owner"));
                }
            }
            _ => {
                let target = header_index(sections, section_index)?;
                let start = number(symbol, 8, 8)?
                    .checked_add(addend)
                    .ok_or_else(|| invalid("parameter address overflow"))?;
                if start >= target.size || number(target.header, 8, 8)? & 4 != 0 {
                    return Err(invalid("parameter points outside safe data"));
                }
                if offset % 40 == 0 {
                    let name = string(target.content, start)?;
                    if !matches!(name, "preflight_only" | "filesystem_hiding" | "probe_mask")
                        || !names.insert(name)
                    {
                        return Err(invalid("unexpected parameter name"));
                    }
                    row.0 = Some(name);
                } else {
                    if !matches!(target.kind, 1 | 8)
                        || number(target.header, 8, 8)? & 1 == 0
                        || ![sections.get(".data"), sections.get(".bss")]
                            .into_iter()
                            .flatten()
                            .any(|s| s.index == section_index)
                    {
                        return Err(invalid("parameter argument is not writable scalar storage"));
                    }
                    row.2 = Some((section_index, start));
                }
            }
        }
    }
    ops.sort();
    if ops != ["param_ops_bool", "param_ops_bool", "param_ops_uint"] || names.len() != 3 {
        return Err(invalid("incomplete scalar parameters"));
    }
    let mut scalars = Vec::new();
    for (name, ops, argument) in rows.into_values() {
        let name = name.ok_or_else(|| invalid("missing parameter name"))?;
        let size = if name == "probe_mask" { 4 } else { 1 };
        if ops
            != Some(if size == 4 {
                "param_ops_uint"
            } else {
                "param_ops_bool"
            })
        {
            return Err(invalid("mismatched scalar setter"));
        }
        let (section, start) = argument.ok_or_else(|| invalid("missing scalar argument"))?;
        let target = header_index(sections, section)?;
        if !start.is_multiple_of(size)
            || start.checked_add(size).is_none_or(|end| end > target.size)
        {
            return Err(invalid("scalar crosses storage boundary"));
        }
        let mut matches = Vec::new();
        for symbol in symbols.content.as_chunks::<24>().0 {
            if string(strings.content, number(symbol, 0, 4)?)? == name {
                matches.push((
                    number(symbol, 4, 1)?,
                    number(symbol, 6, 2)?,
                    number(symbol, 8, 8)?,
                    number(symbol, 16, 8)?,
                ));
            }
        }
        if matches != [(1, section, start, size)] {
            return Err(invalid(
                "parameter argument differs from its exact local scalar",
            ));
        }
        if scalars.iter().any(|&(ss, other, length)| {
            ss == section && start < other + length && other < start + size
        }) {
            return Err(invalid("overlapping scalar storage"));
        }
        scalars.push((section, start, size));
        for relocation in sections.values().filter(|s| matches!(s.kind, 4 | 9)) {
            if number(relocation.header, 44, 4)? != section {
                continue;
            }
            let stride = if relocation.kind == 4 { 24 } else { 16 };
            if number(relocation.header, 56, 8)? != stride
                || !relocation.size.is_multiple_of(stride)
            {
                return Err(invalid("invalid scalar storage relocation"));
            }
            for entry in relocation.content.chunks_exact(stride as usize) {
                let offset = number(entry, 0, 8)?;
                if offset < start + size && offset.saturating_add(16) > start {
                    return Err(invalid("relocation overlaps parameter scalar"));
                }
            }
        }
    }
    Ok(())
}
pub fn capabilities(data: &[u8]) -> io::Result<KmodCapabilities> {
    let sections = sections(data)?;
    capabilities_in(&sections)
}
fn capabilities_in(sections: &BTreeMap<&str, Section<'_>>) -> io::Result<KmodCapabilities> {
    let mut preflight = Vec::new();
    let mut init = Vec::new();
    let mut parameters = Vec::new();
    for record in modinfo(sections)? {
        if let Some(value) = record.strip_prefix("vpnhide_preflight=") {
            preflight.push(value);
        }
        if let Some(value) = record.strip_prefix("vpnhide_init_contract=") {
            init.push(value);
        }
        if record.starts_with("parmtype=") {
            parameters.push(record);
        }
    }
    let constructors = sections.iter().any(|(name, s)| {
        s.size != 0
            && (matches!(*name, ".ctors" | ".init_array" | ".preinit_array")
                || matches!(s.kind, 14 | 16))
    });
    let mut result = KmodCapabilities::default();
    if !preflight.is_empty() {
        if preflight != ["1"]
            || parameters
                .iter()
                .filter(|p| p.starts_with("parmtype=preflight_only:"))
                .copied()
                .collect::<Vec<_>>()
                != ["parmtype=preflight_only:bool"]
            || constructors
        {
            return Err(invalid("unverified preflight capability"));
        }
        result.preflight_protocol = Some(1);
    }
    if !init.is_empty() {
        parameters.sort();
        if init != ["1"]
            || constructors
            || parameters
                != [
                    "parmtype=filesystem_hiding:bool",
                    "parmtype=preflight_only:bool",
                    "parmtype=probe_mask:uint",
                ]
        {
            return Err(invalid("unverified closed init contract"));
        }
        standard_parameters(sections)?;
        result.init_contract = Some(1);
    }
    Ok(result)
}
pub fn preflight_protocol(data: &[u8]) -> io::Result<Option<u32>> {
    capabilities(data).map(|c| c.preflight_protocol)
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KmodInspection {
    pub sha256: String,
    pub bytes: u64,
    pub preflight_protocol: Option<u32>,
    pub init_contract: Option<u32>,
    pub constructors: bool,
    pub imports: std::collections::BTreeSet<String>,
    pub versions: BTreeMap<String, String>,
    pub extended_versions: bool,
}
fn content<'a>(sections: &'a BTreeMap<&str, Section<'a>>, name: &str) -> io::Result<&'a [u8]> {
    match sections.get(name) {
        None => Ok(&[]),
        Some(section) if section.kind != 8 => Ok(section.content),
        Some(_) => Err(invalid("metadata cannot use SHT_NOBITS")),
    }
}
/// Imports/CRC are research evidence, not proof of private kernel structure ABI.
/// Uses the same bounded ELF parser and capability checks as the device reader.
pub fn inspect(data: &[u8]) -> io::Result<KmodInspection> {
    use sha2::{Digest, Sha256};
    let sections = sections(data)?;
    let capability = capabilities_in(&sections)?;
    let mut imports = std::collections::BTreeSet::new();
    let symbols = content(&sections, ".symtab")?;
    if !symbols.is_empty() {
        let header = &sections[".symtab"];
        if header.kind != 2
            || number(header.header, 56, 8)? != 24
            || !symbols.len().is_multiple_of(24)
            || symbols.len() / 24 > 262_144
        {
            return Err(invalid("invalid symbol table"));
        }
        let strings = header_index(&sections, number(header.header, 40, 4)?)?;
        if strings.kind != 3 || strings.size > 16 * 1024 * 1024 {
            return Err(invalid("invalid symbol strings"));
        }
        for symbol in symbols.as_chunks::<24>().0 {
            let name = number(symbol, 0, 4)?;
            if number(symbol, 6, 2)? == 0
                && name != 0
                && matches!(number(symbol, 4, 1)? >> 4, 1 | 2)
            {
                imports.insert(string(strings.content, name)?.to_owned());
            }
        }
    }
    let mut versions = BTreeMap::new();
    let raw = content(&sections, "__versions")?;
    if !raw.len().is_multiple_of(64) {
        return Err(invalid("unsupported modversion record size"));
    }
    for row in raw.as_chunks::<64>().0 {
        let crc = number(row, 0, 8)?;
        let name = string(&row[8..], 0)?;
        if name.is_empty()
            || crc > u64::from(u32::MAX)
            || versions
                .insert(name.to_owned(), format!("{crc:08x}"))
                .is_some()
        {
            return Err(invalid("invalid or duplicate modversion"));
        }
    }
    Ok(KmodInspection {
        sha256: format!("{:x}", Sha256::digest(data)),
        bytes: data.len() as u64,
        preflight_protocol: capability.preflight_protocol,
        init_contract: capability.init_contract,
        constructors: sections.iter().any(|(name, s)| {
            s.size != 0
                && (matches!(*name, ".ctors" | ".init_array" | ".preinit_array")
                    || matches!(s.kind, 14 | 16))
        }),
        imports,
        versions,
        extended_versions: !content(&sections, "__version_ext_names")?.is_empty()
            || !content(&sections, "__version_ext_crcs")?.is_empty(),
    })
}
/// GKI distribution payloads must retain normal loader version checking. This
/// checks the exact ELF, not a producer label; values are still checked against
/// the running kernel by its loader. It does not prove private structure ABI.
/// Generic non-MODVERSIONS QEMU/experimental modules use `inspect` instead.
pub fn inspect_gki(data: &[u8]) -> io::Result<KmodInspection> {
    let inspected = inspect(data)?;
    if inspected.extended_versions {
        return Err(invalid("unsupported extended GKI modversion contract"));
    }
    if !inspected.versions.contains_key("module_layout")
        || inspected
            .imports
            .iter()
            .any(|name| !inspected.versions.contains_key(name))
    {
        return Err(invalid("GKI payload lacks complete loader symbol CRCs"));
    }
    Ok(inspected)
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct KmodComparison {
    pub left_sha256: String,
    pub right_sha256: String,
    pub common_imports: Vec<String>,
    pub crc_evidence_available: bool,
    pub common_version_records: usize,
    pub left_only_imports: Vec<String>,
    pub right_only_imports: Vec<String>,
    pub changed_crcs: BTreeMap<String, [String; 2]>,
    pub left_only_versions: Vec<String>,
    pub right_only_versions: Vec<String>,
    pub missing_import_crcs: BTreeMap<String, Vec<String>>,
    pub private_abi_proven: bool,
}
pub fn compare(left: &KmodInspection, right: &KmodInspection) -> KmodComparison {
    use std::collections::BTreeSet;
    let a: BTreeSet<_> = left.versions.keys().cloned().collect();
    let b: BTreeSet<_> = right.versions.keys().cloned().collect();
    let common: BTreeSet<_> = a.intersection(&b).cloned().collect();
    KmodComparison {
        left_sha256: left.sha256.clone(),
        right_sha256: right.sha256.clone(),
        common_imports: left.imports.intersection(&right.imports).cloned().collect(),
        crc_evidence_available: !left.versions.is_empty() && !right.versions.is_empty(),
        common_version_records: common.len(),
        left_only_imports: left.imports.difference(&right.imports).cloned().collect(),
        right_only_imports: right.imports.difference(&left.imports).cloned().collect(),
        changed_crcs: common
            .iter()
            .filter(|n| left.versions[*n] != right.versions[*n])
            .map(|n| {
                (
                    n.clone(),
                    [left.versions[n].clone(), right.versions[n].clone()],
                )
            })
            .collect(),
        left_only_versions: a.difference(&b).cloned().collect(),
        right_only_versions: b.difference(&a).cloned().collect(),
        missing_import_crcs: BTreeMap::from([
            (
                "left".into(),
                left.imports.difference(&a).cloned().collect(),
            ),
            (
                "right".into(),
                right.imports.difference(&b).cloned().collect(),
            ),
        ]),
        private_abi_proven: false,
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::fixtures::{Section as RawSection, init_contract_payload, relocatable};

    #[test]
    fn gki_requires_every_import_and_module_layout_crc_without_guessing_values() {
        use crate::fixtures::{gki_contract_payload, gki_contract_with_versions};
        let valid = gki_contract_payload(true);
        assert_eq!(
            inspect_gki(&valid).unwrap().versions["param_ops_uint"],
            "00000000"
        );
        let legacy = init_contract_payload(true);
        assert!(inspect(&legacy).is_ok());
        assert!(inspect_gki(&legacy).is_err());
        for records in [
            vec![],
            vec![("module_layout", 1)],
            vec![("param_ops_bool", 2), ("param_ops_uint", 3)],
            vec![("module_layout", 1), ("param_ops_bool", 2)],
            vec![
                ("module_layout", 1),
                ("param_ops_bool", 2),
                ("param_ops_uint", 3),
                ("module_layout", 4),
            ],
        ] {
            assert!(inspect_gki(&gki_contract_with_versions(true, &records)).is_err());
        }
    }

    #[test]
    fn legacy_and_arbitrary_text_are_not_capability_evidence() {
        for bytes in [
            relocatable(b"license=GPL\0", &[]),
            relocatable(
                b"license=GPL\0",
                &[RawSection::new(
                    ".text",
                    b"vpnhide_preflight=1\0".as_slice(),
                    1,
                )],
            ),
        ] {
            assert_eq!(inspect(&bytes).unwrap().preflight_protocol, None);
        }
    }
    #[test]
    fn host_inspection_preserves_strict_metadata_and_constructor_guards() {
        let marker = b"vpnhide_preflight=1\0parmtype=preflight_only:bool\0";
        assert_eq!(
            inspect(&relocatable(marker, &[]))
                .unwrap()
                .preflight_protocol,
            Some(1)
        );
        for bad in [
            b"vpnhide_preflight=1\0".as_slice(),
            b"vpnhide_preflight=2\0parmtype=preflight_only:bool\0",
            b"vpnhide_preflight=1\0parmtype=preflight_only:bool\0vpnhide_preflight=1\0",
            b"vpnhide_preflight=1\0parmtype=preflight_only:bool\0parmtype=preflight_only:uint\0",
        ] {
            assert!(inspect(&relocatable(bad, &[])).is_err());
        }
        assert!(
            inspect(&relocatable(
                marker,
                &[RawSection::new(".init_array", b"constructor".as_slice(), 1)]
            ))
            .is_err()
        );
    }
    #[test]
    fn malformed_section_extents_names_and_duplicate_metadata_refuse_inspection() {
        let valid = relocatable(b"license=GPL\0", &[]);
        for size in [0, 7, 63, valid.len() - 1] {
            assert!(inspect(&valid[..size]).is_err());
        }
        let mut overflow = valid.clone();
        overflow[40..48].copy_from_slice(&u64::MAX.to_le_bytes());
        for bad in [
            overflow,
            relocatable(b"unterminated", &[]),
            relocatable(
                b"license=GPL\0",
                &[RawSection::new(".modinfo", b"again\0".as_slice(), 1)],
            ),
            relocatable(
                b"license=GPL\0",
                &[RawSection::new(&"x".repeat(257), Vec::new(), 1)],
            ),
        ] {
            assert!(inspect(&bad).is_err());
        }
    }
    #[test]
    fn independently_encoded_contract_matches_preserved_golden_and_exact_scalar_failures() {
        let golden = include_bytes!("../../../fixtures/payloads/init-contract.elf");
        let valid = init_contract_payload(true);
        assert_eq!(valid.as_slice(), golden);
        assert_eq!(inspect(&valid).unwrap().init_contract, Some(1));
        let no_preflight = inspect(&init_contract_payload(false)).unwrap();
        assert_eq!(no_preflight.init_contract, Some(1));
        assert_eq!(no_preflight.preflight_protocol, None);
        let table = u64::from_le_bytes(valid[40..48].try_into().unwrap()) as usize;
        let relocation = u64::from_le_bytes(
            valid[table + 6 * 64 + 24..table + 6 * 64 + 32]
                .try_into()
                .unwrap(),
        ) as usize;
        type Mutations<'a> = (&'a [u8], &'a [(u64, u64, u64)]);
        for (expected, changes) in [
            (
                include_bytes!("../../../fixtures/payloads/invalid-module-alias.elf").as_slice(),
                &[(32, 3, 0)][..],
            ),
            (
                include_bytes!("../../../fixtures/payloads/invalid-swapped-setters.elf").as_slice(),
                &[(56, 2, 0), (96, 1, 0)][..],
            ),
            (
                include_bytes!("../../../fixtures/payloads/invalid-scalar-overrun.elf").as_slice(),
                &[(112, 5, 7)][..],
            ),
            (
                include_bytes!("../../../fixtures/payloads/invalid-scalar-misaligned.elf")
                    .as_slice(),
                &[(112, 5, 3)][..],
            ),
            (
                include_bytes!("../../../fixtures/payloads/invalid-other-scalar.elf").as_slice(),
                &[(32, 5, 1)][..],
            ),
        ] as [Mutations<'_>; 5]
        {
            let mut bad = valid.clone();
            for &(offset, symbol, addend) in changes {
                let kind = [0, 8, 16, 32]
                    .iter()
                    .position(|v| *v == offset % 40)
                    .unwrap();
                let at = relocation + (offset as usize / 40 * 4 + kind) * 24;
                bad[at..at + 8].copy_from_slice(&offset.to_le_bytes());
                bad[at + 8..at + 16].copy_from_slice(&(symbol << 32 | 257).to_le_bytes());
                bad[at + 16..at + 24].copy_from_slice(&addend.to_le_bytes());
            }
            assert_eq!(bad, expected);
            assert!(inspect(&bad).is_err());
        }
        let mut wrong = valid.clone();
        wrong[table + 6 * 64 + 44..table + 6 * 64 + 48].copy_from_slice(&2u32.to_le_bytes());
        assert!(inspect(&wrong).is_err());
        wrong = valid;
        wrong[table + 8 * 64 + 8..table + 8 * 64 + 16].copy_from_slice(&7u64.to_le_bytes());
        assert!(inspect(&wrong).is_err());
    }
    fn version(name: &str, crc: u64) -> Vec<u8> {
        let mut bytes = vec![0; 64];
        bytes[..8].copy_from_slice(&crc.to_le_bytes());
        bytes[8..8 + name.len()].copy_from_slice(name.as_bytes());
        bytes
    }
    #[test]
    fn crc_differences_missing_evidence_and_duplicate_records_remain_explicit() {
        let record = |n| {
            inspect(&relocatable(
                b"license=GPL\0",
                &[RawSection::new("__versions", version("symbol", n), 1)],
            ))
            .unwrap()
        };
        let report = compare(&record(1), &record(2));
        assert_eq!(
            report.changed_crcs,
            BTreeMap::from([("symbol".into(), ["00000001".into(), "00000002".into()])])
        );
        assert!(!report.private_abi_proven);
        let empty = inspect(&relocatable(b"license=GPL\0", &[])).unwrap();
        assert!(empty.versions.is_empty());
        assert!(!compare(&empty, &empty).crc_evidence_available);
        let duplicate = [version("symbol", 1), version("symbol", 2)].concat();
        assert!(
            inspect(&relocatable(
                b"license=GPL\0",
                &[RawSection::new("__versions", duplicate, 1)]
            ))
            .is_err()
        );
        assert!(
            inspect(&relocatable(
                b"license=GPL\0",
                &[RawSection::new(
                    "__versions",
                    version("symbol", u64::MAX),
                    1
                )]
            ))
            .is_err()
        );
    }
    #[test]
    fn undefined_imports_without_crcs_and_extended_records_do_not_prove_private_abi() {
        let mut symbols = vec![0; 48];
        symbols[24..28].copy_from_slice(&1u32.to_le_bytes());
        symbols[28] = 16;
        let mut symtab = RawSection::new(".symtab", symbols, 2);
        symtab.link = 4;
        symtab.entry_size = 24;
        let report = inspect(&relocatable(
            b"license=GPL\0",
            &[
                symtab,
                RawSection::new(".strtab", b"\0missing\0".as_slice(), 3),
                RawSection::new("__version_ext_names", b"missing\0".as_slice(), 1),
            ],
        ))
        .unwrap();
        assert!(report.imports.contains("missing"));
        assert!(report.extended_versions);
        let compared = compare(&report, &report);
        assert_eq!(compared.missing_import_crcs["left"], ["missing"]);
        assert!(!compared.private_abi_proven);
    }

    pub(crate) fn fixture(modinfo: &[u8], constructor: bool) -> Vec<u8> {
        let names = b"\0.shstrtab\0.modinfo\0.init_array\0";
        let mut data = vec![0; 64];
        data[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        data[16..18].copy_from_slice(&1u16.to_le_bytes());
        data[18..20].copy_from_slice(&183u16.to_le_bytes());
        data[20..24].copy_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(names);
        let metadata = data.len();
        data.extend_from_slice(modinfo);
        let table = data.len();
        data[40..48].copy_from_slice(&(table as u64).to_le_bytes());
        data[58..60].copy_from_slice(&64u16.to_le_bytes());
        let count = if constructor { 4u16 } else { 3u16 };
        data[60..62].copy_from_slice(&count.to_le_bytes());
        data[62..64].copy_from_slice(&1u16.to_le_bytes());
        data.resize(table + usize::from(count) * 64, 0);
        for (index, name, kind, at, size) in [
            (1, 1u32, 3u32, 64, names.len()),
            (2, 11, 1, metadata, modinfo.len()),
            (3, 20, 1, metadata, usize::from(constructor)),
        ] {
            if index >= usize::from(count) {
                continue;
            }
            let start = table + index * 64;
            data[start..start + 4].copy_from_slice(&name.to_le_bytes());
            data[start + 4..start + 8].copy_from_slice(&kind.to_le_bytes());
            data[start + 24..start + 32].copy_from_slice(&(at as u64).to_le_bytes());
            data[start + 32..start + 40].copy_from_slice(&(size as u64).to_le_bytes());
        }
        data
    }

    #[test]
    fn capabilities_require_exact_final_metadata_and_no_constructor() {
        let marker = b"vpnhide_preflight=1\0parmtype=preflight_only:bool\0";
        assert_eq!(
            preflight_protocol(&fixture(marker, false)).unwrap(),
            Some(1)
        );
        assert_eq!(
            preflight_protocol(&fixture(b"license=GPL\0", false)).unwrap(),
            None
        );
        for bad in [
            fixture(b"vpnhide_preflight=1\0", false),
            fixture(
                b"vpnhide_preflight=2\0parmtype=preflight_only:bool\0",
                false,
            ),
            fixture(
                b"vpnhide_preflight=1\0vpnhide_preflight=1\0parmtype=preflight_only:bool\0",
                false,
            ),
            fixture(marker, true),
            fixture(b"unterminated", false),
        ] {
            assert!(preflight_protocol(&bad).is_err());
        }
        let valid = fixture(marker, false);
        for size in 0..valid.len() {
            assert!(preflight_protocol(&valid[..size]).is_err());
        }
        let mut overflow = valid;
        overflow[40..48].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(preflight_protocol(&overflow).is_err());
        let mut wrong_kind = fixture(marker, false);
        let table = number(&wrong_kind, 40, 8).unwrap() as usize;
        wrong_kind[table + 2 * 64 + 4..table + 2 * 64 + 8].copy_from_slice(&3u32.to_le_bytes());
        assert!(preflight_protocol(&wrong_kind).is_err());
        assert!(string(&vec![b'x'; 1024], 0).is_err());
    }
    #[test]
    fn closed_init_contract_uses_shared_final_elf_and_rejects_owned_parameter_callbacks() {
        let valid = include_bytes!("../../../fixtures/payloads/init-contract.elf");
        assert_eq!(
            capabilities(valid).unwrap(),
            KmodCapabilities {
                preflight_protocol: Some(1),
                init_contract: Some(1)
            }
        );
        for bad in [
            include_bytes!("../../../fixtures/payloads/invalid-module-alias.elf").as_slice(),
            include_bytes!("../../../fixtures/payloads/invalid-swapped-setters.elf").as_slice(),
            include_bytes!("../../../fixtures/payloads/invalid-scalar-overrun.elf").as_slice(),
            include_bytes!("../../../fixtures/payloads/invalid-scalar-misaligned.elf").as_slice(),
            include_bytes!("../../../fixtures/payloads/invalid-other-scalar.elf").as_slice(),
        ] {
            assert!(capabilities(bad).is_err());
        }
        let mut owned_callback = valid.to_vec();
        let at = owned_callback
            .windows(14)
            .position(|b| b == b"param_ops_bool")
            .unwrap();
        owned_callback[at..at + 14].copy_from_slice(b"owned_ops_bool");
        assert!(capabilities(&owned_callback).is_err());
        assert!(capabilities(&fixture(b"vpnhide_init_contract=1\0", false)).is_err());
        for size in 0..valid.len() {
            assert!(capabilities(&valid[..size]).is_err());
        }
    }
}
