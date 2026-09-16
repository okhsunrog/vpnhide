//! The shared proc node belongs to exactly one kernel backend. Never treat an
//! unreadable or malformed node as absence, or configure it with another family.
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Seek, Write};
use std::path::Path;

use crate::Result;

pub(crate) const BUILTIN_BACKEND_ID: u32 = 4;
pub(crate) const KMOD_BACKEND_ID: u32 = 0;

pub(crate) fn parse_backend(text: &str) -> Result<u32> {
    let mut status = false;
    let mut fields = std::collections::BTreeMap::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if !status {
            if line != "vpnhide 1 status" {
                return Err("kernel control: missing status header".into());
            }
            status = true;
            continue;
        }
        if line.starts_with("vpnhide ") {
            break;
        }
        let words: Vec<_> = line.split_whitespace().collect();
        if words.len() != 2 || !["backend", "kver", "hooks", "error"].contains(&words[0]) {
            return Err("kernel control: malformed status field".into());
        }
        let value = u32::from_str_radix(words[1].strip_prefix("0x").unwrap_or(words[1]), 16)?;
        if fields.insert(words[0], value).is_some() {
            return Err("kernel control: duplicate status field".into());
        }
    }
    if fields.len() != 4 {
        return Err("kernel control: incomplete status".into());
    }
    Ok(fields["backend"])
}

pub(crate) fn observed_backend(path: &Path) -> Result<Option<u32>> {
    match fs::read_to_string(path) {
        Ok(text) => parse_backend(&text).map(Some),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

pub(crate) fn load_kmod_if_absent<T>(
    observed: Result<Option<u32>>,
    load: impl FnOnce() -> Result<T>,
) -> Result<T> {
    match observed? {
        None => load(),
        Some(BUILTIN_BACKEND_ID) => Err("built-in VPN Hide is already active; remove the unnecessary kmod companion (insmod not attempted)".into()),
        Some(id) => Err(format!("kernel control already belongs to backend 0x{id:x}; insmod not attempted").into()),
    }
}

/// Check the same open proc file that receives the write. No truncation and no
/// reopen between identity validation and delivery to a potentially new owner.
pub(crate) fn write_config(path: &Path, expected: u32, wire: &str) -> Result<()> {
    let mut ctl = OpenOptions::new().read(true).write(true).open(path)?;
    let mut status = String::new();
    ctl.read_to_string(&mut status)?;
    let backend = parse_backend(&status)?;
    if backend != expected {
        return Err(format!(
            "refusing config: expected backend 0x{expected:x}, found 0x{backend:x}"
        )
        .into());
    }
    ctl.rewind()?;
    if ctl.write(wire.as_bytes())? != wire.len() {
        return Err("kernel control: short config write; not retrying a partial snapshot".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn only_absence_invokes_loader() {
        for observed in [
            Ok(Some(BUILTIN_BACKEND_ID)),
            Ok(Some(KMOD_BACKEND_ID)),
            Ok(Some(99)),
            Err(io::Error::from(io::ErrorKind::PermissionDenied).into()),
            parse_backend("garbage").map(Some),
        ] {
            let invoked = Cell::new(false);
            assert!(
                load_kmod_if_absent(observed, || {
                    invoked.set(true);
                    Ok(())
                })
                .is_err()
            );
            assert!(!invoked.get());
        }
        let invoked = Cell::new(false);
        load_kmod_if_absent(Ok(None), || {
            invoked.set(true);
            Ok(())
        })
        .unwrap();
        assert!(invoked.get());
    }

    #[test]
    fn parses_status_before_stats_and_rejects_ambiguous_identity() {
        let text =
            "# comment\nvpnhide 1 status\nbackend 0x4\nkver 0x60100\nhooks 0x3ff\nerror 0x0\n";
        assert_eq!(
            parse_backend(&format!("{text}vpnhide 1 stats\n0x1234 0x0:0x1\n")).unwrap(),
            4
        );
        assert!(parse_backend(&format!("{text}backend 0x0\n")).is_err());
        assert!(parse_backend("vpnhide 1 status\nbackend 0x4\n").is_err());
    }

    #[test]
    fn foreign_or_malformed_control_is_not_modified() {
        let path = std::env::temp_dir().join(format!("vpnhide-control-{}", std::process::id()));
        for text in [
            "vpnhide 1 status\nbackend 0x4\nkver 0x0\nhooks 0x0\nerror 0x0\n",
            "invalid",
        ] {
            fs::write(&path, text).unwrap();
            assert!(write_config(&path, KMOD_BACKEND_ID, "new config").is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), text);
        }
        fs::remove_file(path).unwrap();
    }
}
