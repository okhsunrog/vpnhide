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

/// No effect and no implicit trust elevation. Even one candidate only means
/// catalog uniqueness; actual loading still needs runtime/ownership/health gates.
pub fn resolve(manifest: &Manifest, release: &str) -> std::io::Result<KmiResolution> {
    manifest.validate()?;
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
        conflicting_hints: hints.len() > 1,
        candidates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
