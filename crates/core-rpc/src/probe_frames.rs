//! Portable contract for bounded Core root/debug snapshot framing.
//! Android only records metrics, renders errors and handles files/permissions.
use std::collections::{HashMap, HashSet};

pub const MAX_OUTPUT_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_SECTIONS: usize = 4096;
pub const MAX_NAME_BYTES: usize = 128;

pub mod prefixes {
    pub const ROOT_BEGIN: &str = "__VPNHIDE_ROOT_SECTION_BEGIN__:";
    pub const ROOT_END: &str = "__VPNHIDE_ROOT_SECTION_END__:";
    pub const ROOT_TIMING: &str = "__VPNHIDE_ROOT_TIMING__:";
    pub const DEBUG_BEGIN: &str = "__VPNHIDE_DEBUG_SECTION_BEGIN__:";
    pub const DEBUG_END: &str = "__VPNHIDE_DEBUG_SECTION_END__:";
    pub use vpnhide_native_policy::package_inventory::{
        PM_USER_BEGIN, PM_USER_END, PM_USERS_STATUS,
    };
}

pub const REQUIRED_ROOT_SNAPSHOT_SECTIONS: &[&str] = &[
    "kmod_prop",
    "zygisk_prop",
    "kpm_prop",
    "core_prop",
    "kmod_module_dir",
    "zygisk_module_dir",
    "kpm_module_dir",
    "core_module_dir",
    "kmod_artifact_state",
    "kpm_artifact_state",
    "zygisk_artifact_state",
    "kmod_disabled",
    "kpm_disabled",
    "zygisk_disabled",
    "core_disabled",
    "kmod_pending_update",
    "kpm_pending_update",
    "zygisk_pending_update",
    "core_pending_update",
    "canonical_config",
    "superkey_saved",
    "current_boot_id",
    "zygisk_status",
    "kernel_release",
    "kmod_state",
    "kpm_state",
    "lsposed_state",
    "getenforce",
    "kpatch_runtime",
    "pm_packages",
    "pm_users",
    "snapshot_shell_uid",
    "proc_exists",
    "ports_chain",
    "lsposed_framework",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    Root,
    Debug,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    InputTooLarge,
    TooManySections,
    TooManyTimings,
    NameTooLong,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub name: String,
    pub body: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timing {
    pub name: String,
    pub duration_ms: i64,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedSections {
    pub complete: Vec<Section>,
    pub incomplete: Option<Section>,
    pub timings: Vec<Timing>,
    pub error: Option<FrameError>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootSectionValidation {
    pub missing: Vec<&'static str>,
    pub error: Option<FrameError>,
}

pub fn validate_root_sections<'a>(
    names: impl IntoIterator<Item = &'a str>,
) -> RootSectionValidation {
    let mut present = HashSet::new();
    let mut error = None;
    for (count, name) in names.into_iter().enumerate() {
        if count >= MAX_SECTIONS {
            error = Some(FrameError::TooManySections);
            break;
        }
        if name.len() > MAX_NAME_BYTES {
            error = Some(FrameError::NameTooLong);
            break;
        }
        present.insert(name);
    }
    RootSectionValidation {
        missing: REQUIRED_ROOT_SNAPSHOT_SECTIONS
            .iter()
            .copied()
            .filter(|s| !present.contains(s))
            .collect(),
        error,
    }
}

// Match Kotlin/JVM Char.isWhitespace for this compatibility reader, including
// nonbreaking spaces and C0 separators, excluding NEL. No locale-dependent trim.
fn legacy_space(c: char) -> bool {
    matches!(c, '\t'..='\r' | '\u{001c}'..='\u{0020}' | '\u{00a0}' | '\u{1680}' |
        '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}')
}

pub fn parse(kind: FrameKind, raw: &str) -> ParsedSections {
    let mut out = ParsedSections::default();
    if raw.len() > MAX_OUTPUT_BYTES {
        out.error = Some(FrameError::InputTooLarge);
        return out;
    }
    let (begin, end) = match kind {
        FrameKind::Root => (prefixes::ROOT_BEGIN, prefixes::ROOT_END),
        FrameKind::Debug => (prefixes::DEBUG_BEGIN, prefixes::DEBUG_END),
    };
    let mut current: Option<String> = None;
    let mut body = String::new();
    let mut indexes: HashMap<String, usize> = HashMap::new();
    // Kotlin lineSequence recognizes LF, CRLF, bare CR and a trailing empty
    // line. Strip CR only at each LF boundary, then split remaining bare CRs.
    let lines = raw
        .split('\n')
        .flat_map(|s| s.strip_suffix('\r').unwrap_or(s).split('\r'))
        .chain(raw.ends_with('\r').then_some(""));
    for line in lines {
        if kind == FrameKind::Root
            && let Some(metric) = line.strip_prefix(prefixes::ROOT_TIMING)
        {
            if let Some((name, value)) = metric.split_once('=') {
                let name = name.trim_matches(legacy_space);
                if !name.is_empty()
                    && let Ok(value) = value.trim_matches(legacy_space).parse::<i64>()
                {
                    if name.len() > MAX_NAME_BYTES {
                        out.error = Some(FrameError::NameTooLong);
                        break;
                    }
                    if out.timings.len() >= MAX_SECTIONS {
                        out.error = Some(FrameError::TooManyTimings);
                        break;
                    }
                    out.timings.push(Timing {
                        name: name.into(),
                        duration_ms: value,
                    });
                }
            }
            continue;
        }
        if let Some(name) = line.strip_prefix(begin) {
            if name.len() > MAX_NAME_BYTES {
                out.error = Some(FrameError::NameTooLong);
                break;
            }
            current = Some(name.into());
            body.clear();
        } else if let Some(offset) = line.find(end) {
            if offset > 0 && current.is_some() {
                if !body.is_empty() {
                    body.push('\n');
                }
                body.push_str(&line[..offset]);
            }
            let name = &line[offset + end.len()..];
            if current.as_deref() == Some(name) {
                if !indexes.contains_key(name) && out.complete.len() >= MAX_SECTIONS {
                    out.error = Some(FrameError::TooManySections);
                    break;
                }
                if kind == FrameKind::Debug {
                    body.truncate(body.trim_end_matches(legacy_space).len());
                }
                let value = std::mem::take(&mut body);
                if let Some(index) = indexes.get(name) {
                    out.complete[*index].body = value;
                } else {
                    indexes.insert(name.into(), out.complete.len());
                    out.complete.push(Section {
                        name: name.into(),
                        body: value,
                    });
                }
                current = None;
                body.clear();
            } else if kind == FrameKind::Root {
                current = None;
                body.clear();
            }
        } else if current.is_some() {
            if !body.is_empty() {
                body.push('\n');
            }
            body.push_str(line);
        }
    }
    if kind == FrameKind::Debug
        && let Some(name) = current
    {
        body.truncate(body.trim_end_matches(legacy_space).len());
        out.incomplete = Some(Section { name, body });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_keys_match_the_independent_snapshot_contract_fixture() {
        let expected: Vec<String> = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/core-probes/root-required.json"
        )))
        .unwrap();
        assert_eq!(expected, REQUIRED_ROOT_SNAPSHOT_SECTIONS);
    }
    fn body(result: &ParsedSections, name: &str) -> Option<String> {
        result
            .complete
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.body.clone())
    }
    #[test]
    fn root_keeps_exact_multiline_content_and_signed_timings_without_borrowing_debug_frames() {
        let raw = "noise\n__VPNHIDE_ROOT_SECTION_BEGIN__:config\nfirst\n\nlast  \n__VPNHIDE_ROOT_SECTION_END__:config\n__VPNHIDE_ROOT_TIMING__: target_files = +42 \n__VPNHIDE_ROOT_TIMING__:negative=-1\n__VPNHIDE_ROOT_TIMING__:bad=9223372036854775808\n__VPNHIDE_ROOT_SECTION_BEGIN__:empty\n__VPNHIDE_ROOT_SECTION_END__:empty\n__VPNHIDE_DEBUG_SECTION_BEGIN__:wrong\ndata\n__VPNHIDE_DEBUG_SECTION_END__:wrong";
        let out = parse(FrameKind::Root, raw);
        assert_eq!(body(&out, "config").as_deref(), Some("first\n\nlast  "));
        assert_eq!(body(&out, "empty").as_deref(), Some(""));
        assert!(body(&out, "wrong").is_none());
        assert_eq!(
            out.timings,
            [
                Timing {
                    name: "target_files".into(),
                    duration_ms: 42
                },
                Timing {
                    name: "negative".into(),
                    duration_ms: -1
                }
            ]
        );
        assert!(out.incomplete.is_none() && out.error.is_none());
    }
    #[test]
    fn root_discards_incomplete_and_mismatched_sections_debug_keeps_qualified_partial_text() {
        let root = "__VPNHIDE_ROOT_SECTION_BEGIN__:expected\nfirst\n__VPNHIDE_ROOT_SECTION_END__:other\nsecond\n__VPNHIDE_ROOT_SECTION_END__:expected\n__VPNHIDE_ROOT_SECTION_BEGIN__:partial\nlost";
        let r = parse(FrameKind::Root, root);
        assert!(r.complete.is_empty() && r.incomplete.is_none());
        let debug = "__VPNHIDE_DEBUG_SECTION_BEGIN__:expected\nfirst\n__VPNHIDE_DEBUG_SECTION_END__:other\nsecond\n__VPNHIDE_DEBUG_SECTION_END__:expected\n__VPNHIDE_DEBUG_SECTION_BEGIN__:partial\nlost  ";
        let r = parse(FrameKind::Debug, debug);
        assert_eq!(body(&r, "expected").as_deref(), Some("first\nsecond"));
        assert_eq!(
            r.incomplete,
            Some(Section {
                name: "partial".into(),
                body: "lost".into()
            })
        );
    }
    #[test]
    fn inline_end_newline_styles_and_last_duplicate_keep_legacy_order() {
        for eol in ["\n", "\r\n", "\r"] {
            let raw = [
                "__VPNHIDE_DEBUG_SECTION_BEGIN__:x",
                "{}__VPNHIDE_DEBUG_SECTION_END__:x",
                "__VPNHIDE_DEBUG_SECTION_BEGIN__:y",
                "value",
                "__VPNHIDE_DEBUG_SECTION_END__:y",
                "__VPNHIDE_DEBUG_SECTION_BEGIN__:x",
                "updated",
                "__VPNHIDE_DEBUG_SECTION_END__:x",
                "",
            ]
            .join(eol);
            let r = parse(FrameKind::Debug, &raw);
            assert!(r.error.is_none());
            assert_eq!(
                r.complete,
                [
                    Section {
                        name: "x".into(),
                        body: "updated".into()
                    },
                    Section {
                        name: "y".into(),
                        body: "value".into()
                    }
                ]
            );
        }
        let r = parse(
            FrameKind::Root,
            "__VPNHIDE_ROOT_SECTION_BEGIN__:x\n{}__VPNHIDE_ROOT_SECTION_END__:x",
        );
        assert_eq!(body(&r, "x").as_deref(), Some("{}"));
    }
    #[test]
    fn debug_trim_retains_nel_but_matches_jvm_nonbreaking_and_c0_spaces() {
        let raw = "__VPNHIDE_DEBUG_SECTION_BEGIN__:x\nv\u{0085}\u{00a0}\u{001c}\u{202f}\n__VPNHIDE_DEBUG_SECTION_END__:x";
        assert_eq!(
            body(&parse(FrameKind::Debug, raw), "x").as_deref(),
            Some("v\u{0085}")
        );
    }
    #[test]
    fn required_validation_accepts_empty_values_not_missing_sections_and_bounds_input() {
        let names: Vec<_> = REQUIRED_ROOT_SNAPSHOT_SECTIONS
            .iter()
            .copied()
            .filter(|s| *s != "snapshot_shell_uid")
            .collect();
        assert_eq!(
            validate_root_sections(names).missing,
            ["snapshot_shell_uid"]
        );
        assert!(
            validate_root_sections(REQUIRED_ROOT_SNAPSHOT_SECTIONS.iter().copied())
                .missing
                .is_empty()
        );
        assert_eq!(
            validate_root_sections(["future"; MAX_SECTIONS + 1]).error,
            Some(FrameError::TooManySections)
        );
        let long = "x".repeat(MAX_NAME_BYTES + 1);
        assert_eq!(
            validate_root_sections([long.as_str()]).error,
            Some(FrameError::NameTooLong)
        );
        assert_eq!(REQUIRED_ROOT_SNAPSHOT_SECTIONS.len(), 35);
        assert_eq!(
            REQUIRED_ROOT_SNAPSHOT_SECTIONS
                .iter()
                .collect::<HashSet<_>>()
                .len(),
            35
        );
    }
    #[test]
    fn oversized_and_excessive_frames_never_claim_complete_validity() {
        let r = parse(FrameKind::Root, &"x".repeat(MAX_OUTPUT_BYTES + 1));
        assert_eq!(r.error, Some(FrameError::InputTooLarge));
        assert!(r.complete.is_empty());
        let mut input = String::new();
        for i in 0..=MAX_SECTIONS {
            input.push_str(&format!(
                "__VPNHIDE_ROOT_SECTION_BEGIN__:{i}\n__VPNHIDE_ROOT_SECTION_END__:{i}\n"
            ));
        }
        let r = parse(FrameKind::Root, &input);
        assert_eq!(r.complete.len(), MAX_SECTIONS);
        assert_eq!(r.error, Some(FrameError::TooManySections));
        let mut debug = String::new();
        for i in 0..MAX_SECTIONS {
            debug.push_str(&format!(
                "__VPNHIDE_DEBUG_SECTION_BEGIN__:{i}\n__VPNHIDE_DEBUG_SECTION_END__:{i}\n"
            ));
        }
        debug.push_str("__VPNHIDE_DEBUG_SECTION_BEGIN__:overflow\nvaluable partial\n__VPNHIDE_DEBUG_SECTION_END__:overflow");
        let r = parse(FrameKind::Debug, &debug);
        assert_eq!(r.error, Some(FrameError::TooManySections));
        assert_eq!(
            r.incomplete,
            Some(Section {
                name: "overflow".into(),
                body: "valuable partial".into()
            })
        );
        let r = parse(
            FrameKind::Root,
            &format!(
                "__VPNHIDE_ROOT_SECTION_BEGIN__:{}",
                "x".repeat(MAX_NAME_BYTES + 1)
            ),
        );
        assert_eq!(r.error, Some(FrameError::NameTooLong));
        let r = parse(
            FrameKind::Root,
            &"__VPNHIDE_ROOT_TIMING__:phase=1\n".repeat(MAX_SECTIONS + 1),
        );
        assert_eq!(r.timings.len(), MAX_SECTIONS);
        assert_eq!(r.error, Some(FrameError::TooManyTimings));
    }
}
