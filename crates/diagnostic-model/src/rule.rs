use crate::RouteFixtureSpec;
use serde::{Deserialize, Serialize};

pub const DIAGNOSTIC_RULE_ACTION: u8 = 3; // FR_ACT_NOP
pub const DIAGNOSTIC_RULE_PROTOCOL: u8 = 242;
const DIAGNOSTIC_RULE_TABLE: u32 = 29001;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedRule {
    pub priority: Option<u32>,
    pub goto: Option<u32>,
    pub output_name: Option<String>,
    pub uid_range: Option<[u32; 2]>,
    pub table: u32,
    pub protocol: Option<u8>,
    pub action: u8,
    pub flags: u32,
    pub additional_selectors: bool,
}

impl ObservedRule {
    pub(crate) fn valid(&self) -> bool {
        self.uid_range.is_none_or(|[lo, hi]| lo <= hi)
            && self.output_name.as_ref().is_none_or(|name| {
                !name.is_empty() && name.len() <= 15 && name.bytes().all(|b| b.is_ascii_graphic())
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleRole {
    Vpn,
    Ordinary,
    WorkerUid,
}

impl RuleRole {
    pub const ALL: [Self; 3] = [Self::Vpn, Self::Ordinary, Self::WorkerUid];

    pub fn priority(self, spec: &RouteFixtureSpec) -> u32 {
        // The filled slot is checked in both kernel dumps before creation.
        0x4000_0000 | (((spec.session as u32) & 0x0fff_ffff) << 2) | self as u32
    }

    pub fn output(self, spec: &RouteFixtureSpec) -> Option<&str> {
        match self {
            Self::Vpn => Some(&spec.vpn.name),
            Self::Ordinary => Some(&spec.ordinary.name),
            Self::WorkerUid => None,
        }
    }

    pub fn uid(self, spec: &RouteFixtureSpec) -> Option<u32> {
        (self == Self::WorkerUid).then_some(spec.rule_uid).flatten()
    }

    pub fn table(self) -> u32 {
        if self == Self::WorkerUid {
            DIAGNOSTIC_RULE_TABLE
        } else {
            0
        }
    }

    /// A partial rewrite can still reveal an issued test rule.
    pub fn exposes(self, spec: &RouteFixtureSpec, row: &ObservedRule) -> bool {
        row.priority == Some(self.priority(spec))
            || self
                .output(spec)
                .is_some_and(|name| row.output_name.as_deref() == Some(name))
            || self
                .uid(spec)
                .is_some_and(|uid| row.table == self.table() && row.uid_range == Some([uid; 2]))
    }

    /// Independent readback binds all selectors the fixture generated. An
    /// OIF can become detached if Core already lost its TUN descriptors.
    pub fn matches(self, spec: &RouteFixtureSpec, row: &ObservedRule, detached: bool) -> bool {
        row.priority == Some(self.priority(spec))
            && row.output_name.as_deref() == self.output(spec)
            && row.uid_range == self.uid(spec).map(|uid| [uid; 2])
            && row.table == self.table()
            && row.action == DIAGNOSTIC_RULE_ACTION
            && row.protocol == Some(DIAGNOSTIC_RULE_PROTOCOL)
            && row.goto.is_none()
            && !row.additional_selectors
            && (row.flags == 0 || (detached && self != Self::WorkerUid && row.flags == 0x10))
    }
}
