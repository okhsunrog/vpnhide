//! Bounded completed discovery cycles. A fresh warned publication has its own
//! operation and generation; it cannot overwrite old publication/withdrawal receipts.
use super::*;
use crate::journal::{
    Journal, backend_choice::BackendChoiceRecord, projection_bootstrap::ProjectionBootstrap,
};
use crate::maintenance::replacement::ReplacementWork;
use vpnhide_core_rpc::PhaseOutcome;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetiredProjection {
    pub projection: work::ProjectionWork,
    pub withdrawal: withdrawal::work::WithdrawalWork,
    pub choice: BackendChoiceRecord,
    pub replacement: Option<ReplacementWork>,
    pub followup: Option<withdrawal::followup::WithdrawalFollowup>,
    pub bootstrap: Option<ProjectionBootstrap>,
    pub boot_id: String,
    pub source: ImageFence,
    pub tombstone: withdrawal::WithdrawalImage,
}
impl RetiredProjection {
    fn view(&self, journal: &Journal) -> Journal {
        let mut view = journal.clone();
        view.zygisk_projection = Some(self.projection.clone());
        view.zygisk_withdrawal = Some(self.withdrawal.clone());
        view.backend_choice = Some(self.choice.clone());
        view.replacement = self.replacement.clone();
        view.withdrawal_followup = self.followup.clone();
        view.zygisk_bootstrap = self.bootstrap.clone();
        view.bootstrap_resolution = journal
            .resolved_bootstraps
            .iter()
            .find(|r| Some(&r.observed) == self.bootstrap.as_ref())
            .cloned();
        view
    }
    fn valid(&self, journal: &Journal) -> bool {
        let view = self.view(journal);
        self.projection.outcome == PhaseOutcome::Confirmed
            && self.withdrawal.outcome == PhaseOutcome::Confirmed
            && self.boot_id != self.withdrawal.boot_id
            && work::valid_boot(&self.boot_id)
            && self
                .source
                .same_content(view.withdrawal_source(&self.withdrawal))
            && self.tombstone.valid_for(&self.withdrawal.descriptor)
            && work::validate_journal(&view)
            && withdrawal::work::validate_journal(&view)
            && withdrawal::followup::valid_optional(&view)
            && self
                .bootstrap
                .as_ref()
                .is_none_or(|bootstrap| bootstrap.valid(&view) && !view.bootstrap_uncertain())
            && self.replacement.as_ref().is_none_or(|replacement| {
                !replacement.retained()
                    && crate::maintenance::replacement::valid_journal(&view, replacement)
            })
    }
    pub(crate) fn owns(&self, descriptor: &Descriptor) -> bool {
        &self.projection.descriptor == descriptor
            || self.replacement.as_ref().is_some_and(|replacement| {
                replacement
                    .projection
                    .as_ref()
                    .is_some_and(|p| p.history.contains(descriptor) || &p.descriptor == descriptor)
                    || replacement
                        .withdrawn_source
                        .as_ref()
                        .is_some_and(|p| p.history.contains(descriptor))
            })
    }
    pub(crate) fn retains(&self, operation: vpnhide_core_rpc::OperationId) -> bool {
        self.projection.operation == operation
            || self.withdrawal.operation == operation
            || self.choice.operation == operation
            || self
                .replacement
                .as_ref()
                .is_some_and(|r| r.status.operation == operation)
    }
}

pub(crate) fn valid(journal: &Journal) -> bool {
    (journal.projection_history.is_empty() || journal.version >= 21)
        && journal.projection_history.len() <= 16
        && journal
            .projection_history
            .windows(2)
            .all(|p| p[0].projection.operation.0 < p[1].projection.operation.0)
        && journal
            .projection_history
            .iter()
            .all(|cycle| cycle.valid(journal))
}
pub(crate) fn predecessor_valid(journal: &Journal, work: &work::ProjectionWork) -> bool {
    work.previous_withdrawal.as_ref().is_none_or(|image| {
        journal.version >= 21
            && journal.projection_history.iter().any(|cycle| {
                cycle.withdrawal.operation.0 < work.operation.0
                    && cycle.boot_id == work.boot_id
                    && &cycle.tombstone == image
            })
    })
}
pub(crate) fn retire_for_reentry(
    journal: &mut Journal,
    next: &work::ProjectionWork,
) -> io::Result<()> {
    let withdrawal = journal.zygisk_withdrawal.as_ref().ok_or_else(invalid)?;
    let projection = journal.zygisk_projection.as_ref().ok_or_else(invalid)?;
    let image = next.previous_withdrawal.as_ref().ok_or_else(invalid)?;
    if (journal.bootstrap_resolution.is_some() && journal.resolved_bootstraps.len() >= 32)
        || journal.projection_history.len() >= 16
        || journal.has_unknown()
        || journal.zygisk_exposed_in(&next.boot_id)
        || next.boot_id == withdrawal.boot_id
        || next.operation.0 <= withdrawal.operation.0
        || next.request.expected_revision
            != journal
                .backend_choice
                .as_ref()
                .ok_or_else(invalid)?
                .revision
        || !next
            .source
            .same_content(journal.withdrawal_source(withdrawal))
        || !image.valid_for(&withdrawal.descriptor)
        || !work::validate_journal(journal)
        || !withdrawal::work::validate_journal(journal)
    {
        return Err(invalid());
    }
    let cycle = RetiredProjection {
        projection: projection.clone(),
        withdrawal: withdrawal.clone(),
        choice: journal.backend_choice.clone().ok_or_else(invalid)?,
        replacement: journal.replacement.clone(),
        followup: journal.withdrawal_followup.clone(),
        bootstrap: journal.zygisk_bootstrap.clone(),
        boot_id: next.boot_id.clone(),
        source: next.source.clone(),
        tombstone: image.clone(),
    };
    let mut validation = journal.clone();
    if let Some(resolution) = validation.bootstrap_resolution.take() {
        validation.resolved_bootstraps.push(resolution);
    }
    if !cycle.valid(&validation) {
        return Err(invalid());
    }
    journal.resolved_bootstraps = validation.resolved_bootstraps;
    journal.bootstrap_resolution = None;
    journal.projection_history.push(cycle);
    journal.zygisk_projection = None;
    journal.zygisk_withdrawal = None;
    journal.withdrawal_followup = None;
    journal.zygisk_bootstrap = None;
    journal.replacement = None;
    journal.backend_offer = None;
    Ok(())
}

/// Before-spawn failure has no new discovery effect. Restore the original
/// active cycle, retaining the new operation's FailedKnown receipt. Unknown
/// dispatch outcomes never enter this path or restore an older preference.
pub(crate) fn restore_failed_reentry(journal: &mut Journal) {
    let Some(work) = journal.zygisk_projection.as_ref() else {
        return;
    };
    if work.outcome != PhaseOutcome::FailedKnown || work.previous_withdrawal.is_none() {
        return;
    }
    let Some(cycle) = journal.projection_history.last() else {
        return;
    };
    if work.previous_withdrawal.as_ref() != Some(&cycle.tombstone)
        || cycle.withdrawal.operation.0 >= work.operation.0
    {
        return;
    }
    let cycle = journal.projection_history.pop().expect("checked cycle");
    journal.zygisk_projection = Some(cycle.projection);
    journal.zygisk_withdrawal = Some(cycle.withdrawal);
    journal.backend_choice = Some(cycle.choice);
    journal.replacement = cycle.replacement;
    journal.withdrawal_followup = cycle.followup;
    journal.zygisk_bootstrap = cycle.bootstrap;
    if let Some(at) = journal
        .resolved_bootstraps
        .iter()
        .position(|r| Some(&r.observed) == journal.zygisk_bootstrap.as_ref())
    {
        journal.bootstrap_resolution = Some(journal.resolved_bootstraps.remove(at));
    }
}
