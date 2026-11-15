use serde::{Deserialize, Serialize};

use crate::framework::{FrameworkControlFailure as Failure, FrameworkPhaseEvidence};

pub const PACKAGE_VISIBILITY_TARGET: &str = "com.android.settings";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageQuery {
    PackageInfo,
    ApplicationInfo,
    InstalledPackages,
    InstalledApplications,
}

impl PackageQuery {
    const ALL: [Self; 4] = [
        Self::PackageInfo,
        Self::ApplicationInfo,
        Self::InstalledPackages,
        Self::InstalledApplications,
    ];

    fn enumerates(self) -> bool {
        matches!(self, Self::InstalledPackages | Self::InstalledApplications)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageSource {
    pub query: PackageQuery,
    pub filtered: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageObservation {
    pub query: PackageQuery,
    pub target_present: bool,
    pub ordinary_present: bool,
    pub remaining_sha256: Option<String>,
}

fn unique<T>(items: &[T], query: impl Fn(&T) -> PackageQuery) -> bool {
    items.len() <= PackageQuery::ALL.len()
        && PackageQuery::ALL
            .into_iter()
            .all(|kind| items.iter().filter(|item| query(item) == kind).count() <= 1)
}

pub(crate) fn sources_bounded(items: &[PackageSource]) -> bool {
    unique(items, |item| item.query)
}

pub(crate) fn observations_bounded(items: &[PackageObservation]) -> bool {
    unique(items, |item| item.query)
        && items.iter().all(|item| {
            item.remaining_sha256.is_some() == item.query.enumerates()
                && item.remaining_sha256.as_ref().is_none_or(|hash| {
                    hash.len() == 64
                        && hash
                            .bytes()
                            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                })
        })
}

fn source(phase: &FrameworkPhaseEvidence, query: PackageQuery) -> Option<&PackageSource> {
    phase
        .acknowledgement
        .package_visibility
        .iter()
        .find(|e| e.query == query)
}

fn sample(phase: &FrameworkPhaseEvidence, query: PackageQuery) -> Option<&PackageObservation> {
    phase
        .observation
        .package_visibility
        .iter()
        .find(|e| e.query == query)
}

fn visible(phase: &FrameworkPhaseEvidence, query: PackageQuery) -> Option<&PackageObservation> {
    sample(phase, query)
        .filter(|value| value.target_present && source(phase, query).is_some_and(|e| !e.filtered))
}

pub(crate) fn leaked(raw: &FrameworkPhaseEvidence, protected: &FrameworkPhaseEvidence) -> bool {
    raw.envelope.fixture.package_visibility.is_some()
        && PackageQuery::ALL.into_iter().any(|query| {
            visible(raw, query).is_some()
                && source(protected, query).is_some()
                && sample(protected, query).is_some_and(|value| value.target_present)
        })
}

pub(crate) fn evaluate(phases: [&FrameworkPhaseEvidence; 4]) -> Result<(), Failure> {
    let [prepared, raw, protected, revoked] = phases;
    for query in PackageQuery::ALL {
        let baseline = visible(raw, query).ok_or(Failure::MissingPositiveControl)?;
        let before = visible(prepared, query).ok_or(Failure::MissingNegativeControl)?;
        let after = visible(revoked, query).ok_or(Failure::MissingNegativeControl)?;
        let filtered = sample(protected, query).ok_or(Failure::ProbeUnavailable)?;
        if [baseline, before, filtered, after]
            .into_iter()
            .any(|value| {
                !value.ordinary_present || value.remaining_sha256 != baseline.remaining_sha256
            })
        {
            return Err(Failure::OrdinaryObjectsChanged);
        }
        if !source(protected, query).is_some_and(|e| e.filtered) {
            return Err(Failure::ProductionPathNotObserved);
        }
        if filtered.target_present {
            return Err(Failure::ProbeUnavailable);
        }
    }
    Ok(())
}
