//! Shared diagnostic RAW messages on the native backend control transports.
//! Parsing a frame does not authenticate its provider or authorize a grant.
use core::fmt;

pub const VERSION: u32 = 3;
pub const MAX_LEASE_MS: u32 = 5_000;
pub const MAX_TOTAL_MS: u32 = 30_000;
pub const MAX_COMMAND_BYTES: usize = 128;
pub const MAX_REPLY_BYTES: usize = 1_024;
pub const NODE: &str = "/proc/vpnhide_ctl";
pub const LEGACY_NODE: &str = "/proc/vpnhide_diag_raw";
pub const REGISTRATION_MARKER: &str = "VHRAW1!";
/// Input-only name for an ordinary SIOCGIFNAME on loopback. The kernel
/// overwrites it with the real name; registration never changes that result.
pub fn registration_name(token: u64) -> Result<[u8; 16], Error> {
    if token == 0 {
        return Err(Error::Malformed);
    }
    let mut bytes = [0; 16];
    bytes[..8].copy_from_slice(&token.to_le_bytes());
    bytes[8..15].copy_from_slice(REGISTRATION_MARKER.as_bytes());
    Ok(bytes)
}
pub const HEADER: &str = "vpnhide 3 raw-state";
pub const PREFIX: &str = "vpnhide 3 raw\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Malformed,
    Unsupported,
    TooLarge,
    Busy,
    Stale,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Released,
    Armed,
    Registered,
    Active,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    pub backend: u32,
    pub max_ms: u32,
    pub phase: Phase,
    pub tid: u32,
    pub uid: u32,
    pub nonce: u64,
    pub remaining_ms: u32,
    pub renewal: u32,
}
fn decimal(s: &str) -> Option<u32> {
    (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()
}
fn nonce(s: &str) -> Option<u64> {
    (s.len() == 16
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()))
    .then(|| u64::from_str_radix(s, 16).ok())
    .flatten()
}
impl Snapshot {
    /// The proc read stream also carries telemetry. An absent block means an old
    /// provider only when the stream includes its stats boundary.
    pub fn from_proc(text: &str) -> Result<Self, Error> {
        let mut found = None;
        for block in text.split_inclusive("vpnhide 1 stats\n").take(1) {
            let mut lines = block.split_inclusive('\n');
            while let Some(line) = lines.next() {
                if line == format!("{HEADER}\n") {
                    let mut frame = String::from(line);
                    for record in lines.by_ref() {
                        if frame.len() + record.len() > MAX_REPLY_BYTES {
                            return Err(Error::TooLarge);
                        }
                        frame.push_str(record);
                        if record == "end\n" {
                            break;
                        }
                        if record.starts_with("vpnhide ") {
                            return Err(Error::Malformed);
                        }
                    }
                    if found.replace(Self::parse(frame.as_bytes())).is_some() {
                        return Err(Error::Malformed);
                    }
                } else if line.starts_with("vpnhide ") && line.trim_end().ends_with(" raw-state") {
                    return Err(Error::Malformed);
                }
            }
        }
        found.unwrap_or_else(|| {
            if text.lines().any(|line| line == "vpnhide 1 stats") {
                Err(Error::Unsupported)
            } else {
                Err(Error::Malformed)
            }
        })
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_REPLY_BYTES {
            return Err(Error::TooLarge);
        }
        let text = std::str::from_utf8(bytes).map_err(|_| Error::Malformed)?;
        let text = text.strip_suffix("end\n").ok_or(Error::Malformed)?;
        if text.contains('\r') {
            return Err(Error::Malformed);
        }
        let mut lines = text.lines();
        if lines.next() != Some(HEADER) {
            return Err(Error::Malformed);
        }
        let backend = lines
            .next()
            .and_then(|line| u32::from_str_radix(line.strip_prefix("backend 0x")?, 16).ok())
            .ok_or(Error::Malformed)?;
        let next = lines.next().ok_or(Error::Malformed)?;
        if let Some(result) = next.strip_prefix("result ") {
            if lines.next().is_some() {
                return Err(Error::Malformed);
            }
            return Err(match result {
                "busy" => Error::Busy,
                "stale" => Error::Stale,
                "unsupported" => Error::Unsupported,
                _ => Error::Malformed,
            });
        }
        let max_ms = next
            .strip_prefix("max_ms ")
            .and_then(decimal)
            .ok_or(Error::Malformed)?;
        let phase = match lines.next() {
            Some("phase released") => Phase::Released,
            Some("phase armed") => Phase::Armed,
            Some("phase registered") => Phase::Registered,
            Some("phase active") => Phase::Active,
            _ => return Err(Error::Malformed),
        };
        let mut scope = lines
            .next()
            .and_then(|line| line.strip_prefix("scope "))
            .ok_or(Error::Malformed)?
            .split(' ');
        let tid = scope.next().and_then(decimal).ok_or(Error::Malformed)?;
        let uid = scope.next().and_then(decimal).ok_or(Error::Malformed)?;
        let nonce = scope.next().and_then(nonce).ok_or(Error::Malformed)?;
        let remaining_ms = scope.next().and_then(decimal).ok_or(Error::Malformed)?;
        let renewal = lines
            .next()
            .and_then(|line| line.strip_prefix("renewal "))
            .and_then(decimal)
            .ok_or(Error::Malformed)?;
        if scope.next().is_some()
            || lines.next().is_some()
            || max_ms == 0
            || max_ms > MAX_LEASE_MS
            || remaining_ms > max_ms
            || tid > i32::MAX as u32
            || (phase != Phase::Active && renewal != 0)
        {
            return Err(Error::Malformed);
        }
        if match phase {
            Phase::Released => tid != 0 || uid != 0 || remaining_ms != 0,
            _ => tid == 0 || uid == 0 || nonce == 0 || remaining_ms == 0,
        } {
            return Err(Error::Malformed);
        }
        Ok(Self {
            backend,
            max_ms,
            phase,
            tid,
            uid,
            nonce,
            remaining_ms,
            renewal,
        })
    }
    pub fn matches(&self, backend: u32, tid: u32, uid: u32, token: u64, phase: Phase) -> bool {
        self.backend == backend
            && self.tid == tid
            && self.uid == uid
            && self.nonce == token
            && self.phase == phase
            && (phase == Phase::Released || self.remaining_ms > 0)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Arm {
        tid: u32,
        uid: u32,
        nonce: u64,
        duration_ms: u32,
        previous: u64,
    },
    Activate {
        nonce: u64,
    },
    Revoke {
        nonce: u64,
    },
    Renew {
        nonce: u64,
        sequence: u32,
    },
    Status,
}

impl Command {
    pub fn encode(self) -> Result<String, Error> {
        let (token, line) = match self {
            Self::Arm {
                tid,
                uid,
                nonce,
                duration_ms,
                previous,
            } => {
                if tid == 0
                    || tid > i32::MAX as u32
                    || uid == 0
                    || duration_ms == 0
                    || duration_ms > MAX_LEASE_MS
                    || previous == nonce
                {
                    return Err(Error::Malformed);
                }
                (
                    nonce,
                    format!("arm {tid} {uid} {nonce:016x} {duration_ms} {previous:016x}"),
                )
            }
            Self::Activate { nonce } => (nonce, format!("activate {nonce:016x}")),
            Self::Revoke { nonce } => (nonce, format!("revoke {nonce:016x}")),
            Self::Renew { nonce, sequence } if sequence != 0 => {
                (nonce, format!("renew {nonce:016x} {sequence}"))
            }
            Self::Status => return Ok(format!("{PREFIX}status\n")),
            _ => return Err(Error::Malformed),
        };
        if token == 0 {
            return Err(Error::Malformed);
        }
        let frame = format!("{PREFIX}{line}\n");
        if frame.len() > MAX_COMMAND_BYTES {
            return Err(Error::TooLarge);
        }
        Ok(frame)
    }
}

pub fn c_header() -> String {
    format!(
        "/* Generated by vpnhide-tooling diagnostic-raw-header; do not edit. */\n#ifndef VPNHIDE_DIAG_RAW_WIRE_H\n#define VPNHIDE_DIAG_RAW_WIRE_H\n#define VPNHIDE_DIAG_RAW_VERSION {VERSION}\n#define VPNHIDE_DIAG_RAW_MAX_MS {MAX_LEASE_MS}U\n#define VPNHIDE_DIAG_RAW_TOTAL_MS {MAX_TOTAL_MS}U\n#define VPNHIDE_DIAG_RAW_COMMAND_BYTES {MAX_COMMAND_BYTES}U\n#define VPNHIDE_DIAG_RAW_REPLY_BYTES {MAX_REPLY_BYTES}U\n#define VPNHIDE_DIAG_RAW_HEADER \"{HEADER}\\n\"\n#define VPNHIDE_DIAG_RAW_REGISTRATION_MARKER \"{REGISTRATION_MARKER}\"\n#define VPNHIDE_DIAG_RAW_PREFIX {PREFIX:?}\n#endif\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACTIVE: &str = "vpnhide 3 raw-state\nbackend 0x4\nmax_ms 5000\nphase active\nscope 42 10001 0000000000000007 40\nrenewal 1\nend\n";

    #[test]
    fn wire_and_proc_framing() {
        assert_eq!(
            Command::Arm {
                tid: 42,
                uid: 10001,
                nonce: 7,
                duration_ms: 5000,
                previous: 0
            }
            .encode()
            .unwrap(),
            "vpnhide 3 raw\narm 42 10001 0000000000000007 5000 0000000000000000\n"
        );
        assert_eq!(
            Command::Renew {
                nonce: 7,
                sequence: 1
            }
            .encode()
            .unwrap(),
            "vpnhide 3 raw\nrenew 0000000000000007 1\n"
        );
        assert_eq!(Snapshot::parse(ACTIVE.as_bytes()).unwrap().renewal, 1);
        let block = format!("vpnhide 1 status\n{ACTIVE}vpnhide 1 stats\n");
        assert_eq!(
            Snapshot::from_proc(&block),
            Snapshot::parse(ACTIVE.as_bytes())
        );
        assert_eq!(
            Snapshot::from_proc(&format!("{ACTIVE}{ACTIVE}")),
            Err(Error::Malformed)
        );
        assert_eq!(
            Snapshot::from_proc("vpnhide 1 status\nvpnhide 1 stats\n"),
            Err(Error::Unsupported)
        );
    }

    #[test]
    fn untrusted_receipts_do_not_authorize_measurement() {
        for text in [
            ACTIVE.replace("3 raw-state", "2 raw-state"),
            ACTIVE.replace("max_ms 5000", "max_ms 5001"),
            ACTIVE.replace("scope 42", "scope 0"),
            ACTIVE.replace(" 40\n", " 0\n"),
            ACTIVE.replace("phase active", "phase released"),
            ACTIVE.replace("renewal 1", "renewal 1\nrenewal 2"),
            ACTIVE.trim_end().to_owned(),
            ACTIVE.replace("\n", "\r\n"),
        ] {
            assert!(Snapshot::parse(text.as_bytes()).is_err(), "{text}");
        }
        assert_eq!(
            Command::Renew {
                nonce: 1,
                sequence: 0
            }
            .encode(),
            Err(Error::Malformed)
        );
        assert_eq!(Command::Revoke { nonce: 0 }.encode(), Err(Error::Malformed));
        let text = "vpnhide 3 raw-state\nbackend 0x4\nmax_ms 5000\nphase released\nscope 0 0 0000000000000007 0\nrenewal 0\nend\n";
        let s = Snapshot::parse(text.as_bytes()).unwrap();
        assert_eq!(s.phase, Phase::Released);
        assert_eq!(s.nonce, 7);
        assert!(!s.matches(4, 42, 10001, 7, Phase::Active));
    }
}
