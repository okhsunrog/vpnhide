//! Private Core → bundled Zygisk authorization envelope. This is not kernel
//! control-v2: the contained policy remains the unchanged control-v2 wire.
//!
//! A single atomically published file is the authority for new process launches.
//! Generation is durable operation identity, not wall time or daemon PID. Reading
//! an envelope does not revoke authorization already consumed by another process.

use crate::{Config, parse_config};

pub const FILE_NAME: &str = "zygisk-activation.txt";
pub const MAX_BYTES: usize = 64 * 1024;
const HEADER: &str = "vpnhide zygisk-activation 1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authorization {
    pub boot_id: String,
    pub generation: u64,
    pub wire: String,
}

fn valid_boot(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, c)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                c == b'-'
            } else {
                c.is_ascii_digit() || (b'a'..=b'f').contains(&c)
            }
        })
}

impl Authorization {
    pub fn encode(&self) -> Option<String> {
        if !valid_boot(&self.boot_id)
            || self.generation == 0
            || parse_config(self.wire.as_bytes()).is_none()
        {
            return None;
        }
        let value = format!(
            "{HEADER}\nboot {}\ngeneration {}\n{}",
            self.boot_id, self.generation, self.wire
        );
        (value.len() <= MAX_BYTES).then_some(value)
    }

    pub fn decode(bytes: &[u8], current_boot: &str) -> Option<Self> {
        if bytes.len() > MAX_BYTES || !valid_boot(current_boot) {
            return None;
        }
        let text = std::str::from_utf8(bytes).ok()?;
        let mut lines = text.splitn(4, '\n');
        if lines.next()? != HEADER {
            return None;
        }
        let boot = lines.next()?.strip_prefix("boot ")?;
        if boot != current_boot {
            return None;
        }
        let raw_generation = lines.next()?.strip_prefix("generation ")?;
        let generation = raw_generation.parse::<u64>().ok()?;
        if generation == 0 || generation.to_string() != raw_generation {
            return None;
        }
        let wire = lines.next()?;
        parse_config(wire.as_bytes())?;
        Some(Self {
            boot_id: boot.into(),
            generation,
            wire: wire.into(),
        })
    }

    /// Refuse unsupported policy rather than silently masking/truncating it.
    pub fn policy(&self, allowed_hooks: u32) -> Option<Config> {
        let config = parse_config(self.wire.as_bytes())?;
        if config.default_mask != 0
            || config
                .targets
                .iter()
                .any(|t| t.hookmask & !allowed_hooks != 0)
        {
            return None;
        }
        Some(config)
    }

    /// Durable receipt reconciliation must match the exact generation and bytes.
    pub fn matches(&self, generation: u64, wire: &str) -> bool {
        self.generation == generation && self.wire == wire
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const BOOT: &str = "12345678-1234-1234-1234-123456789abc";
    const WIRE: &str = "vpnhide 2 config\ndebug 0\ndefault 0\ntargets 1 2800\nend 1\n";
    fn auth() -> Authorization {
        Authorization {
            boot_id: BOOT.into(),
            generation: 42,
            wire: WIRE.into(),
        }
    }
    #[test]
    fn boot_generation_and_exact_policy_are_distinct_guards() {
        let bytes = auth().encode().unwrap();
        let parsed = Authorization::decode(bytes.as_bytes(), BOOT).unwrap();
        assert_eq!(parsed, auth());
        assert!(parsed.matches(42, WIRE));
        assert!(!parsed.matches(43, WIRE));
        assert!(!parsed.matches(42, &WIRE.replace("debug 0", "debug 1")));
        assert!(
            Authorization::decode(bytes.as_bytes(), "87654321-1234-1234-1234-123456789abc")
                .is_none()
        );
        assert!(Authorization::decode(WIRE.as_bytes(), BOOT).is_none());
    }
    #[test]
    fn unsupported_policy_cannot_be_truncated_into_eligibility() {
        assert!(auth().policy(1).is_some());
        assert!(auth().policy(2).is_none());
        let mut auth = auth();
        auth.wire = WIRE.replace("default 0", "default 1");
        assert!(auth.policy(u32::MAX).is_none());
    }
    #[test]
    fn malformed_and_oversized_authorization_fail_closed() {
        let encoded = auth().encode().unwrap();
        for bytes in [
            encoded.replace("generation 42", "generation 0"),
            encoded.replace("generation 42", "generation 042"),
            encoded.replace("generation 42", "generation 18446744073709551616"),
            encoded.replace("generation 42", "generation 42\ngeneration 43"),
            encoded.replace("activation 1", "activation 2"),
            "x".repeat(MAX_BYTES + 1),
        ] {
            assert!(Authorization::decode(bytes.as_bytes(), BOOT).is_none());
        }
        assert!(Authorization::decode(&[0xff], BOOT).is_none());
        let mut auth = auth();
        auth.boot_id = "not-a-boot".into();
        assert!(auth.encode().is_none());
    }
}
