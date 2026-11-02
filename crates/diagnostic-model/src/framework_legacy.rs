//! Legacy type and enumeration are separate Binder scenarios. Ordinary-object
//! baselines belong to each invocation, not to an assumed static device network.
use crate::{ProbeError, Sample};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyNetworkInfo {
    pub network_type: i32,
    pub state: String,
    pub detailed_state: String,
    pub fixture: bool,
}
impl LegacyNetworkInfo {
    fn bounded(&self) -> bool {
        self.state.len() <= 32
            && self.detailed_state.len() <= 32
            && self
                .state
                .bytes()
                .chain(self.detailed_state.bytes())
                .all(|b| b.is_ascii_uppercase() || b == b'_')
    }
    fn positive(&self) -> bool {
        self.network_type == 17
            && self.fixture
            && self.state == "CONNECTED"
            && self.detailed_state == "CONNECTED"
    }
    fn inactive(&self) -> bool {
        self.network_type == 17
            && !self.fixture
            && self.state == "DISCONNECTED"
            && matches!(self.detailed_state.as_str(), "DISCONNECTED" | "BLOCKED")
    }
    fn exposes_vpn(&self) -> bool {
        self.network_type == 17 && !self.inactive()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyOrdinary {
    pub count: u32,
    pub sha256: String,
    pub fixture_present: bool,
}
impl LegacyOrdinary {
    fn bounded(&self) -> bool {
        self.count <= 256
            && self.sha256.len() == 64
            && self
                .sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyCallWindow {
    pub started_boot_ns: u64,
    pub finished_boot_ns: u64,
}
impl LegacyCallWindow {
    fn bounded(&self) -> bool {
        self.started_boot_ns > 0
            && self.started_boot_ns < self.finished_boot_ns
            && self.finished_boot_ns <= i64::MAX as u64
            && self.finished_boot_ns - self.started_boot_ns <= 25_000_000_000
    }
    fn contains(&self, observed: u64) -> bool {
        self.bounded() && self.started_boot_ns <= observed && observed <= self.finished_boot_ns
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacySourceEvidence {
    pub type_calls: u32,
    pub enumeration_calls: u32,
    pub type_started_boot_ns: u64,
    pub enumeration_started_boot_ns: u64,
    pub type_source: bool,
    pub enumeration_source: bool,
    pub type_normalized: bool,
    pub enumeration_normalized: bool,
    pub enumeration_count: Option<u32>,
    pub ordinary: Option<LegacyOrdinary>,
}
impl LegacySourceEvidence {
    pub fn quiet(&self) -> bool {
        self.type_calls == 1
            && self.enumeration_calls == 1
            && !self.type_source
            && !self.enumeration_source
            && !self.type_normalized
            && !self.enumeration_normalized
            && self.enumeration_count.is_none()
            && self.ordinary.is_none()
    }
    pub fn bounded(&self) -> bool {
        self.type_calls <= 2
            && self.enumeration_calls <= 2
            && self.enumeration_count.is_none_or(|n| n <= 256)
            && self.ordinary.as_ref().is_none_or(LegacyOrdinary::bounded)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyObservation {
    pub type_window: Option<LegacyCallWindow>,
    pub enumeration_window: Option<LegacyCallWindow>,
    pub type_info: Sample<Option<LegacyNetworkInfo>>,
    pub enumeration: Sample<Vec<LegacyNetworkInfo>>,
    pub ordinary: Sample<LegacyOrdinary>,
}
impl LegacyObservation {
    pub fn unavailable() -> Self {
        Self {
            type_window: None,
            enumeration_window: None,
            type_info: Sample::Error(ProbeError::Unsupported),
            enumeration: Sample::Error(ProbeError::Unsupported),
            ordinary: Sample::Error(ProbeError::Unsupported),
        }
    }
    pub fn bounded(&self) -> bool {
        self.type_window
            .as_ref()
            .is_none_or(LegacyCallWindow::bounded)
            && self
                .enumeration_window
                .as_ref()
                .is_none_or(LegacyCallWindow::bounded)
            && self
                .type_info
                .value()
                .is_none_or(|v| v.as_ref().is_none_or(LegacyNetworkInfo::bounded))
            && self
                .enumeration
                .value()
                .is_none_or(|v| v.len() <= 256 && v.iter().all(LegacyNetworkInfo::bounded))
            && self.ordinary.value().is_none_or(LegacyOrdinary::bounded)
    }
    fn type_absent(&self) -> bool {
        self.type_info
            .value()
            .is_some_and(|v| v.as_ref().is_none_or(|v| !v.fixture))
    }
    fn enumeration_absent(&self) -> bool {
        self.enumeration
            .value()
            .is_some_and(|v| v.iter().all(|v| !v.fixture))
            && self.ordinary.value().is_some_and(|v| !v.fixture_present)
    }
}

fn invocation_current(phase: &super::framework::FrameworkPhaseEvidence, type_query: bool) -> bool {
    let (window, observed) = if type_query {
        (
            &phase.observation.legacy.type_window,
            phase.acknowledgement.legacy.type_started_boot_ns,
        )
    } else {
        (
            &phase.observation.legacy.enumeration_window,
            phase.acknowledgement.legacy.enumeration_started_boot_ns,
        )
    };
    window
        .as_ref()
        .is_some_and(|window| window.contains(observed))
}
pub(crate) fn observed_leaks(
    raw: &super::framework::FrameworkPhaseEvidence,
    protected: &super::framework::FrameworkPhaseEvidence,
) -> Vec<super::framework::FrameworkCheck> {
    use super::framework::FrameworkCheck as V;
    let type_positive = invocation_current(raw, true)
        && invocation_current(protected, true)
        && raw.acknowledgement.legacy.type_source
        && protected.acknowledgement.legacy.type_source
        && raw
            .observation
            .legacy
            .type_info
            .value()
            .is_some_and(|v| v.as_ref().is_some_and(LegacyNetworkInfo::positive));
    let enum_positive = invocation_current(raw, false)
        && invocation_current(protected, false)
        && raw.acknowledgement.legacy.enumeration_source
        && protected.acknowledgement.legacy.enumeration_source
        && raw
            .observation
            .legacy
            .enumeration
            .value()
            .is_some_and(|v| v.iter().any(LegacyNetworkInfo::positive));
    let mut leaked = Vec::new();
    // A positive leak is retained independently of negative/preservation/cleanup controls.
    if type_positive
        && protected
            .observation
            .legacy
            .type_info
            .value()
            .is_some_and(|v| v.as_ref().is_some_and(LegacyNetworkInfo::exposes_vpn))
    {
        leaked.push(V::LegacyTypeVpn);
    }
    if enum_positive
        && protected
            .observation
            .legacy
            .enumeration
            .value()
            .is_some_and(|v| v.iter().any(LegacyNetworkInfo::exposes_vpn))
    {
        leaked.push(V::LegacyEnumeration);
    }
    leaked
}

/// Called only after the common exact envelope/process/source checks. Each API
/// still needs its own negative, RAW positive and production normalization path.
pub(crate) fn evaluate_legacy(
    phases: &[super::framework::FrameworkPhaseEvidence],
    cleanup: bool,
) -> super::framework::ScopedFrameworkResult {
    use super::framework::{
        FrameworkCheck as V, FrameworkControlFailure as F, ScopedFrameworkResult,
    };
    let [prepared, raw, protected, revoked] = phases else {
        return ScopedFrameworkResult::default();
    };
    let mut result = ScopedFrameworkResult::default();
    if !cleanup {
        result.controls.push(F::CleanupUnconfirmed);
    }
    let type_positive = invocation_current(raw, true)
        && invocation_current(protected, true)
        && raw.acknowledgement.legacy.type_calls == 1
        && protected.acknowledgement.legacy.type_calls == 1
        && raw.acknowledgement.legacy.type_source
        && protected.acknowledgement.legacy.type_source
        && raw
            .observation
            .legacy
            .type_info
            .value()
            .is_some_and(|v| v.as_ref().is_some_and(LegacyNetworkInfo::positive));
    let enum_positive = invocation_current(raw, false)
        && invocation_current(protected, false)
        && raw.acknowledgement.legacy.enumeration_calls == 1
        && protected.acknowledgement.legacy.enumeration_calls == 1
        && raw.acknowledgement.legacy.enumeration_source
        && protected.acknowledgement.legacy.enumeration_source
        && raw
            .observation
            .legacy
            .enumeration
            .value()
            .is_some_and(|v| v.iter().any(LegacyNetworkInfo::positive));
    result.leaked = observed_leaks(raw, protected);
    let negative_source =
        prepared.acknowledgement.legacy.quiet() && revoked.acknowledgement.legacy.quiet();
    let type_negative = negative_source
        && invocation_current(prepared, true)
        && invocation_current(revoked, true)
        && prepared.observation.legacy.type_absent()
        && revoked.observation.legacy.type_absent();
    let enum_negative = negative_source
        && invocation_current(prepared, false)
        && invocation_current(revoked, false)
        && prepared.observation.legacy.enumeration_absent()
        && revoked.observation.legacy.enumeration_absent();
    let ordinary = [raw, protected].iter().all(|phase| {
        let baseline = &phase.acknowledgement.legacy;
        baseline
            .ordinary
            .as_ref()
            .is_some_and(|v| v.count > 0 && v.fixture_present)
            && phase.observation.legacy.ordinary.value() == baseline.ordinary.as_ref()
            && phase
                .observation
                .legacy
                .enumeration
                .value()
                .is_some_and(|v| Some(v.len() as u32) == baseline.enumeration_count)
    });
    let clean_type = protected
        .observation
        .legacy
        .type_info
        .value()
        .map(|v| v.as_ref().is_some_and(LegacyNetworkInfo::inactive));
    let clean_enum = protected.observation.legacy.enumeration.value().map(|v| {
        v.iter().any(|v| v.network_type == 17)
            && v.iter()
                .filter(|v| v.network_type == 17)
                .all(LegacyNetworkInfo::inactive)
    });
    for (vector, negative, positive, production, clean, preserved) in [
        (
            V::LegacyTypeVpn,
            type_negative,
            type_positive,
            protected.acknowledgement.legacy.type_normalized,
            clean_type,
            true,
        ),
        (
            V::LegacyEnumeration,
            enum_negative,
            enum_positive,
            protected.acknowledgement.legacy.enumeration_normalized,
            clean_enum,
            ordinary,
        ),
    ] {
        let control = if !negative {
            Some(F::MissingNegativeControl)
        } else if !positive {
            Some(F::MissingPositiveControl)
        } else if !preserved {
            Some(F::OrdinaryObjectsChanged)
        } else if clean != Some(true) {
            Some(F::ProbeUnavailable)
        } else if !production {
            Some(F::ProductionPathNotObserved)
        } else {
            None
        };
        if let Some(control) = control {
            result.controls.push(control);
        } else if cleanup {
            result.proven.push(vector);
        }
    }
    result.controls.sort();
    result.controls.dedup();
    result
}
