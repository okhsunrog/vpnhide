//! Bounded telemetry reader shared by consumers and host kernel tests.
use crate::{StatEntry, TELEMETRY_VERSION};
use std::collections::BTreeSet;

/// Reads a complete stats block, optionally following the proc status block.
/// Truncation, duplicate blocks/cells and malformed sparse rows are rejected.
pub fn parse_stats(bytes: &[u8]) -> Option<Vec<StatEntry>> {
    if bytes.len() > 1024 * 1024 || !bytes.ends_with(b"\n") {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;
    let header = format!("vpnhide {TELEMETRY_VERSION} stats");
    let mut found = false;
    let mut rows = Vec::new();
    let mut seen = BTreeSet::new();
    let hex = |s: &str| u64::from_str_radix(s.strip_prefix("0x")?, 16).ok();
    for line in text.lines() {
        if line == header {
            if found {
                return None;
            }
            found = true;
            continue;
        }
        if !found {
            continue;
        }
        let mut words = line.split_ascii_whitespace();
        let uid = u32::try_from(hex(words.next()?)?).ok()?;
        let mut any = false;
        for word in words {
            let (hook, count) = word.split_once(':')?;
            let hook_id = u32::try_from(hex(hook)?).ok()?;
            let count = hex(count)?;
            if count == 0 || !seen.insert((uid, hook_id)) || rows.len() >= 16384 {
                return None;
            }
            rows.push(StatEntry {
                uid,
                hook_id,
                count,
            });
            any = true;
        }
        if !any {
            return None;
        }
    }
    found.then_some(rows)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_proc_fixture_and_invalid_cells() {
        let bytes =
            b"vpnhide 1 read\nvpnhide 1 status\nbackend 0x1\nvpnhide 1 stats\n0x2710 0x1b:0x7\n";
        let stats = parse_stats(bytes).unwrap();
        assert_eq!(
            (stats[0].uid, stats[0].hook_id, stats[0].count),
            (10000, 27, 7)
        );
        for bad in [
            "vpnhide 1 stats\n0x2710 0x1b:0x0\n",
            "vpnhide 1 stats\n0x1 0x2:0x3 0x2:0x4\n",
            "vpnhide 1 stats\nvpnhide 1 stats\n",
            "vpnhide 1 stats\n0x1 0x2:0x3",
            "vpnhide 2 stats\n",
        ] {
            assert!(parse_stats(bad.as_bytes()).is_none(), "{bad}");
        }
    }
}
