//! Proof for Android NetworkCallback delivery to actual worker registrations.
use serde::{Deserialize, Serialize};

use crate::framework::{
    FrameworkControlFailure as Failure, FrameworkPhase, FrameworkPhaseEvidence,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallbackRegistration {
    Listen,
    Default,
    Request,
}

impl CallbackRegistration {
    const ALL: [Self; 3] = [Self::Listen, Self::Default, Self::Request];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallbackSource {
    pub registration: CallbackRegistration,
    pub ordinary_handle: u64,
    pub available_handled: bool,
    pub lost_handled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallbackEvent {
    pub handle: u64,
    pub lost: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallbackObservation {
    pub registration: CallbackRegistration,
    pub events: Vec<CallbackEvent>,
    pub vpn_transport: bool,
    pub vpn_properties: bool,
}

fn unique_registrations<T>(items: &[T], registration: impl Fn(&T) -> CallbackRegistration) -> bool {
    items.len() <= CallbackRegistration::ALL.len()
        && CallbackRegistration::ALL.iter().all(|expected| {
            items
                .iter()
                .filter(|item| registration(item) == *expected)
                .count()
                <= 1
        })
}

pub(crate) fn sources_bounded(items: &[CallbackSource]) -> bool {
    unique_registrations(items, |item| item.registration)
        && items.iter().all(|item| item.ordinary_handle != 0)
}

pub(crate) fn observations_bounded(items: &[CallbackObservation]) -> bool {
    unique_registrations(items, |item| item.registration)
        && items.iter().all(|item| {
            item.events.len() <= 32 && item.events.iter().all(|event| event.handle != 0)
        })
}

fn source(phase: &FrameworkPhaseEvidence, kind: CallbackRegistration) -> Option<&CallbackSource> {
    let identity = &phase.acknowledgement.sources;
    phase
        .acknowledgement
        .callbacks
        .iter()
        .find(|item| item.registration == kind && item.ordinary_handle != identity.vpn_handle)
}

fn observation(
    phase: &FrameworkPhaseEvidence,
    kind: CallbackRegistration,
) -> Option<&CallbackObservation> {
    phase
        .observation
        .callbacks
        .iter()
        .find(|item| item.registration == kind)
}

impl CallbackObservation {
    fn has_delivery(&self) -> bool {
        !self.events.is_empty() || self.vpn_transport || self.vpn_properties
    }

    fn exposes_vpn(&self, vpn: u64) -> bool {
        self.vpn_transport || self.vpn_properties || self.events.iter().any(|e| e.handle == vpn)
    }

    fn observed(&self, sequence: &[CallbackEvent]) -> bool {
        let mut remaining = sequence.iter();
        let mut next = remaining.next();
        for event in &self.events {
            if next == Some(event) {
                next = remaining.next();
            }
        }
        next.is_none()
    }
}

fn available(handle: u64) -> CallbackEvent {
    CallbackEvent {
        handle,
        lost: false,
    }
}

fn lost(handle: u64) -> CallbackEvent {
    CallbackEvent { handle, lost: true }
}

fn raw_control(phase: &FrameworkPhaseEvidence, kind: CallbackRegistration) -> Option<u64> {
    let issued = phase.acknowledgement.sources.vpn_handle;
    let src = source(phase, kind)?;
    let obs = observation(phase, kind)?;
    (!src.available_handled
        && !src.lost_handled
        && obs.vpn_transport
        && obs.vpn_properties
        && obs.observed(&[
            available(issued),
            lost(issued),
            available(src.ordinary_handle),
        ]))
    .then_some(src.ordinary_handle)
}

pub(crate) fn leaked(raw: &FrameworkPhaseEvidence, protected: &FrameworkPhaseEvidence) -> bool {
    let vpn = raw.acknowledgement.sources.vpn_handle;
    CallbackRegistration::ALL.into_iter().any(|kind| {
        raw_control(raw, kind).is_some()
            && source(protected, kind).is_some()
            && observation(protected, kind).is_some_and(|observation| observation.exposes_vpn(vpn))
    })
}

pub(crate) fn evaluate(
    phases: [&FrameworkPhaseEvidence; 4],
) -> Result<(), (FrameworkPhase, Failure)> {
    let [prepared, raw, protected, revoked] = phases;
    let vpn = raw.acknowledgement.sources.vpn_handle;
    for kind in CallbackRegistration::ALL {
        let ordinary =
            raw_control(raw, kind).ok_or((FrameworkPhase::Raw, Failure::MissingPositiveControl))?;
        let protected_failure = |reason| (FrameworkPhase::Protected, reason);
        let src =
            source(protected, kind).ok_or(protected_failure(Failure::MissingPositiveControl))?;
        if src.ordinary_handle != ordinary {
            return Err(protected_failure(Failure::OrdinaryObjectsChanged));
        }
        if !src.available_handled || !src.lost_handled {
            return Err(protected_failure(Failure::ProductionPathNotObserved));
        }
        let obs =
            observation(protected, kind).ok_or(protected_failure(Failure::ProbeUnavailable))?;
        let physical = available(ordinary);
        let ordinary_delivered = match kind {
            CallbackRegistration::Listen => obs.observed(&[physical]),
            // AVAILABLE is rebuilt for the cover. The subsequent VPN LOST does
            // not remove that cover. The platform's duplicate is deduplicated.
            CallbackRegistration::Default | CallbackRegistration::Request => {
                obs.events == [physical]
            }
        };
        if obs.exposes_vpn(vpn) || !ordinary_delivered {
            return Err(protected_failure(Failure::ProbeUnavailable));
        }
        for phase in [prepared, revoked] {
            let failure = |reason| (phase.envelope.phase, reason);
            if !phase.acknowledgement.callbacks.is_empty() {
                return Err(failure(Failure::MissingNegativeControl));
            }
            let Some(obs) = observation(phase, kind).filter(|obs| obs.has_delivery()) else {
                return Err(failure(Failure::ProbeUnavailable));
            };
            if obs.exposes_vpn(vpn) || !obs.observed(&[available(ordinary)]) {
                return Err(failure(Failure::MissingNegativeControl));
            }
        }
    }
    Ok(())
}
