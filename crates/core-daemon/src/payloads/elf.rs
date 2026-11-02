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

pub fn preflight_protocol(data: &[u8]) -> io::Result<Option<u32>> {
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
    let entry_size = number(data, 58, 2)?;
    let count = number(data, 60, 2)?;
    let names_index = number(data, 62, 2)?;
    if entry_size != 64 || !(1..=4096).contains(&count) || names_index == 0 || names_index >= count
    {
        return Err(invalid("unsupported ELF section table"));
    }
    let table = bytes(data, offset, count * 64)?;
    let names_header = bytes(table, names_index * 64, 64)?;
    let names_size = number(names_header, 32, 8)?;
    if number(names_header, 4, 4)? != 3 || names_size > 1024 * 1024 {
        return Err(invalid("invalid ELF section-name table"));
    }
    let names = bytes(data, number(names_header, 24, 8)?, names_size)?;
    let mut sections = BTreeMap::new();
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
        if sections.insert(name, (kind, size, content)).is_some() {
            return Err(invalid("duplicate ELF section"));
        }
    }
    let mut protocol = Vec::new();
    let mut parameters = Vec::new();
    if let Some((kind, _, modinfo)) = sections.get(".modinfo") {
        if *kind != 1 || modinfo.len() > 64 * 1024 || modinfo.last() != Some(&0) {
            return Err(invalid("invalid module metadata"));
        }
        for record in modinfo.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let record = std::str::from_utf8(record).map_err(invalid)?;
            if !record.is_ascii() {
                return Err(invalid("non-ASCII module metadata"));
            }
            if let Some(value) = record.strip_prefix("vpnhide_preflight=") {
                protocol.push(value);
            }
            if record.starts_with("parmtype=preflight_only:") {
                parameters.push(record);
            }
        }
    }
    if protocol.is_empty() {
        return Ok(None);
    }
    if protocol != ["1"]
        || parameters != ["parmtype=preflight_only:bool"]
        || [".ctors", ".init_array", ".preinit_array"]
            .into_iter()
            .any(|name| sections.get(name).is_some_and(|(_, size, _)| *size != 0))
    {
        return Err(invalid("unverified preflight capability"));
    }
    Ok(Some(1))
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
}
