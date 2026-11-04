//! Passive KMI evidence. A release hint is never a loader/private-layout proof.
//! Candidate identities come exclusively from the already validated manifest.

use serde::{Deserialize, Serialize};

use super::{Manifest, PayloadKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinuxSeries {
    pub major: u32,
    pub minor: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KmiCandidate {
    pub payload_index: usize,
    pub kmi: String,
    pub matches_release_hint: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KmiResolution {
    pub series: Option<LinuxSeries>,
    pub release_hint: Option<String>,
    pub conflicting_hints: bool,
    /// A quorum of distinct files/names bound to live module build IDs. This is
    /// selection evidence, never private-layout, successful-load or hook proof.
    pub module_hint: Option<String>,
    pub candidates: Vec<KmiCandidate>,
}

/// Require an ordinary leading numeric kernel version. Do not find arbitrary
/// version substrings, normalize signed integers or accept unbounded input.
pub fn series(release: &str) -> Option<LinuxSeries> {
    if release.len() > 256 || release.bytes().any(|b| b.is_ascii_control()) {
        return None;
    }
    let version = release.split('-').next()?;
    let parts: Vec<_> = version.split('.').collect();
    if !(2..=3).contains(&parts.len())
        || parts
            .iter()
            .any(|p| p.is_empty() || p.len() > 5 || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    Some(LinuxSeries {
        major: parts[0].parse().ok()?,
        minor: parts[1].parse().ok()?,
    })
}

fn release_hints(release: &str) -> (Option<LinuxSeries>, Option<String>, bool) {
    let series = series(release);
    let hints: Vec<_> = release
        .split('-')
        .skip(1)
        .filter(|p| {
            p.strip_prefix("android")
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
        .collect();
    let release_hint = match (series, hints.as_slice()) {
        (Some(s), [hint]) => Some(format!("{hint}-{}.{}", s.major, s.minor)),
        _ => None,
    };
    (series, release_hint, hints.len() > 1)
}

/// No effect and no implicit trust elevation. Even one candidate only means
/// catalog uniqueness; actual loading still needs runtime/ownership/health gates.
pub fn resolve(manifest: &Manifest, release: &str) -> std::io::Result<KmiResolution> {
    manifest.validate()?;
    let (series, release_hint, conflicting_hints) = release_hints(release);
    let mut candidates: Vec<_> = manifest
        .payloads
        .iter()
        .enumerate()
        .filter_map(|(payload_index, payload)| {
            let PayloadKind::Kmod { kmi } = &payload.implementation else {
                return None;
            };
            let (_, version) = kmi.split_once('-')?;
            (series.is_some() && self::series(version) == series).then(|| KmiCandidate {
                payload_index,
                kmi: kmi.clone(),
                matches_release_hint: release_hint.as_ref() == Some(kmi),
            })
        })
        .collect();
    // Stable technical presentation only; this ordering never resolves ambiguity.
    candidates.sort_by(|a, b| a.kmi.cmp(&b.kmi));
    Ok(KmiResolution {
        series,
        release_hint,
        conflicting_hints,
        module_hint: None,
        candidates,
    })
}

pub fn resolve_with_modules(
    manifest: &Manifest,
    release: &str,
    observed: &super::standard_modules::Observation,
) -> std::io::Result<KmiResolution> {
    let mut result = resolve(manifest, release)?;
    let (hint, conflicting) = module_hints(
        release,
        !observed.incomplete,
        observed.modules.iter().map(|m| {
            (
                m.metadata.name.as_str(),
                m.sha256.as_str(),
                m.metadata.vermagic.as_str(),
            )
        }),
    );
    result.conflicting_hints |= conflicting;
    if let Some(hint) = hint {
        result.candidates.retain(|candidate| candidate.kmi == hint);
        result.module_hint = Some(hint);
    }
    Ok(result)
}

fn module_hints<'a>(
    release: &str,
    complete: bool,
    modules: impl Iterator<Item = (&'a str, &'a str, &'a str)>,
) -> (Option<String>, bool) {
    use std::collections::BTreeSet;
    let (series, uname_hint, mut conflicting) = release_hints(release);
    let mut hints = BTreeSet::new();
    let mut names = BTreeSet::new();
    let mut hashes = BTreeSet::new();
    for (name, hash, vermagic) in modules {
        let Some(release) = vermagic.split_whitespace().next() else {
            continue;
        };
        let (module_series, hint, module_conflict) = release_hints(release);
        if module_series != series || module_conflict {
            conflicting = true;
            continue;
        }
        let Some(hint) = hint else {
            continue;
        };
        hints.insert(hint);
        names.insert(name.replace('-', "_"));
        hashes.insert(hash);
    }
    conflicting |= hints.len() > 1
        || hints
            .iter()
            .any(|hint| uname_hint.as_ref().is_some_and(|uname| uname != hint));
    let hint = if !conflicting && complete && names.len() >= 2 && hashes.len() >= 2 {
        hints.into_iter().next()
    } else {
        None
    };
    (hint, conflicting)
}

pub(crate) fn corroborates_record(evidence: &vpnhide_core_rpc::NativeSelectionEvidence) -> bool {
    let (hint, conflicting) = module_hints(
        &evidence.kernel_release,
        evidence.inventory_complete,
        evidence
            .modules
            .iter()
            .map(|m| (m.name.as_str(), m.sha256.as_str(), m.vermagic.as_str())),
    );
    !conflicting && hint.is_some() && hint == evidence.module_hint
}

pub(crate) fn supersedes_conflict(
    current: &vpnhide_core_rpc::NativeSelectionEvidence,
    previous: &vpnhide_core_rpc::NativeSelectionEvidence,
) -> bool {
    previous.boot_id != current.boot_id
        || !previous.conflicting_hints
        || (!current.conflicting_hints && corroborates_record(current))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bound(
        name: &str,
        release: &str,
        digest: char,
    ) -> super::super::standard_modules::BoundModule {
        super::super::standard_modules::BoundModule {
            source: format!("/vendor_dlkm/lib/modules/{name}.ko").into(),
            sha256: digest.to_string().repeat(64),
            metadata: super::super::elf::ModuleMetadata {
                name: name.into(),
                vermagic: format!("{release} SMP modversions aarch64"),
                srcversion: None,
                build_id: Some(vec![1, 2, 3, 4]),
            },
        }
    }

    #[test]
    fn only_distinct_live_module_quorum_can_settle_a_manifest_tie() {
        use super::super::standard_modules::Observation;
        let manifest = super::super::tests::manifest();
        let mut observed = Observation {
            modules: vec![bound("ksu_next", "5.15.123-android13", 'a')],
            ..Default::default()
        };
        assert_eq!(
            resolve_with_modules(&manifest, "5.15.123-custom", &observed)
                .unwrap()
                .candidates
                .len(),
            2
        );
        observed.modules.push(observed.modules[0].clone());
        assert_eq!(
            resolve_with_modules(&manifest, "5.15.123-custom", &observed)
                .unwrap()
                .candidates
                .len(),
            2
        );
        observed.modules[1] = bound("wifi", "5.15.123-android13", 'b');
        for reverse in [false, true] {
            let mut manifest = manifest.clone();
            if reverse {
                manifest.payloads.reverse();
                observed.modules.reverse();
            }
            let resolved = resolve_with_modules(&manifest, "5.15.123-custom", &observed).unwrap();
            assert_eq!(resolved.candidates.len(), 1);
            assert_eq!(resolved.candidates[0].kmi, "android13-5.15");
            assert_eq!(resolved.module_hint.as_deref(), Some("android13-5.15"));
        }
        observed.incomplete = true;
        assert_eq!(
            resolve_with_modules(&manifest, "5.15.123-custom", &observed)
                .unwrap()
                .candidates
                .len(),
            2
        );
        observed.incomplete = false;
        observed.modules[1].sha256 = observed.modules[0].sha256.clone();
        assert_eq!(
            resolve_with_modules(&manifest, "5.15.123-custom", &observed)
                .unwrap()
                .candidates
                .len(),
            2
        );
    }

    #[test]
    fn contrary_module_evidence_survives_partial_inventory_and_uname_hints() {
        use super::super::standard_modules::Observation;
        let manifest = super::super::tests::manifest();
        let mut observed = Observation {
            modules: vec![
                bound("wifi", "5.15.123-android13", 'a'),
                bound("ksu_next", "5.15.123-android14", 'b'),
            ],
            incomplete: true,
            ..Default::default()
        };
        let conflict = resolve_with_modules(&manifest, "5.15.123-custom", &observed).unwrap();
        assert!(conflict.conflicting_hints);
        assert!(conflict.module_hint.is_none());
        observed.modules.pop();
        assert!(
            resolve_with_modules(&manifest, "5.15.123-android14", &observed)
                .unwrap()
                .conflicting_hints
        );
        observed.modules[0].metadata.vermagic = "6.1.1-android14 SMP".into();
        assert!(
            resolve_with_modules(&manifest, "5.15.123", &observed)
                .unwrap()
                .conflicting_hints
        );
    }

    #[test]
    fn kernel_version_is_bounded_and_anchored() {
        assert_eq!(
            series("6.1.157-android14-11-custom"),
            Some(LinuxSeries { major: 6, minor: 1 })
        );
        for bad in [
            "x6.1.2",
            " 6.1.2",
            "6.1.2\n",
            "6.1.x",
            "+6.1.2",
            "6..1",
            "6.1.2.3",
            "6.1android14",
            "6.1.999999",
        ] {
            assert_eq!(series(bad), None, "{bad:?}");
        }
        assert_eq!(series(&format!("6.1.2-{}", "x".repeat(256))), None);
    }

    #[test]
    fn candidates_are_manifest_derived_and_release_hint_cannot_choose_a_tie() {
        let manifest = super::super::tests::manifest();
        let r = resolve(&manifest, "5.15.100-android14-custom").unwrap();
        assert_eq!(r.candidates.len(), 2);
        assert!(!r.candidates[0].matches_release_hint);
        assert!(r.candidates[1].matches_release_hint);
        let mut reversed = manifest.clone();
        reversed.payloads.reverse();
        let other = resolve(&reversed, "5.15.100-android14-custom").unwrap();
        assert_eq!(
            r.candidates.iter().map(|c| &c.kmi).collect::<Vec<_>>(),
            other.candidates.iter().map(|c| &c.kmi).collect::<Vec<_>>()
        );
        assert_eq!(resolve(&manifest, "5.15.100").unwrap().candidates.len(), 2);
        let conflict = resolve(&manifest, "5.15.100-android13-android14").unwrap();
        assert!(conflict.conflicting_hints);
        assert!(conflict.release_hint.is_none());
        let unknown = resolve(&manifest, "6.18.0-android17").unwrap();
        assert!(unknown.candidates.is_empty());
        assert_eq!(unknown.release_hint.as_deref(), Some("android17-6.18"));
        let mut partial = manifest.clone();
        partial.payloads.retain(
            |p| !matches!(&p.implementation, PayloadKind::Kmod { kmi } if kmi == "android14-5.15"),
        );
        let partial = resolve(&partial, "5.15.100-android14").unwrap();
        assert_eq!(partial.candidates.len(), 1);
        assert!(!partial.candidates[0].matches_release_hint);
    }
}
