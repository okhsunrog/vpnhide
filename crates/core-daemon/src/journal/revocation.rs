//! Independent, retained authorization revocation evidence. Removing a file is
//! never evidence that injected hooks or processes were unloaded.
use super::{Journal, ZygiskAuthorityRecord};
use serde::{Deserialize, Serialize};
use vpnhide_core_rpc::OperationId;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RevocationState {
    Running,
    Confirmed,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ZygiskRevocation {
    pub operation: Option<OperationId>,
    pub current_boot: String,
    pub authority: ZygiskAuthorityRecord,
    pub state: RevocationState,
}
impl ZygiskRevocation {
    pub fn uncertain(&self) -> bool {
        self.state != RevocationState::Confirmed
    }
    pub(super) fn valid(&self, issued: u64) -> bool {
        !self.current_boot.is_empty()
            && self.current_boot.len() <= 128
            && self.authority.reference().is_some()
            && self.authority.operation.0 <= issued
            && self.operation.is_none_or(|op| op.0 > 0 && op.0 <= issued)
    }
}
impl Journal {
    pub fn revocation_uncertain(&self) -> bool {
        self.zygisk_revocation
            .as_ref()
            .is_some_and(ZygiskRevocation::uncertain)
    }
}

impl ZygiskRevocation {
    pub fn view(&self) -> Option<vpnhide_core_rpc::ZygiskRevocationStatus> {
        use vpnhide_core_rpc::ZygiskRevocationState as S;
        Some(vpnhide_core_rpc::ZygiskRevocationStatus {
            operation: self.operation.map(|op| op.0),
            current_boot: self.current_boot.clone(),
            authorization: self.authority.reference()?,
            state: match self.state {
                RevocationState::Running => S::Running,
                RevocationState::Confirmed => S::Confirmed,
                RevocationState::Unknown => S::Unknown,
            },
        })
    }
}
