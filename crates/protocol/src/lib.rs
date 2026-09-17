//! Rust side of the vpnhide control/stats wire format (docs/protocol.md §4).
//!
//! This mirrors the freestanding C in `kmod/shared/vpnhide_logic.h` byte for
//! byte. The two are NOT shared code — parity is held by the language-
//! independent golden vectors in `kmod/shared/protocol_vectors.tsv`, which the
//! tests at the bottom run against this implementation (protocol §8 Layer 1).
//! When the spec changes, change both sides and add a vector.
//!
//! Roles (§1.3): backends PARSE config and EMIT stats/status. The activator
//! also uses this crate to FORMAT config snapshots for native backends.

use core::iter;

pub mod generated;

pub use generated::hook_ids;

/// Version of the **control** protocol — the `config` payload, activator →
/// backend. Its reader is the module that ships in the same flashable zip as
/// the activator writing it, so the two never meet across a version boundary
/// and a bump costs nothing.
pub const CONTROL_VERSION: u32 = 2;

/// Version of the **telemetry** protocol — the `stats` and `status` payloads,
/// backend → app. One `/proc/vpnhide_ctl` read returns both back to back, so
/// they have one reader, one delivery, and therefore one version: bumping them
/// apart could not express anything. That reader is the **app**, which updates
/// independently of the modules, so a bump here breaks an older APK's dashboard
/// and diagnostics. Do not move it without shipping the APK in step.
pub const TELEMETRY_VERSION: u32 = 1;

/// Size of KernelPatch's fixed KPM control-argument buffer, mirrored from
/// `kmod/third_party/KernelPatch/kernel/include/kpmodule.h`.
///
/// A payload must be strictly shorter than this value because the transport
/// stores a trailing NUL in the same buffer. Keep the runtime check in the
/// activator and the capacity guard test below tied to this constant.
pub const KPM_ARGS_LEN: usize = 1024;

/// Exact version this implementation reads for `kind`'s protocol.
fn current_version(kind: Kind) -> u32 {
    match kind {
        Kind::Config => CONTROL_VERSION,
        Kind::Stats | Kind::Status => TELEMETRY_VERSION,
    }
}

/// Maximum number of `target` records a native backend will store. The backends
/// keep a fixed `targets[MAX_TARGET_UIDS]` array, so a config carrying more than
/// this many native targets is truncated on projection. This is the single
/// source of truth for the cap on the wire boundary; the C backends mirror it as
/// `#define MAX_TARGET_UIDS` in kmod/vpnhide_kmod.c, kmod/kpm/vpnhide_kpm.c and
/// builtin/security/vpnhide/vpnhide_internal.h — keep all four in sync.
pub const MAX_TARGET_UIDS: usize = 160;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Kind {
    Config,
    Stats,
    Status,
}

/// One parsed UID-to-hookmask entry from a grouped `targets` record (§4.3).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Target {
    pub uid: u32,
    pub hookmask: u32,
}

/// A parsed `config` snapshot. `debug` is `None` when no `debug` line was
/// present ("unchanged from default", §4.3), else `Some(flag)`.
///
/// `default_mask` is the hookmask for any uid **not** in `targets`. Zero — the
/// value when no `default` record is present — makes `targets` the set of apps
/// to act on, which is the blacklist the project ships today. A non-zero default
/// inverts that: everyone is acted on and `targets` becomes the exception list.
/// The wire carries the mechanism; choosing to emit a non-zero default is a
/// producer decision.
///
/// `targets` is **sorted ascending by uid**, which is what lets a backend
/// binary-search it on every hooked call instead of walking it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Config {
    pub debug: Option<bool>,
    pub default_mask: u32,
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
    iter::from_fn(move || {
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

/// Bare hex, no `0x`: the numeric primitive of the v2 `config` payload (§4.4).
/// The prefix costs two bytes on every number, and the KPM's config transport
/// caps the whole payload at 1024 bytes — that is a real ceiling on how many
/// apps fit, so the prefix buys nothing worth its width here. Telemetry output
/// remains prefixed because its wire formats are still version 1.
fn parse_hex_bare(tok: &[u8], bits: u32) -> Option<u64> {
    if tok.is_empty() {
        return None;
    }
    let max: u64 = if bits >= 64 {
        u64::MAX
    } else {
        u32::MAX as u64
    };
    let mut v: u64 = 0;
    for &c in tok {
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
/// not the current version **for that kind** (§3 version fuse).
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
        let kind = match it.next()? {
            b"config" => Kind::Config,
            b"stats" => Kind::Stats,
            b"status" => Kind::Status,
            _ => return None,
        };
        // The fuse is per kind: the version is only meaningful once we know
        // which payload it labels.
        if ver != current_version(kind) {
            return None;
        }
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

/// Telemetry fields preserve absence: a valid header alone must not establish
/// backend zero. Display consumers may default missing fields; ownership checks
/// require a complete observation. Unknown/malformed records are skipped (§4.5).
#[derive(Default, Debug, PartialEq, Eq)]
pub struct StatusFields {
    pub backend: Option<u32>,
    pub kver: Option<u32>,
    pub hooks: Option<u32>,
    pub error: Option<u32>,
}

impl StatusFields {
    pub fn complete(self) -> Option<Status> {
        Some(Status {
            backend: self.backend?,
            kver: self.kver?,
            hooks: self.hooks?,
            error: self.error?,
        })
    }
}

pub fn parse_status_fields(buf: &[u8]) -> Option<StatusFields> {
    let (kind, rest) = parse_header(buf)?;
    if kind != Kind::Status {
        return None;
    }
    let mut fields = StatusFields::default();
    for line in lines(rest).filter_map(significant) {
        let mut words = tokens(line);
        let key = words.next()?;
        // A proc read concatenates status and stats; stop at the next block.
        if key == b"vpnhide" {
            break;
        }
        let Some(value) = words.next().and_then(|v| {
            v.strip_prefix(b"0x")
                .or_else(|| v.strip_prefix(b"0X"))
                .and_then(|hex| parse_hex_bare(hex, 32))
        }) else {
            continue;
        };
        if words.next().is_some() {
            continue;
        }
        match key {
            b"backend" => fields.backend = Some(value as u32),
            b"kver" => fields.kver = Some(value as u32),
            b"hooks" => fields.hooks = Some(value as u32),
            b"error" => fields.error = Some(value as u32),
            _ => (),
        }
    }
    Some(fields)
}

// --- config parse (§4.3) ---------------------------------------------------

/// Parse a `config` payload. `None` if rejected whole.
///
/// Rejected whole: bad/missing header, wrong version, a non-config kind, a
/// missing or mismatched `end` record, or more uids than [`MAX_TARGET_UIDS`].
/// Unknown keywords are still skipped (§4.5) so the grammar stays extensible.
///
/// The `end <count>` fuse is what makes a truncated payload fail closed. The
/// KPM transport copies the config through a fixed 1024-byte buffer and
/// truncates silently when it overflows, so "parse whatever prefix arrived"
/// means quietly applying a partial target set — the exact failure this rejects.
pub fn parse_config(buf: &[u8]) -> Option<Config> {
    let (kind, rest) = parse_header(buf)?;
    if kind != Kind::Config {
        return None;
    }
    let mut cfg = Config {
        debug: None,
        default_mask: 0,
        targets: Vec::new(),
    };
    let mut uids_seen: u64 = 0;
    let mut declared: Option<u64> = None;
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
            Some(b"default") => {
                let Some(mask) = it.next().and_then(|t| parse_hex_bare(t, 32)) else {
                    continue; // malformed ⇒ skip line
                };
                cfg.default_mask = mask as u32;
            }
            Some(b"targets") => {
                let Some(mask) = it.next().and_then(|t| parse_hex_bare(t, 32)) else {
                    continue; // malformed group head ⇒ skip line
                };
                for tok in it {
                    // A malformed uid inside a group is NOT skipped: it would
                    // desync the `end` count, so the payload is rejected.
                    let uid = parse_hex_bare(tok, 32)?;
                    uids_seen += 1;
                    if !set_target(&mut cfg.targets, uid as u32, mask as u32) {
                        return None; // over the backend ceiling
                    }
                }
            }
            Some(b"end") => {
                // A malformed count leaves any earlier `end` standing, matching
                // how every other malformed record is skipped.
                if let Some(v) = it.next().and_then(|t| parse_hex_bare(t, 32)) {
                    declared = Some(v);
                }
            }
            _ => {} // unknown keyword ⇒ skip line (§4.5)
        }
    }
    if declared? != uids_seen {
        return None;
    }
    Some(cfg)
}

/// Insert keeping `targets` sorted by uid; duplicate uid ⇒ last wins (§4.3).
/// Returns false when the set is full and the uid is new — the caller rejects
/// the payload rather than dropping the overflow, because a config the backend
/// cannot hold in full means producer and backend disagree about the ceiling.
///
/// Sorted-on-parse is a contract, not an optimisation: it is what lets the
/// kernel backends binary-search the array on every hooked call, and it makes
/// the parsed form independent of the order the producer grouped uids in.
fn set_target(targets: &mut Vec<Target>, uid: u32, hookmask: u32) -> bool {
    match targets.binary_search_by_key(&uid, |t| t.uid) {
        Ok(i) => {
            targets[i].hookmask = hookmask;
            true
        }
        Err(i) => {
            if targets.len() >= MAX_TARGET_UIDS {
                return false;
            }
            targets.insert(i, Target { uid, hookmask });
            true
        }
    }
}

// --- serialise (§4.3/§4.4) -------------------------------------------------

/// Serialise a `config` snapshot (bare lowercase-out hex, §4.4).
///
/// Targets are grouped by hookmask, one `targets` record per distinct mask.
/// That is where the density comes from: a per-uid record spends its keyword and
/// its mask on every app, and in practice almost every app carries the same
/// mask, so one group head amortises across the whole set. Masks ascending, and
/// uids ascending within a group, so the output is a function of the input set
/// alone.
pub fn format_config(debug: bool, default_mask: u32, targets: &[Target]) -> String {
    let mut out = format!("vpnhide {CONTROL_VERSION} config\n");
    out.push_str(if debug { "debug 1\n" } else { "debug 0\n" });
    if default_mask != 0 {
        out.push_str(&format!("default {default_mask:x}\n"));
    }
    let mut masks: Vec<u32> = targets.iter().map(|t| t.hookmask).collect();
    masks.sort_unstable();
    masks.dedup();
    let mut count = 0usize;
    for mask in masks {
        out.push_str(&format!("targets {mask:x}"));
        let mut uids: Vec<u32> = targets
            .iter()
            .filter(|t| t.hookmask == mask)
            .map(|t| t.uid)
            .collect();
        uids.sort_unstable();
        for uid in uids {
            out.push_str(&format!(" {uid:x}"));
            count += 1;
        }
        out.push('\n');
    }
    out.push_str(&format!("end {count:x}\n"));
    out
}

/// Serialise a `stats` snapshot. Entries grouped by uid (consecutive same-uid
/// entries share a line); lowercase-out hex (§4.4).
pub fn format_stats(entries: &[StatEntry]) -> String {
    let mut out = format!("vpnhide {TELEMETRY_VERSION} stats\n");
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
        "vpnhide {TELEMETRY_VERSION} status\nbackend 0x{:x}\nkver 0x{:x}\nhooks 0x{:x}\nerror 0x{:x}\n",
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
        let mut got = format!("debug={dbg};def=0x{:x}", cfg.default_mask);
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

    /// The producer has no counterpart in C, so the vectors cannot hold it —
    /// these pin the serialise direction and the density it buys.
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
            format_config(false, 0, &targets),
            "vpnhide 2 config\ndebug 0\ntargets 4 2947\ntargets 3ff 27fa\nend 2\n",
        );
    }

    #[test]
    fn groups_targets_sharing_a_mask_onto_one_record() {
        // The whole point of the v2 shape: one group head amortises across every
        // app carrying the same mask, which is the normal case. Per-uid records
        // would spend a keyword and a mask on each.
        let targets: Vec<Target> = (0..8)
            .map(|i| Target {
                uid: 0x2710 + i,
                hookmask: 0x3ff,
            })
            .collect();
        let wire = format_config(false, 0, &targets);
        assert_eq!(
            wire,
            "vpnhide 2 config\ndebug 0\ntargets 3ff 2710 2711 2712 2713 2714 2715 2716 2717\nend 8\n",
        );
        assert_eq!(parse_config(wire.as_bytes()).unwrap().targets, targets);
    }

    #[test]
    fn representative_full_target_set_fits_the_kpm_transport() {
        // Ordinary Android app UIDs use four or five hexadecimal digits across
        // owner/work-profile sets. The activator separately checks the actual
        // wire length because unusual maximum-width UIDs or many distinct
        // masks can still exceed the fixed KernelPatch transport independently
        // of the target count.
        let targets: Vec<Target> = (0..MAX_TARGET_UIDS as u32)
            .map(|offset| {
                let half = MAX_TARGET_UIDS as u32 / 2;
                Target {
                    uid: if offset < half {
                        10_000 + offset
                    } else {
                        1_010_000 + offset - half
                    },
                    hookmask: u32::MAX,
                }
            })
            .collect();
        let wire = format_config(true, u32::MAX, &targets);

        assert!(
            wire.len() < KPM_ARGS_LEN,
            "{MAX_TARGET_UIDS} representative Android targets need {} bytes plus NUL, KPM_ARGS_LEN is {KPM_ARGS_LEN}",
            wire.len(),
        );
    }

    #[test]
    fn round_trips_a_default_mask() {
        let targets = [Target {
            uid: 0x2710,
            hookmask: 0,
        }];
        let wire = format_config(true, 0x3ff, &targets);
        assert_eq!(
            wire,
            "vpnhide 2 config\ndebug 1\ndefault 3ff\ntargets 0 2710\nend 1\n",
        );
        let cfg = parse_config(wire.as_bytes()).unwrap();
        assert_eq!(cfg.default_mask, 0x3ff);
        assert_eq!(cfg.debug, Some(true));
    }

    #[test]
    fn rejects_a_target_set_over_the_backend_ceiling() {
        let targets: Vec<Target> = (0..=MAX_TARGET_UIDS as u32)
            .map(|i| Target {
                uid: 0x2710 + i,
                hookmask: 0x3ff,
            })
            .collect();
        assert_eq!(targets.len(), MAX_TARGET_UIDS + 1);
        // A producer that overshoots must not have its overflow silently
        // dropped: that is how a target set goes quietly partial.
        assert!(parse_config(format_config(false, 0, &targets).as_bytes()).is_none());
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
                ["status_fields", input, expect] => {
                    let parsed = parse_status_fields(&decode(input));
                    let actual = parsed
                        .map(|s| {
                            [s.backend, s.kver, s.hooks, s.error]
                                .map(|v| v.map(|n| format!("{n:x}")).unwrap_or("?".into()))
                                .join(",")
                        })
                        .unwrap_or("INVALID".into());
                    assert_eq!(actual, *expect, "status fields: {input}");
                }
                ["clamp", full, outlen, expect] => run_clamp(full, outlen, expect),
                _ => panic!("unrecognised vector: {line}"),
            }
            count += 1;
        }
        assert!(count >= 30, "expected the full vector set, ran {count}");
    }
}
