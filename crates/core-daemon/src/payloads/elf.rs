//! Bounded final-ELF capability inspection, independent of a producer's claim.
//! A marker alone does not prove early-init semantics; the acceptance gate is
//! separate. Never discover a capability by trying a parameter on an old ko.

use std::{collections::BTreeMap, io};

use super::{MAX_PAYLOAD_BYTES, invalid};

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
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
    let mut preflight = Vec::new();
    let mut init = Vec::new();
    let mut parameters = Vec::new();
    if let Some(metadata) = sections.get(".modinfo") {
        if metadata.kind != 1 || metadata.size > 64 * 1024 || metadata.content.last() != Some(&0) {
            return Err(invalid("invalid module metadata"));
        }
        for record in metadata
            .content
            .split(|b| *b == 0)
            .filter(|r| !r.is_empty())
        {
            let record = std::str::from_utf8(record).map_err(invalid)?;
            if !record.is_ascii() {
                return Err(invalid("non-ASCII module metadata"));
            }
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
        standard_parameters(&sections)?;
        result.init_contract = Some(1);
    }
    Ok(result)
}
pub fn preflight_protocol(data: &[u8]) -> io::Result<Option<u32>> {
    capabilities(data).map(|c| c.preflight_protocol)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

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
        let valid = include_bytes!("../../../../fixtures/payloads/init-contract.elf");
        assert_eq!(
            capabilities(valid).unwrap(),
            KmodCapabilities {
                preflight_protocol: Some(1),
                init_contract: Some(1)
            }
        );
        for bad in [
            include_bytes!("../../../../fixtures/payloads/invalid-module-alias.elf").as_slice(),
            include_bytes!("../../../../fixtures/payloads/invalid-swapped-setters.elf").as_slice(),
            include_bytes!("../../../../fixtures/payloads/invalid-scalar-overrun.elf").as_slice(),
            include_bytes!("../../../../fixtures/payloads/invalid-scalar-misaligned.elf")
                .as_slice(),
            include_bytes!("../../../../fixtures/payloads/invalid-other-scalar.elf").as_slice(),
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
