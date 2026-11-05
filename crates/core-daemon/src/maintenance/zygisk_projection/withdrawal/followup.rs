//! A later non-Zygisk preference has its own receipt. The original withdrawal
//! remains immutable; this record never claims that old processes exited.
use super::{WithdrawalImage, observation::WithdrawalBootProof};
use crate::{
    journal::{Journal, backend_choice::BackendChoiceRecord},
    maintenance::image::ImageFence,
};
use serde::{Deserialize, Serialize};
use vpnhide_core_rpc::{BackendChoice, NativeBackend, OperationId, PhaseOutcome};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WithdrawalFollowup {
    pub choice: BackendChoiceRecord,
    pub withdrawal: OperationId,
    pub source: ImageFence,
    pub image: WithdrawalImage,
}

pub(crate) fn valid_optional(journal: &Journal) -> bool {
    journal.withdrawal_followup.is_none() || valid(journal)
}

pub(crate) fn valid(journal: &Journal) -> bool {
    let Some(next) = &journal.withdrawal_followup else {
        return false;
    };
    let Some(work) = &journal.zygisk_withdrawal else {
        return false;
    };
    journal.version >= 21
        && work.outcome == PhaseOutcome::Confirmed
        && next.withdrawal == work.operation
        && next.choice.valid(journal.issued_through)
        && next.choice.operation.0 > work.operation.0
        && next.choice.revision > work.request.expected_revision + 1
        && next.choice.requested_in_boot != work.boot_id
        && next.choice.request.choice
            != (BackendChoice::Fixed {
                backend: NativeBackend::Zygisk,
            })
        && (next.source.same_content(journal.withdrawal_source(work))
            || journal
                .replacement
                .as_ref()
                .and_then(|replacement| replacement.withdrawn_source.as_ref())
                .is_some_and(|source| {
                    source
                        .source_history
                        .iter()
                        .any(|prior| prior.same_content(&next.source))
                }))
        && next.image.valid_for(&work.descriptor)
        && journal.backend_choice.as_ref() == Some(&next.choice)
        && journal.get(next.choice.operation).is_some_and(|entry| {
            entry.command
                == Some(crate::journal::CommandIntent::StageBackendChoice {
                    request: next.choice.request.clone(),
                })
                && entry.record.settled == Some(PhaseOutcome::Confirmed)
                && entry.record.phases.len() == 1
                && entry.record.phases[0].outcome == PhaseOutcome::Confirmed
        })
}

pub(crate) fn require_proof(
    journal: &Journal,
    env: &crate::native_boot::NativeEnv,
) -> Result<WithdrawalBootProof, String> {
    let work = journal
        .zygisk_withdrawal
        .as_ref()
        .ok_or("withdrawal unavailable")?;
    let proof = env
        .withdrawal
        .as_ref()
        .ok_or("backend choice requires new-boot withdrawal proof")?;
    if !proof.valid_for(work)
        || !proof.source.same_content(journal.withdrawal_source(work))
        || journal.zygisk_exposed_in(&proof.boot_id)
    {
        return Err("withdrawal proof does not identify the current safe boot".into());
    }
    proof
        .recheck(env)
        .map_err(|e| format!("withdrawal proof changed: {e}"))?;
    Ok(proof.clone())
}
