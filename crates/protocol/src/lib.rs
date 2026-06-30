//! Rust side of the vpnhide control/stats wire format (docs/protocol.md §4).
//!
//! This mirrors the freestanding C in `kmod/shared/vpnhide_logic.h` byte for
//! byte. The two are NOT shared code — parity is held by the language-
//! independent golden vectors in `kmod/shared/protocol_vectors.tsv`, which the
//! tests at the bottom run against this implementation (protocol §8 Layer 1).
//! When the spec changes, change both sides and add a vector.
//!
//! Roles (§1.4): backends PARSE config and EMIT stats/status. The activator
//! also uses this crate to FORMAT config snapshots for native backends.

#![allow(dead_code)]

pub mod generated;

pub use generated::hook_ids;

/// Wire version this implementation speaks (header line, §4.2).
pub const PROTO_VERSION: u32 = 1;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Kind {
    Config,
    Stats,
    Status,
}

/// One `target <uid> <hookmask>` record (§4.3).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Target {
    pub uid: u32,
    pub hookmask: u32,
}

/// A parsed `config` snapshot. `debug` is `None` when no `debug` line was
/// present ("unchanged from default", §4.3), else `Some(flag)`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Config {
    pub debug: Option<bool>,
    pub targets: Vec<Target>,
}

/// One sparse `<hook_id>:<count>` stats cell for a uid (§4.3). Producers group
/// consecutive entries by uid; [`format_stats`] emits one line per uid run.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct StatEntry {
    pub uid: u32,
    pub hook_id: u32,
    pub count: u64,
}

/// A `status` snapshot (§4.3): backend id, kernel version, installed-hook mask,
/// dominant error code.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Status {
    pub backend: u32,
    pub kver: u32,
    pub hooks: u32,
    pub error: u32,
}

// --- lexical helpers (§4.1) ------------------------------------------------

fn is_sep(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

fn is_ascii_printable(b: u8) -> bool {
    (0x20..=0x7e).contains(&b)
}

/// Iterator over physical lines of `buf`, yielding each line's content with a
/// trailing CR stripped (CRLF → LF, §4.1).
fn lines(buf: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut i = 0usize;
    let len = buf.len();
    core::iter::from_fn(move || {
        if i >= len {
            return None;
        }
        let s = i;
        let mut e = s;
        while e < len && buf[e] != b'\n' {
            e += 1;
        }
        i = if e < len { e + 1 } else { len };
        let mut end = e;
        if end > s && buf[end - 1] == b'\r' {
            end -= 1;
        }
        Some(&buf[s..end])
    })
}

/// Classify a line: `Some(content_after_leading_ws)` if significant (has a
/// token, not a `#` comment) AND fully ASCII (tab allowed as a separator);
/// `None` for blank / comment / non-ASCII lines.
fn significant(line: &[u8]) -> Option<&[u8]> {
    if line.iter().any(|&b| !is_ascii_printable(b) && b != b'\t') {
        return None; // non-ASCII ⇒ reject this line (§4.1)
    }
    let start = line.iter().position(|&b| !is_sep(b))?;
    let rest = &line[start..];
    if rest[0] == b'#' {
        return None; // comment
    }
    Some(rest)
}

/// True for a line that may legally precede the header (§4.2): blank /
/// whitespace-only, or a `#` comment. A line carrying any non-comment token —
/// **even a non-ASCII one** — is NOT ignorable: the first such line must be the
/// header. This is deliberately narrower than [significant], which also folds in
/// the ASCII check; the header scan needs to tell "skip this" apart from
/// "significant but invalid", to match the C and Kotlin parsers which reject the
/// whole payload when the first significant line is non-ASCII.
fn is_ignorable(line: &[u8]) -> bool {
    match line.iter().position(|&b| !is_sep(b)) {
        None => true,                       // blank / whitespace-only
        Some(start) => line[start] == b'#', // comment
    }
}

/// Whitespace-delimited tokens of a line (runs of separators collapse, §4.1).
fn tokens(line: &[u8]) -> impl Iterator<Item = &[u8]> {
    line.split(|&b| is_sep(b)).filter(|t| !t.is_empty())
}

/// Parse the one numeric primitive (§4.4): `0x` (mandatory) + ≥1 hex digit, any
/// case on read; reject if it overflows `BITS` (32/64). Returns `None` on any
/// malformation — the caller skips the line (or rejects, for the header).
fn parse_hex(tok: &[u8], bits: u32) -> Option<u64> {
    if tok.len() < 3 || tok[0] != b'0' || (tok[1] != b'x' && tok[1] != b'X') {
        return None;
    }
    let max: u64 = if bits >= 64 {
        u64::MAX
    } else {
        u32::MAX as u64
    };
    let mut v: u64 = 0;
    for &c in &tok[2..] {
        let d = match c {
            b'0'..=b'9' => (c - b'0') as u64,
            b'a'..=b'f' => (c - b'a' + 10) as u64,
            b'A'..=b'F' => (c - b'A' + 10) as u64,
            _ => return None,
        };
        if v > (max - d) / 16 {
            return None; // width overflow
        }
        v = v * 16 + d;
    }
    Some(v)
}

// --- header (§4.2) ---------------------------------------------------------

/// Parse the mandatory header line, returning `(kind, rest_after_header)`.
/// `None` (reject whole) when the header is missing/malformed or its version is
/// newer than this reader knows (§3 version fuse).
fn parse_header(buf: &[u8]) -> Option<(Kind, &[u8])> {
    let len = buf.len();
    let mut i = 0usize;
    while i < len {
        let s = i;
        let mut e = s;
        while e < len && buf[e] != b'\n' {
            e += 1;
        }
        i = if e < len { e + 1 } else { len }; // start of the following line
        let mut end = e;
        if end > s && buf[end - 1] == b'\r' {
            end -= 1; // CRLF → LF (§4.1)
        }

        let line = &buf[s..end];
        if is_ignorable(line) {
            continue; // blank / comment before the header is allowed
        }
        // The first significant line MUST be the header. If it is significant
        // but non-ASCII, `significant` returns None and `?` rejects the whole
        // payload rather than skipping it — parity with the C (kmod/KPM) and
        // Kotlin (LSPosed) parsers (§4.2).
        let content = significant(line)?;
        // first significant line == the mandatory header
        let mut it = tokens(content);
        if it.next() != Some(b"vpnhide".as_slice()) {
            return None;
        }
        let ver = it.next().and_then(parse_decimal)?;
        if ver > PROTO_VERSION {
            return None;
        }
        let kind = match it.next()? {
            b"config" => Kind::Config,
            b"stats" => Kind::Stats,
            b"status" => Kind::Status,
            _ => return None,
        };
        return Some((kind, &buf[i..]));
    }
    None
}

fn parse_decimal(tok: &[u8]) -> Option<u32> {
    if tok.is_empty() {
        return None;
    }
    let mut v: u32 = 0;
    for &c in tok {
        if !c.is_ascii_digit() {
            return None;
        }
        v = v.checked_mul(10)?.checked_add((c - b'0') as u32)?;
    }
    Some(v)
}

/// Sniff a payload's kind from its header (transport dispatch, §7.1).
pub fn peek_kind(buf: &[u8]) -> Option<Kind> {
    parse_header(buf).map(|(k, _)| k)
}

// --- config parse (§4.3) ---------------------------------------------------

/// Parse a `config` payload. `None` if rejected whole (bad/missing header,
/// version too new, or a non-config kind). Unknown keywords and malformed
/// numeric lines are skipped (§4.5); duplicate uid ⇒ last wins (§4.3).
pub fn parse_config(buf: &[u8]) -> Option<Config> {
    let (kind, rest) = parse_header(buf)?;
    if kind != Kind::Config {
        return None;
    }
    let mut cfg = Config {
        debug: None,
        targets: Vec::new(),
    };
    for line in lines(rest) {
        let Some(content) = significant(line) else {
            continue;
        };
        let mut it = tokens(content);
        match it.next() {
            Some(b"debug") => match it.next() {
                Some(b"0") => cfg.debug = Some(false),
                Some(b"1") => cfg.debug = Some(true),
                _ => {} // malformed flag ⇒ skip
            },
            Some(b"target") => {
                let (Some(uid), Some(hm)) = (
                    it.next().and_then(|t| parse_hex(t, 32)),
                    it.next().and_then(|t| parse_hex(t, 32)),
                ) else {
                    continue; // malformed ⇒ skip line
                };
                set_target(&mut cfg.targets, uid as u32, hm as u32);
            }
            _ => {} // unknown keyword ⇒ skip line (§4.5)
        }
    }
    Some(cfg)
}

fn set_target(targets: &mut Vec<Target>, uid: u32, hookmask: u32) {
    if let Some(t) = targets.iter_mut().find(|t| t.uid == uid) {
        t.hookmask = hookmask; // last wins, keeps position
    } else {
        targets.push(Target { uid, hookmask });
    }
}

// --- serialise (§4.3/§4.4) -------------------------------------------------

/// Serialise a `config` snapshot (lowercase-out hex, §4.4).
pub fn format_config(debug: bool, targets: &[Target]) -> String {
    let mut out = String::from("vpnhide 1 config\n");
    out.push_str(if debug { "debug 1\n" } else { "debug 0\n" });
    for t in targets {
        out.push_str(&format!("target 0x{:x} 0x{:x}\n", t.uid, t.hookmask));
    }
    out
}

/// Serialise a `stats` snapshot. Entries grouped by uid (consecutive same-uid
/// entries share a line); lowercase-out hex (§4.4).
pub fn format_stats(entries: &[StatEntry]) -> String {
    let mut out = String::from("vpnhide 1 stats\n");
    let mut i = 0;
    while i < entries.len() {
        let uid = entries[i].uid;
        out.push_str(&format!("0x{uid:x}"));
        while i < entries.len() && entries[i].uid == uid {
            out.push_str(&format!(
                " 0x{:x}:0x{:x}",
                entries[i].hook_id, entries[i].count
            ));
            i += 1;
        }
        out.push('\n');
    }
    out
}

/// Serialise a `status` snapshot (lowercase-out hex, §4.4).
pub fn format_status(s: &Status) -> String {
    format!(
        "vpnhide 1 status\nbackend 0x{:x}\nkver 0x{:x}\nhooks 0x{:x}\nerror 0x{:x}\n",
        s.backend, s.kver, s.hooks, s.error
    )
}

/// Clamp a fully-serialised snapshot to `outlen` bytes on a LINE boundary, for
/// a single-buffer transport (§7.2). Returns the clamped length: the whole
/// snapshot if it fits (ends in `\n`, complete), else the largest run of
/// complete lines minus its trailing `\n` (the absent newline signals
/// truncation; no record is ever cut mid-line).
pub fn clamp_to_line(buf: &[u8], outlen: usize) -> usize {
    if buf.len() <= outlen {
        return buf.len();
    }
    let mut p = outlen;
    while p > 0 && buf[p - 1] != b'\n' {
        p -= 1;
    }
    if p > 0 { p - 1 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn vectors_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../kmod/shared/protocol_vectors.tsv")
    }

    /// Decode the `\n \t \r \\ \xNN` escapes used in the vectors file.
    fn decode(s: &str) -> Vec<u8> {
        let b = s.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'\\' && i + 1 < b.len() {
                match b[i + 1] {
                    b'n' => {
                        out.push(b'\n');
                        i += 2;
                    }
                    b't' => {
                        out.push(b'\t');
                        i += 2;
                    }
                    b'r' => {
                        out.push(b'\r');
                        i += 2;
                    }
                    b'\\' => {
                        out.push(b'\\');
                        i += 2;
                    }
                    b'x' if i + 3 < b.len() => {
                        let h = std::str::from_utf8(&b[i + 2..i + 4]).unwrap();
                        out.push(u8::from_str_radix(h, 16).unwrap());
                        i += 4;
                    }
                    _ => {
                        out.push(b[i]);
                        i += 1;
                    }
                }
            } else {
                out.push(b[i]);
                i += 1;
            }
        }
        out
    }

    fn decode_str(s: &str) -> String {
        String::from_utf8(decode(s)).unwrap()
    }

    fn run_cfg(input: &str, expect: &str) {
        let cfg = parse_config(&decode(input));
        if expect == "REJECT" {
            assert!(cfg.is_none(), "expected REJECT for {input:?}");
            return;
        }
        let cfg = cfg.unwrap_or_else(|| panic!("unexpected REJECT for {input:?}"));
        let dbg = match cfg.debug {
            None => -1,
            Some(false) => 0,
            Some(true) => 1,
        };
        let mut got = format!("debug={dbg}");
        for t in &cfg.targets {
            got.push_str(&format!(";0x{:x}:0x{:x}", t.uid, t.hookmask));
        }
        assert_eq!(got, expect, "cfg mismatch for {input:?}");
    }

    fn run_kind(input: &str, expect: &str) {
        let got = match peek_kind(&decode(input)) {
            Some(Kind::Config) => "CONFIG",
            Some(Kind::Stats) => "STATS",
            Some(Kind::Status) => "STATUS",
            None => "INVALID",
        };
        assert_eq!(got, expect, "kind mismatch for {input:?}");
    }

    fn run_stats(entries: &str, expect: &str) {
        let mut e = Vec::new();
        for grp in decode_str(entries).split(';') {
            if grp.is_empty() {
                continue;
            }
            let parts: Vec<&str> = grp.split(',').collect();
            if parts.len() != 3 {
                continue;
            }
            let p = |s: &str| u64::from_str_radix(s.trim().trim_start_matches("0x"), 16).unwrap();
            e.push(StatEntry {
                uid: p(parts[0]) as u32,
                hook_id: p(parts[1]) as u32,
                count: p(parts[2]),
            });
        }
        assert_eq!(format_stats(&e), decode_str(expect), "stats mismatch");
    }

    #[test]
    fn formats_config_snapshot() {
        let targets = [
            Target {
                uid: 0x27fa,
                hookmask: 0x3ff,
            },
            Target {
                uid: 0x2947,
                hookmask: 0x4,
            },
        ];
        assert_eq!(
            format_config(false, &targets),
            "vpnhide 1 config\ndebug 0\ntarget 0x27fa 0x3ff\ntarget 0x2947 0x4\n",
        );
    }

    fn run_status(fields: &str, expect: &str) {
        let f = decode_str(fields);
        let v: Vec<&str> = f.split(',').collect();
        let p = |s: &str| u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).unwrap();
        let s = Status {
            backend: p(v[0]),
            kver: p(v[1]),
            hooks: p(v[2]),
            error: p(v[3]),
        };
        assert_eq!(format_status(&s), decode_str(expect), "status mismatch");
    }

    fn run_clamp(full: &str, outlen: &str, expect: &str) {
        let b = decode(full);
        let n = clamp_to_line(&b, outlen.parse().unwrap());
        assert_eq!(&b[..n], decode(expect).as_slice(), "clamp mismatch");
    }

    #[test]
    fn golden_vectors() {
        let text = std::fs::read_to_string(vectors_path()).expect("read vectors");
        let mut count = 0;
        for line in text.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let f: Vec<&str> = line.split('|').collect();
            match f.as_slice() {
                ["cfg", input, expect] => run_cfg(input, expect),
                ["kind", input, expect] => run_kind(input, expect),
                ["stats", entries, expect] => run_stats(entries, expect),
                ["status", fields, expect] => run_status(fields, expect),
                ["clamp", full, outlen, expect] => run_clamp(full, outlen, expect),
                _ => panic!("unrecognised vector: {line}"),
            }
            count += 1;
        }
        assert!(count >= 30, "expected the full vector set, ran {count}");
    }
}
