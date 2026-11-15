use serde::{Deserialize, Serialize};

use crate::ControlFailure;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortsVector {
    Tcp4,
    Udp4,
    Tcp6,
    Udp6,
}

impl PortsVector {
    pub const ALL: [Self; 4] = [Self::Tcp4, Self::Udp4, Self::Tcp6, Self::Udp6];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortsFixtureSpec {
    pub ports: [u16; 4],
    pub challenge: [u8; 16],
}

impl PortsFixtureSpec {
    pub fn valid(&self) -> bool {
        !self.ports.contains(&0) && self.challenge.iter().any(|&byte| byte != 0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortsSample {
    Echo,
    Refused,
    Unavailable,
}

pub type PortsObservation = [PortsSample; 4];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortsWorkerPhase {
    Prepared,
    Protected { before: PortsObservation },
    Revoked { protected: PortsObservation },
    Finished { revoked: PortsObservation },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortsWorkerRequest {
    pub session: u64,
    pub core_epoch: u64,
    pub phase: PortsWorkerPhase,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopedPortsResult {
    pub proven: Vec<PortsVector>,
    pub leaked: Vec<PortsVector>,
    pub controls: Vec<ControlFailure>,
}

impl ScopedPortsResult {
    pub fn complete(&self) -> bool {
        self.proven == PortsVector::ALL && self.leaked.is_empty() && self.controls.is_empty()
    }
}

#[derive(Default)]
pub struct PortsEvidence {
    pub before: Option<PortsObservation>,
    pub root: Option<PortsObservation>,
    pub protected: Option<PortsObservation>,
    pub revoked: Option<PortsObservation>,
    pub rules_confirmed: bool,
    pub rule_matches: [bool; 4],
}

impl PortsEvidence {
    pub fn evaluate(&self, cleanup: bool) -> ScopedPortsResult {
        use PortsSample::*;
        let mut result = ScopedPortsResult::default();
        for (index, vector) in PortsVector::ALL.into_iter().enumerate() {
            let before = self.before.map(|s| s[index]);
            let root = self.root.map(|s| s[index]);
            let protected = self.protected.map(|s| s[index]);
            let revoked = self.revoked.map(|s| s[index]);
            let positive = self.rules_confirmed && before == Some(Echo) && root == Some(Echo);
            if positive && protected == Some(Echo) {
                result.leaked.push(vector);
            }
            // A local UDP REJECT can fail at send(2) without a connection refusal.
            // Only an attributed hit on the exact live REJECT rule substitutes
            // for that socket response.
            let denied = protected == Some(Refused)
                || (matches!(vector, PortsVector::Udp4 | PortsVector::Udp6)
                    && protected == Some(Unavailable));
            let error = if !positive {
                Some(ControlFailure::MissingPositiveControl)
            } else if !cleanup {
                Some(ControlFailure::CleanupUnconfirmed)
            } else if revoked != Some(Echo) {
                Some(ControlFailure::MissingNegativeControl)
            } else if denied && self.rule_matches[index] {
                result.proven.push(vector);
                None
            } else if protected == Some(Echo) {
                None
            } else {
                Some(ControlFailure::ProbeUnavailable)
            };
            if let Some(error) = error
                && !result.controls.contains(&error)
            {
                result.controls.push(error);
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use PortsSample::*;

    #[test]
    fn all_four_vectors_need_live_root_and_worker_controls_and_cleanup() {
        let mut evidence = PortsEvidence {
            before: Some([Echo; 4]),
            root: Some([Echo; 4]),
            protected: Some([Refused; 4]),
            revoked: Some([Echo; 4]),
            rules_confirmed: true,
            rule_matches: [true; 4],
        };
        assert!(evidence.evaluate(true).complete());
        evidence.protected = Some([Refused, Unavailable, Refused, Unavailable]);
        assert!(evidence.evaluate(true).complete());
        evidence.rule_matches[1] = false;
        assert_eq!(
            evidence.evaluate(true).proven,
            [PortsVector::Tcp4, PortsVector::Tcp6, PortsVector::Udp6]
        );
        assert!(evidence.evaluate(false).proven.is_empty());
        evidence.root = Some([Echo, Unavailable, Echo, Echo]);
        evidence.protected = Some([Refused, Echo, Echo, Refused]);
        evidence.revoked = Some([Echo, Echo, Echo, Unavailable]);
        let result = evidence.evaluate(true);
        assert_eq!(result.proven, [PortsVector::Tcp4]);
        assert_eq!(result.leaked, [PortsVector::Tcp6]);
        assert_eq!(evidence.evaluate(false).leaked, [PortsVector::Tcp6]);
        evidence.rules_confirmed = false;
        assert!(evidence.evaluate(true).leaked.is_empty());
    }
}
