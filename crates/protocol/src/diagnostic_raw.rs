//! Private diagnostic RAW control, separate from config-v2/telemetry-v1.
//! Parsing a frame does not authenticate its provider or authorize a grant.
use core::fmt;

pub const VERSION: u32 = 1;
pub const MAX_LEASE_MS: u32 = 5_000;
pub const MAX_COMMAND_BYTES: usize = 128;
pub const MAX_REPLY_BYTES: usize = 1_024;
pub const NODE: &str = "/proc/vpnhide_diag_raw";
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
pub const NODE_NAME: &str = "vpnhide_diag_raw";
fn header() -> String {
    format!("vpnhide diagnostic-raw {VERSION}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Malformed,
    Unsupported,
    TooLarge,
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
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_REPLY_BYTES {
            return Err(Error::TooLarge);
        }
        let text = std::str::from_utf8(bytes).map_err(|_| Error::Malformed)?;
        if !text.ends_with('\n') || text.contains('\r') {
            return Err(Error::Malformed);
        }
        let mut lines = text.lines();
        if lines.next() != Some(header().as_str()) {
            return Err(Error::Unsupported);
        }
        let (mut backend, mut max_ms, mut phase, mut tid, mut uid, mut token, mut remaining) =
            (None, None, None, None, None, None, None);
        for line in lines {
            let (key, value) = line.split_once(' ').ok_or(Error::Malformed)?;
            match key {
                "backend" if backend.is_none() => {
                    backend = Some(decimal(value).ok_or(Error::Malformed)?)
                }
                "max_ms" if max_ms.is_none() => {
                    max_ms = Some(decimal(value).ok_or(Error::Malformed)?)
                }
                "phase" if phase.is_none() => {
                    phase = Some(match value {
                        "released" => Phase::Released,
                        "armed" => Phase::Armed,
                        "registered" => Phase::Registered,
                        "active" => Phase::Active,
                        _ => return Err(Error::Malformed),
                    })
                }
                "tid" if tid.is_none() => tid = Some(decimal(value).ok_or(Error::Malformed)?),
                "uid" if uid.is_none() => uid = Some(decimal(value).ok_or(Error::Malformed)?),
                "nonce" if token.is_none() => token = Some(nonce(value).ok_or(Error::Malformed)?),
                "remaining_ms" if remaining.is_none() => {
                    remaining = Some(decimal(value).ok_or(Error::Malformed)?)
                }
                _ => return Err(Error::Malformed),
            }
        }
        let s = Self {
            backend: backend.ok_or(Error::Malformed)?,
            max_ms: max_ms.ok_or(Error::Malformed)?,
            phase: phase.ok_or(Error::Malformed)?,
            tid: tid.ok_or(Error::Malformed)?,
            uid: uid.ok_or(Error::Malformed)?,
            nonce: token.ok_or(Error::Malformed)?,
            remaining_ms: remaining.ok_or(Error::Malformed)?,
        };
        if s.max_ms == 0 {
            return Err(Error::Unsupported);
        }
        if s.max_ms > MAX_LEASE_MS || s.remaining_ms > s.max_ms || s.tid > i32::MAX as u32 {
            return Err(Error::Malformed);
        }
        if match s.phase {
            Phase::Released => s.tid != 0 || s.uid != 0 || s.remaining_ms != 0,
            _ => s.tid == 0 || s.uid == 0 || s.nonce == 0 || s.remaining_ms == 0,
        } {
            return Err(Error::Malformed);
        }
        Ok(s)
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
    },
    Activate {
        nonce: u64,
    },
    Revoke {
        nonce: u64,
    },
}
impl Command {
    pub fn encode(self) -> Result<String, Error> {
        let (token, line) = match self {
            Self::Arm {
                tid,
                uid,
                nonce,
                duration_ms,
            } => {
                if tid == 0
                    || tid > i32::MAX as u32
                    || uid == 0
                    || duration_ms == 0
                    || duration_ms > MAX_LEASE_MS
                {
                    return Err(Error::Malformed);
                }
                (
                    nonce,
                    format!("v{VERSION} arm {tid} {uid} {nonce:016x} {duration_ms}\n"),
                )
            }
            Self::Activate { nonce } => (nonce, format!("v{VERSION} activate {nonce:016x}\n")),
            Self::Revoke { nonce } => (nonce, format!("v{VERSION} revoke {nonce:016x}\n")),
        };
        if token == 0 {
            return Err(Error::Malformed);
        }
        if line.len() > MAX_COMMAND_BYTES {
            return Err(Error::TooLarge);
        }
        Ok(line)
    }
}
/// Native adapters include the generated constants; no source-text parsing.
pub fn c_header() -> String {
    let header = header();
    format!(
        "/* Generated by vpnhide-tooling diagnostic-raw-header; do not edit. */\n#ifndef VPNHIDE_DIAG_RAW_WIRE_H\n#define VPNHIDE_DIAG_RAW_WIRE_H\n#define VPNHIDE_DIAG_RAW_VERSION {VERSION}\n#define VPNHIDE_DIAG_RAW_MAX_MS {MAX_LEASE_MS}U\n#define VPNHIDE_DIAG_RAW_COMMAND_BYTES {MAX_COMMAND_BYTES}U\n#define VPNHIDE_DIAG_RAW_REPLY_BYTES {MAX_REPLY_BYTES}U\n#define VPNHIDE_DIAG_RAW_HEADER \"{header}\\n\"\n#define VPNHIDE_DIAG_RAW_REGISTRATION_MARKER \"{REGISTRATION_MARKER}\"\n#define VPNHIDE_DIAG_RAW_NODE_NAME \"{NODE_NAME}\"\n#define VPNHIDE_DIAG_RAW_PREFIX \"v{VERSION} \"\n#endif\n"
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registration_has_explicit_byte_order_and_survives_kernel_name_termination() {
        assert_eq!(
            registration_name(0x0123456789abcdef).unwrap(),
            *b"\xef\xcd\xab\x89\x67\x45\x23\x01VHRAW1!\0"
        );
        assert_eq!(registration_name(0), Err(Error::Malformed));
    }
    #[test]
    fn generated_native_contract_is_current() {
        assert_eq!(
            c_header(),
            include_str!("../../../kmod/generated/diagnostic_raw.h")
        );
        assert_eq!(NODE, format!("/proc/{NODE_NAME}"));
    }
    const ACTIVE: &str = "vpnhide diagnostic-raw 1\nbackend 0\nmax_ms 5000\nphase active\ntid 42\nuid 10001\nnonce 0123456789abcdef\nremaining_ms 40\n";
    #[test]
    fn literal_private_frames_and_commands_do_not_reuse_the_config_wire() {
        let s = Snapshot::parse(ACTIVE.as_bytes()).unwrap();
        assert!(s.matches(0, 42, 10001, 0x0123456789abcdef, Phase::Active));
        assert_eq!(
            Command::Arm {
                tid: 42,
                uid: 10001,
                nonce: 7,
                duration_ms: 5000
            }
            .encode()
            .unwrap(),
            "v1 arm 42 10001 0000000000000007 5000\n"
        );
        for (command, wire) in [
            (
                Command::Activate { nonce: 7 },
                "v1 activate 0000000000000007\n",
            ),
            (Command::Revoke { nonce: 7 }, "v1 revoke 0000000000000007\n"),
        ] {
            assert_eq!(command.encode().unwrap(), wire)
        }
        assert_eq!(Command::Revoke { nonce: 0 }.encode(), Err(Error::Malformed));
    }
    #[test]
    fn unknown_incomplete_duplicate_and_impossible_states_refuse_before_commands() {
        for text in [
            ACTIVE.replace("diagnostic-raw 1", "diagnostic-raw 2"),
            ACTIVE.replace("max_ms 5000", "max_ms 5001"),
            ACTIVE.replace("tid 42", "tid 0"),
            ACTIVE.replace("remaining_ms 40", "remaining_ms 0"),
            ACTIVE.replace("uid 10001", "uid -1"),
            ACTIVE.replace("phase active", "phase released"),
            ACTIVE.replace("nonce 0123456789abcdef", "nonce 0"),
            format!("{ACTIVE}backend 0\n"),
            ACTIVE.replace("remaining_ms 40\n", ""),
            ACTIVE.trim_end().to_owned(),
            ACTIVE.replace("\n", "\r\n"),
        ] {
            assert!(Snapshot::parse(text.as_bytes()).is_err(), "{text}")
        }
        assert_eq!(
            Snapshot::parse(&vec![b'x'; MAX_REPLY_BYTES + 1]),
            Err(Error::TooLarge)
        );
        assert_eq!(
            Command::Arm {
                tid: 42,
                uid: 10001,
                nonce: 7,
                duration_ms: MAX_LEASE_MS + 1
            }
            .encode(),
            Err(Error::Malformed)
        );
    }
    #[test]
    fn released_readback_keeps_nonce_but_never_retains_live_identity() {
        let text = "vpnhide diagnostic-raw 1\nbackend 4\nmax_ms 5000\nphase released\ntid 0\nuid 0\nnonce 0000000000000007\nremaining_ms 0\n";
        let s = Snapshot::parse(text.as_bytes()).unwrap();
        assert_eq!(s.phase, Phase::Released);
        assert_eq!(s.nonce, 7);
        assert!(!s.matches(4, 42, 10001, 7, Phase::Active));
    }
}
