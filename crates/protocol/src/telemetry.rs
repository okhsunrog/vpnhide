//! Bounded telemetry reader shared by consumers and host kernel tests.
use crate::{StatEntry, TELEMETRY_VERSION};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
pub const MAX_TELEMETRY_BYTES: usize = 1024 * 1024;
pub const MAX_STATS_CELLS: usize = 16_384;

/// Counter bits are unsigned on the wire. A decrease means a reset/unknown
/// interval, never an inferred wraparound. Missing baseline and an explicitly
/// captured empty baseline have different meanings. This is numeric evidence,
/// not proof that two snapshots share a backend/process generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CounterChange {
    Unavailable,
    Reset,
    Count(u64),
}

pub fn counter_change(current: u64, baseline: Option<u64>, captured: bool) -> CounterChange {
    if !captured {
        return CounterChange::Unavailable;
    }
    match baseline {
        Some(baseline) if current < baseline => CounterChange::Reset,
        baseline => CounterChange::Count(current - baseline.unwrap_or(0)),
    }
}

/// LSPosed's extensible metadata is display evidence, not a hook grant.
pub fn metadata(raw: &str) -> BTreeMap<String, String> {
    if raw.len() > MAX_TELEMETRY_BYTES {
        return BTreeMap::new();
    }
    raw.lines()
        .filter_map(|line| {
            let line = line.trim().strip_prefix("meta ")?;
            let (key, value) = line.split_once(' ').unwrap_or((line, ""));
            (!key.trim().is_empty()).then(|| (key.to_owned(), value.to_owned()))
        })
        .collect()
}

/// Tolerant telemetry display reader (§4.3–4.5). This does not certify a
/// complete counter capture: malformed cells are ignored and duplicate cells
/// remain separate, as required by the wire's extensible display grammar.
/// Qualification uses the stricter `parse_stats` below.
pub fn display_stats(bytes: &[u8]) -> Option<Vec<StatEntry>> {
    if bytes.len() > MAX_TELEMETRY_BYTES {
        return None;
    }
    let (kind, rest) = crate::parse_header(bytes)?;
    if kind != crate::Kind::Stats {
        return None;
    }
    let hex = |v: &[u8], bits| {
        v.strip_prefix(b"0x")
            .or_else(|| v.strip_prefix(b"0X"))
            .and_then(|v| crate::parse_hex_bare(v, bits))
    };
    let mut entries = Vec::new();
    for line in crate::lines(rest).filter_map(crate::significant) {
        let mut words = crate::tokens(line);
        let Some(uid) = words.next().and_then(|v| hex(v, 32)) else {
            continue;
        };
        for word in words {
            let mut pair = word.split(|b| *b == b':');
            let hook_id = pair.next().and_then(|v| hex(v, 32));
            let count = pair.next().and_then(|v| hex(v, 64));
            if let (Some(hook_id), Some(count), None) = (hook_id, count, pair.next()) {
                entries.push(StatEntry {
                    uid: uid as u32,
                    hook_id: hook_id as u32,
                    count,
                });
            }
        }
    }
    Some(entries)
}

/// Extract one concatenated proc block without interpreting its records. An
/// unsupported version remains a boundary, and its reader subsequently refuses
/// it; a later block cannot lend that rejected header valid-looking records.
pub fn extract_block(raw: &str, kind: crate::Kind) -> Option<String> {
    if raw.len() > MAX_TELEMETRY_BYTES {
        return None;
    }
    let classify = |line: &str| {
        let content = crate::significant(line.as_bytes())?;
        let mut words = crate::tokens(content);
        if words.next()? != b"vpnhide" {
            return None;
        }
        crate::parse_decimal(words.next()?)?;
        match words.next()? {
            b"config" => Some(crate::Kind::Config),
            b"stats" => Some(crate::Kind::Stats),
            b"status" => Some(crate::Kind::Status),
            _ => None,
        }
    };
    let lines = raw
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect::<Vec<_>>();
    let start = lines.iter().position(|line| classify(line) == Some(kind))?;
    let end = lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find_map(|(i, line)| {
            let content = crate::significant(line.as_bytes())?;
            (crate::tokens(content).next() == Some(b"vpnhide".as_slice())).then_some(i)
        })
        .unwrap_or(lines.len());
    Some(lines[start..end].join("\n"))
}

/// Reads a complete stats block, optionally following the proc status block.
/// Truncation, duplicate blocks/cells and malformed sparse rows are rejected.
pub fn parse_stats(bytes: &[u8]) -> Option<Vec<StatEntry>> {
    if bytes.len() > MAX_TELEMETRY_BYTES || !bytes.ends_with(b"\n") {
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
            if count == 0 || !seen.insert((uid, hook_id)) || rows.len() >= MAX_STATS_CELLS {
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
    fn counter_intervals_use_unsigned_order_and_never_guess_a_wrap() {
        for (current, baseline, expected) in [
            (0, None, CounterChange::Count(0)),
            (5, None, CounterChange::Count(5)),
            (10, Some(3), CounterChange::Count(7)),
            (9, Some(9), CounterChange::Count(0)),
            (2, Some(9), CounterChange::Reset),
            (u64::MAX, Some(0), CounterChange::Count(u64::MAX)),
            (1 << 63, Some((1 << 63) - 1), CounterChange::Count(1)),
            ((1 << 63) + 1, Some(1 << 63), CounterChange::Count(1)),
            (0, Some(u64::MAX), CounterChange::Reset),
        ] {
            assert_eq!(counter_change(current, baseline, true), expected);
            assert_eq!(
                counter_change(current, baseline, false),
                CounterChange::Unavailable
            );
        }
    }
    #[test]
    fn metadata_preserves_value_spaces_and_last_duplicate() {
        assert_eq!(
            metadata(
                "vpnhide 1 status\nmeta version old\n meta version 1.0 \r\nmeta detail two words\nmeta missing\nmeta  invalid\n"
            ),
            BTreeMap::from([
                ("version".into(), "1.0".into()),
                ("detail".into(), "two words".into()),
                ("missing".into(), "".into())
            ])
        );
    }
    #[test]
    fn display_retains_unsigned_cells_and_skips_only_bad_records() {
        let input = b"# banner\r\nvpnhide 1 stats\r\n0XFFFFFFFF 0X0:0XFFFFFFFFFFFFFFFF bad 0x1:0x0 0x1:0x7:0x9\n0x2 0x3:0x4 0x3:0x5\n0x100000000 0x0:0x1\n0x3 0x100000000:0x1\n0x3 0x0:0x10000000000000000\n";
        assert_eq!(
            display_stats(input),
            Some(vec![
                StatEntry {
                    uid: u32::MAX,
                    hook_id: 0,
                    count: u64::MAX
                },
                StatEntry {
                    uid: u32::MAX,
                    hook_id: 1,
                    count: 0
                },
                StatEntry {
                    uid: 2,
                    hook_id: 3,
                    count: 4
                },
                StatEntry {
                    uid: 2,
                    hook_id: 3,
                    count: 5
                },
            ])
        );
        assert!(parse_stats(input).is_none());
        for input in [
            b"vpnhide +1 stats\n".as_slice(),
            b"vpnhide 2 stats\n",
            b"vpnhide 1 status\n",
            b"\xff\nvpnhide 1 stats\n",
        ] {
            assert!(display_stats(input).is_none());
        }
    }
    #[test]
    fn block_boundaries_do_not_admit_following_or_rejected_protocol_rows() {
        let input = "meta version 1.0\r\nvpnhide 1 status\r\nbackend 0x2\r\nvpnhide 1 stats\r\n0x1 0x2:0x3\r\nvpnhide +1 status\r\n0x9 0x9:0x9\r\n";
        assert_eq!(
            extract_block(input, crate::Kind::Status).as_deref(),
            Some("vpnhide 1 status\nbackend 0x2")
        );
        let stats = extract_block(input, crate::Kind::Stats).unwrap();
        assert_eq!(
            display_stats(stats.as_bytes()),
            Some(vec![StatEntry {
                uid: 1,
                hook_id: 2,
                count: 3
            }])
        );
        assert_eq!(
            extract_block("vpnhide 99 stats\n0x1 0x2:0x3", crate::Kind::Stats).as_deref(),
            Some("vpnhide 99 stats\n0x1 0x2:0x3")
        );
        assert!(
            display_stats(
                extract_block("vpnhide 99 stats\n", crate::Kind::Stats)
                    .unwrap()
                    .as_bytes()
            )
            .is_none()
        );
    }
    #[test]
    fn oversized_display_inputs_are_refused_and_qualification_stays_strict() {
        let input = format!("vpnhide 1 stats\n{}", " ".repeat(MAX_TELEMETRY_BYTES));
        assert!(display_stats(input.as_bytes()).is_none());
        assert!(extract_block(&input, crate::Kind::Stats).is_none());
        for input in [
            "vpnhide 1 stats\n0x1 0x2:0x0\n",
            "vpnhide 1 stats\n0x1 0x2:0x3 0x2:0x4\n",
            "vpnhide 1 stats\n0x1 0x2:0x3",
        ] {
            assert!(display_stats(input.as_bytes()).is_some());
            assert!(parse_stats(input.as_bytes()).is_none());
        }
    }
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
