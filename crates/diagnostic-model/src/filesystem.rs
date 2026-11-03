//! Scoped path-existence APIs, separate from directory enumeration.
use crate::{ControlFailure, NativeFixtureSpec, NativeReadActor, Sample, SysInterfaceSurface};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilesystemVector {
    Stat,
    Access,
    OpenDirectory,
}
impl FilesystemVector {
    pub const ALL: [Self; 3] = [Self::Stat, Self::Access, Self::OpenDirectory];
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilesystemCheck {
    pub actor: NativeReadActor,
    pub surface: SysInterfaceSurface,
    pub vector: FilesystemVector,
}
impl FilesystemCheck {
    pub fn required() -> Vec<Self> {
        NativeReadActor::ALL
            .into_iter()
            .flat_map(|actor| {
                SysInterfaceSurface::ALL
                    .into_iter()
                    .flat_map(move |surface| {
                        FilesystemVector::ALL.into_iter().map(move |vector| Self {
                            actor,
                            surface,
                            vector,
                        })
                    })
            })
            .collect()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilesystemObservation {
    pub check: FilesystemCheck,
    /// Successful path lookup is visible. Only a confirmed ENOENT is absence;
    /// permission failures and unsupported syscalls retain their own error.
    pub fixture: Sample<bool>,
    pub ordinary: Sample<bool>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilesystemMeasurement {
    pub entries: Vec<FilesystemObservation>,
}
impl FilesystemMeasurement {
    pub fn bounded(&self) -> bool {
        let mut seen = BTreeSet::new();
        self.entries.len() <= FilesystemCheck::required().len()
            && self.entries.iter().all(|e| seen.insert(e.check))
    }
    fn get(&self, check: FilesystemCheck) -> Option<&FilesystemObservation> {
        self.entries.iter().find(|e| e.check == check)
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopedFilesystemResult {
    pub proven: Vec<FilesystemCheck>,
    pub leaked: Vec<FilesystemCheck>,
    pub controls: Vec<ControlFailure>,
}
impl ScopedFilesystemResult {
    pub fn complete(&self) -> bool {
        self.controls.is_empty()
            && self.leaked.is_empty()
            && FilesystemCheck::required()
                .iter()
                .all(|c| self.proven.contains(c))
    }
    fn problem(&mut self, failure: ControlFailure) {
        if !self.controls.contains(&failure) {
            self.controls.push(failure);
        }
    }
}
fn values(observation: Option<&FilesystemObservation>) -> Option<(bool, bool)> {
    let observation = observation?;
    Some((
        *observation.fixture.value()?,
        *observation.ordinary.value()?,
    ))
}
pub fn evaluate_filesystem_fixture(
    spec: &NativeFixtureSpec,
    samples: [Option<&FilesystemMeasurement>; 5],
) -> ScopedFilesystemResult {
    let mut result = ScopedFilesystemResult::default();
    if !spec.valid() || samples[..3].iter().flatten().any(|m| !m.bounded()) {
        result.problem(ControlFailure::InvalidSpec);
        return result;
    }
    if samples[3..].iter().flatten().any(|m| !m.bounded()) {
        result.problem(ControlFailure::InvalidSpec);
    }
    for check in FilesystemCheck::required() {
        let [n, s, w, e, c] = samples.map(|m| m.filter(|m| m.bounded()).and_then(|m| m.get(check)));
        let (Some(n), Some(s), Some(w)) = (values(n), values(s), w) else {
            result.problem(ControlFailure::ProbeUnavailable);
            continue;
        };
        if n != (false, true) {
            result.problem(ControlFailure::MissingNegativeControl);
            continue;
        }
        if s != (true, true) {
            result.problem(ControlFailure::MissingPositiveControl);
            continue;
        }
        let leaked = w.fixture.value() == Some(&true);
        if leaked {
            result.leaked.push(check);
        }
        let mut controlled = true;
        if w.fixture.value().is_none() || w.ordinary.value().is_none() {
            result.problem(ControlFailure::ProbeUnavailable);
            controlled = false;
        }
        if w.ordinary.value() == Some(&false) {
            result.problem(ControlFailure::OrdinaryObjectsChanged);
            controlled = false;
        }
        for failure in
            crate::later_control_failures(values(e), ControlFailure::FixtureChanged, |v| {
                *v == (true, true)
            })
            .chain(crate::later_control_failures(
                values(c),
                ControlFailure::CleanupUnconfirmed,
                |v| *v == (false, true),
            ))
        {
            result.problem(failure);
            controlled = false;
        }
        if controlled && !leaked {
            result.proven.push(check);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProbeError, evaluate_paired_native_fixture};
    fn sample(present: bool) -> FilesystemMeasurement {
        FilesystemMeasurement {
            entries: FilesystemCheck::required()
                .into_iter()
                .map(|check| FilesystemObservation {
                    check,
                    fixture: Sample::Value(present),
                    ordinary: Sample::Value(true),
                })
                .collect(),
        }
    }
    #[test]
    fn all_eighteen_actor_surface_operations_require_independent_controls() {
        let (spec, _, _) = crate::native_pair::tests::fixture();
        let n = sample(false);
        let s = sample(true);
        let result =
            evaluate_filesystem_fixture(&spec, [Some(&n), Some(&s), Some(&n), Some(&s), Some(&n)]);
        assert!(result.complete());
        assert_eq!(result.proven.len(), 18);
        for i in 0..18 {
            let mut w = n.clone();
            w.entries.remove(i);
            assert!(
                !evaluate_filesystem_fixture(
                    &spec,
                    [Some(&n), Some(&s), Some(&w), Some(&s), Some(&n)]
                )
                .complete()
            );
            let mut w = n.clone();
            w.entries[i].ordinary = Sample::Value(false);
            let failed = evaluate_filesystem_fixture(
                &spec,
                [Some(&n), Some(&s), Some(&w), Some(&s), Some(&n)],
            );
            assert!(
                failed
                    .controls
                    .contains(&ControlFailure::OrdinaryObjectsChanged)
            );
            assert!(!failed.complete());
        }
    }
    #[test]
    fn denial_unsupported_or_missing_stimulus_is_neither_hiding_nor_qualified_leak() {
        let (spec, _, _) = crate::native_pair::tests::fixture();
        let n = sample(false);
        let s = sample(true);
        for errno in [1, 13, 38] {
            let mut denied = n.clone();
            denied
                .entries
                .iter_mut()
                .for_each(|e| e.fixture = Sample::Error(ProbeError::Errno(errno)));
            let result = evaluate_filesystem_fixture(
                &spec,
                [Some(&n), Some(&s), Some(&denied), Some(&s), Some(&n)],
            );
            assert!(result.proven.is_empty() && result.leaked.is_empty());
            assert!(result.controls.contains(&ControlFailure::ProbeUnavailable));
            let result = evaluate_filesystem_fixture(
                &spec,
                [Some(&n), Some(&denied), Some(&s), Some(&s), Some(&n)],
            );
            assert!(result.proven.is_empty() && result.leaked.is_empty());
        }
        let result =
            evaluate_filesystem_fixture(&spec, [Some(&n), Some(&n), Some(&s), Some(&s), Some(&n)]);
        assert!(result.leaked.is_empty());
        assert!(
            result
                .controls
                .contains(&ControlFailure::MissingPositiveControl)
        );
        let mut duplicate = s.clone();
        duplicate.entries.push(duplicate.entries[0].clone());
        assert!(!duplicate.bounded());
    }
    #[test]
    fn qualified_existence_survives_failed_ordinary_and_late_controls() {
        let (spec, _, _) = crate::native_pair::tests::fixture();
        let n = sample(false);
        let s = sample(true);
        let mut w = n.clone();
        w.entries[1].fixture = Sample::Value(true);
        w.entries[1].ordinary = Sample::Error(ProbeError::Errno(13));
        for (e, c, reason) in [
            (None, Some(&n), ControlFailure::FixtureChanged),
            (Some(&s), None, ControlFailure::CleanupUnconfirmed),
            (Some(&s), Some(&s), ControlFailure::CleanupUnconfirmed),
        ] {
            let result = evaluate_filesystem_fixture(&spec, [Some(&n), Some(&s), Some(&w), e, c]);
            assert_eq!(result.leaked, [w.entries[1].check]);
            assert!(result.controls.contains(&reason));
            assert!(!result.complete());
        }
    }
    #[test]
    fn root_only_data_cannot_replace_same_worker_raw_path_visibility() {
        let (spec, mut n, mut s) = crate::native_pair::tests::fixture();
        n.filesystem = Some(sample(false));
        s.filesystem = Some(sample(true));
        for raw in [None, Some(&n)] {
            let result = evaluate_paired_native_fixture(&spec, [&n, &s, &n, &s, &n], raw);
            assert!(result.filesystem.proven.is_empty());
            let result = evaluate_paired_native_fixture(&spec, [&n, &s, &s, &s, &n], raw);
            assert_eq!(result.filesystem.leaked.len(), 18);
        }
        assert!(
            evaluate_paired_native_fixture(&spec, [&n, &s, &n, &s, &n], Some(&s))
                .filesystem
                .complete()
        );
    }
}
