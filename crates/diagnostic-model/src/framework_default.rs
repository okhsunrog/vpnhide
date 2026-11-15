//! Scoped getActiveNetwork evidence. The platform reply before fixture injection
//! and the worker's ordinary reply outside the fixture are independent controls.
use serde::{Deserialize, Serialize};

use crate::{
    ProbeError, Sample,
    framework::{FrameworkControlFailure as Failure, FrameworkPhaseEvidence},
    framework_legacy::LegacyCallWindow,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveNetworkSource {
    pub calls: u32,
    pub started_boot_ns: u64,
    pub original_handle: Option<u64>,
    pub replaced: bool,
}

impl ActiveNetworkSource {
    pub(crate) fn bounded(&self) -> bool {
        self.calls <= 2 && self.original_handle != Some(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveNetworkObservation {
    pub window: Option<LegacyCallWindow>,
    pub handle: Sample<Option<u64>>,
    pub transport_vpn: Sample<bool>,
}

impl ActiveNetworkObservation {
    pub(crate) fn bounded(&self) -> bool {
        self.window.as_ref().is_none_or(LegacyCallWindow::bounded)
            && self.handle.value() != Some(&Some(0))
    }

    pub fn unavailable() -> Self {
        Self {
            window: None,
            handle: Sample::Error(ProbeError::Unsupported),
            transport_vpn: Sample::Error(ProbeError::Unsupported),
        }
    }
}

fn same_call(phase: &FrameworkPhaseEvidence) -> bool {
    phase
        .observation
        .active_network
        .window
        .as_ref()
        .is_some_and(|window| window.contains(phase.acknowledgement.active_network.started_boot_ns))
}

fn source_handle(phase: &FrameworkPhaseEvidence) -> Option<u64> {
    let ack = &phase.acknowledgement;
    let handle = ack.active_network.original_handle?;
    (ack.active_network.calls > 0
        && same_call(phase)
        && handle != ack.sources.vpn_handle
        && handle != ack.sources.ordinary_handle)
        .then_some(handle)
}

fn handle(phase: &FrameworkPhaseEvidence) -> Option<u64> {
    phase
        .observation
        .active_network
        .handle
        .value()
        .copied()
        .flatten()
}

fn is_vpn(phase: &FrameworkPhaseEvidence) -> Option<bool> {
    phase
        .observation
        .active_network
        .transport_vpn
        .value()
        .copied()
}

pub(crate) fn leaked(raw: &FrameworkPhaseEvidence, protected: &FrameworkPhaseEvidence) -> bool {
    let issued = Some(raw.acknowledgement.sources.vpn_handle);
    source_handle(raw).is_some()
        && source_handle(protected).is_some()
        && handle(raw) == issued
        && handle(protected) == issued
}

pub(crate) fn evaluate(phases: [&FrameworkPhaseEvidence; 4]) -> Result<(), Failure> {
    let [prepared, raw, protected, revoked] = phases;
    let issued = raw.acknowledgement.sources.vpn_handle;
    let quiet = |phase: &FrameworkPhaseEvidence| {
        let source = &phase.acknowledgement.active_network;
        source.calls == 1
            && same_call(phase)
            && source.original_handle.is_none()
            && !source.replaced
            && is_vpn(phase) == Some(false)
    };
    if !quiet(prepared) || !quiet(revoked) {
        return Err(Failure::MissingNegativeControl);
    }
    let original = source_handle(raw).ok_or(Failure::MissingPositiveControl)?;
    if [raw, protected]
        .iter()
        .any(|phase| phase.acknowledgement.active_network.calls != 1)
        || raw.acknowledgement.active_network.replaced
        || handle(raw) != Some(issued)
        || is_vpn(raw) != Some(true)
        || source_handle(protected).is_none()
    {
        return Err(Failure::MissingPositiveControl);
    }
    // The platform may route this UID through a real VPN. The two quiet calls
    // observe the normal filtered reply, which can have a different handle.
    let expected = handle(prepared)
        .filter(|&handle| handle != issued && handle != raw.acknowledgement.sources.ordinary_handle)
        .ok_or(Failure::MissingNegativeControl)?;
    if handle(revoked) != Some(expected) || source_handle(protected) != Some(original) {
        return Err(Failure::OrdinaryObjectsChanged);
    }
    if !protected.acknowledgement.active_network.replaced {
        return Err(Failure::ProductionPathNotObserved);
    }
    if handle(protected) != Some(expected) || is_vpn(protected) != Some(false) {
        return Err(Failure::ProbeUnavailable);
    }
    Ok(())
}
