//! Core verifies the actual network extras received by the worker's broadcasts.
use serde::{Deserialize, Serialize};

use crate::framework::{FrameworkControlFailure as Failure, FrameworkPhaseEvidence};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingRegistration {
    Listen,
    Request,
}

impl PendingRegistration {
    const ALL: [Self; 2] = [Self::Listen, Self::Request];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingSource {
    pub registration: PendingRegistration,
    pub ordinary_handle: u64,
    pub suppressed: bool,
    pub cover_handle: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingObservation {
    pub registration: PendingRegistration,
    pub handles: Vec<u64>,
    pub malformed: bool,
}

fn unique<T>(items: &[T], registration: impl Fn(&T) -> PendingRegistration) -> bool {
    items.len() <= PendingRegistration::ALL.len()
        && PendingRegistration::ALL.into_iter().all(|kind| {
            items
                .iter()
                .filter(|item| registration(item) == kind)
                .count()
                <= 1
        })
}

pub(crate) fn sources_bounded(items: &[PendingSource]) -> bool {
    unique(items, |item| item.registration)
        && items.iter().all(|item| {
            item.ordinary_handle != 0
                && item.cover_handle != Some(0)
                && !(item.suppressed && item.cover_handle.is_some())
        })
}

pub(crate) fn observations_bounded(items: &[PendingObservation]) -> bool {
    unique(items, |item| item.registration)
        && items
            .iter()
            .all(|item| item.handles.len() <= 4 && !item.handles.contains(&0))
}

fn source(phase: &FrameworkPhaseEvidence, mode: PendingRegistration) -> Option<&PendingSource> {
    let identity = &phase.acknowledgement.sources;
    phase
        .acknowledgement
        .pending_intents
        .iter()
        .find(|item| item.registration == mode && item.ordinary_handle != identity.vpn_handle)
}

fn observation(
    phase: &FrameworkPhaseEvidence,
    mode: PendingRegistration,
) -> Option<&PendingObservation> {
    phase
        .observation
        .pending_intents
        .iter()
        .find(|item| item.registration == mode)
}

fn delivered(phase: &FrameworkPhaseEvidence, mode: PendingRegistration, expected: u64) -> bool {
    observation(phase, mode).is_some_and(|obs| !obs.malformed && obs.handles == [expected])
}

fn raw_control(phase: &FrameworkPhaseEvidence, mode: PendingRegistration) -> Option<u64> {
    let src = source(phase, mode)?;
    (!src.suppressed
        && src.cover_handle.is_none()
        && delivered(phase, mode, phase.acknowledgement.sources.vpn_handle))
    .then_some(src.ordinary_handle)
}

pub(crate) fn leaked(raw: &FrameworkPhaseEvidence, protected: &FrameworkPhaseEvidence) -> bool {
    let vpn = raw.acknowledgement.sources.vpn_handle;
    PendingRegistration::ALL.into_iter().any(|mode| {
        raw_control(raw, mode).is_some()
            && source(protected, mode).is_some()
            && observation(protected, mode).is_some_and(|obs| obs.handles.contains(&vpn))
    })
}

pub(crate) fn evaluate(phases: [&FrameworkPhaseEvidence; 4]) -> Result<(), Failure> {
    let [prepared, raw, protected, revoked] = phases;
    for mode in PendingRegistration::ALL {
        let ordinary = raw_control(raw, mode).ok_or(Failure::MissingPositiveControl)?;
        let src = source(protected, mode).ok_or(Failure::MissingPositiveControl)?;
        if [prepared, revoked].into_iter().any(|phase| {
            !phase.acknowledgement.pending_intents.is_empty() || !delivered(phase, mode, ordinary)
        }) {
            return Err(Failure::MissingNegativeControl);
        }
        if src.ordinary_handle != ordinary {
            return Err(Failure::OrdinaryObjectsChanged);
        }
        let handled = match mode {
            PendingRegistration::Listen => src.suppressed,
            PendingRegistration::Request => src.cover_handle == Some(ordinary),
        };
        if !handled {
            return Err(Failure::ProductionPathNotObserved);
        }
        if !delivered(protected, mode, ordinary) {
            return Err(Failure::ProbeUnavailable);
        }
    }
    Ok(())
}
