//! Retained empty-worker authority, never a user-policy delivery receipt. A
//! publication intent is synced before writing; restart never republishes it.
use super::{Journal, ZygiskAuthorityRecord};
use crate::native_boot::prepared::BundleIdentity;
use serde::{Deserialize, Serialize};
use vpnhide_core_rpc::{NativeBackend, PhaseOutcome};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionBootstrap {
    pub binding: BundleIdentity,
    pub authority: ZygiskAuthorityRecord,
    /// Exact earlier grant seen before admission, including a prior boot's
    /// bootstrap. Publication failure cannot erase its recovery identity.
    pub previous_authority: Option<ZygiskAuthorityRecord>,
    pub publication_intent: bool,
    pub outcome: PhaseOutcome,
}
impl ProjectionBootstrap {
    pub fn held(&self) -> bool {
        matches!(self.outcome, PhaseOutcome::Running | PhaseOutcome::Unknown)
    }
    pub(crate) fn fence_restart(&mut self) {
        if self.outcome == PhaseOutcome::Running {
            self.outcome = if self.publication_intent {
                PhaseOutcome::Unknown
            } else {
                PhaseOutcome::FailedKnown
            };
        }
    }
    pub(crate) fn valid(&self, journal: &Journal) -> bool {
        let Some(recipient) = &self.binding.zygisk_recipient else {
            return false;
        };
        let Some(reference) = self.authority.reference() else {
            return false;
        };
        crate::maintenance::zygisk_projection::work::valid_boot(&reference.boot_id)
            && self.binding.valid_recipient()
            && self.binding.wire == vpnhide_protocol::format_config(false, 0, &[])
            && self.authority.intent.wire == self.binding.wire
            && self.authority.intent.backend == NativeBackend::Zygisk
            && self.authority.intent.zygisk_module.as_deref() == Some(recipient.path.as_path())
            && (reference.directory_device, reference.directory_inode) == recipient.image.directory
            && self.authority.operation.0 > recipient.image.descriptor.operation
            && self.authority.operation.0 <= journal.issued_through
            && journal.zygisk_projection.as_ref().is_some_and(|work| {
                work.outcome == PhaseOutcome::Confirmed
                    && work.descriptor == recipient.image.descriptor
            })
            && self.previous_authority.as_ref().is_none_or(|previous| {
                previous.reference().is_some_and(|prior| {
                    previous.operation.0 < self.authority.operation.0
                        && previous.intent.zygisk_module == self.authority.intent.zygisk_module
                        && (prior.directory_device, prior.directory_inode)
                            == recipient.image.directory
                })
            })
            && match self.outcome {
                PhaseOutcome::Running | PhaseOutcome::FailedKnown => true,
                PhaseOutcome::Confirmed | PhaseOutcome::Unknown => self.publication_intent,
                _ => false,
            }
    }
}
impl Journal {
    pub fn bootstrap_uncertain(&self) -> bool {
        self.zygisk_bootstrap
            .as_ref()
            .is_some_and(ProjectionBootstrap::held)
    }
}
