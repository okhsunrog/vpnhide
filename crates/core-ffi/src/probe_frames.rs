//! Local typed snapshot codec; no RPC, root effect, URL or path input.
use vpnhide_core_rpc::probe_frames as model;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CoreProbeFrameKind {
    Root,
    Debug,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CoreProbeFrameError {
    InputTooLarge,
    TooManySections,
    TooManyTimings,
    NameTooLong,
}
impl From<model::FrameError> for CoreProbeFrameError {
    fn from(v: model::FrameError) -> Self {
        match v {
            model::FrameError::InputTooLarge => Self::InputTooLarge,
            model::FrameError::TooManySections => Self::TooManySections,
            model::FrameError::TooManyTimings => Self::TooManyTimings,
            model::FrameError::NameTooLong => Self::NameTooLong,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreProbeSection {
    pub name: String,
    pub body: String,
}
impl From<model::Section> for CoreProbeSection {
    fn from(v: model::Section) -> Self {
        Self {
            name: v.name,
            body: v.body,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreProbeTiming {
    pub name: String,
    pub duration_ms: i64,
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreParsedProbeSections {
    pub complete: Vec<CoreProbeSection>,
    pub incomplete: Option<CoreProbeSection>,
    pub timings: Vec<CoreProbeTiming>,
    pub error: Option<CoreProbeFrameError>,
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreRootSectionValidation {
    pub missing: Vec<String>,
    pub error: Option<CoreProbeFrameError>,
}

#[uniffi::export]
pub fn core_parse_probe_sections(kind: CoreProbeFrameKind, raw: String) -> CoreParsedProbeSections {
    let kind = match kind {
        CoreProbeFrameKind::Root => model::FrameKind::Root,
        CoreProbeFrameKind::Debug => model::FrameKind::Debug,
    };
    let v = model::parse(kind, &raw);
    CoreParsedProbeSections {
        complete: v.complete.into_iter().map(Into::into).collect(),
        incomplete: v.incomplete.map(Into::into),
        timings: v
            .timings
            .into_iter()
            .map(|v| CoreProbeTiming {
                name: v.name,
                duration_ms: v.duration_ms,
            })
            .collect(),
        error: v.error.map(Into::into),
    }
}

#[uniffi::export]
pub fn core_required_root_snapshot_sections() -> Vec<String> {
    model::REQUIRED_ROOT_SNAPSHOT_SECTIONS
        .iter()
        .map(|s| (*s).into())
        .collect()
}

#[uniffi::export]
pub fn core_validate_root_snapshot_sections(present: Vec<String>) -> CoreRootSectionValidation {
    let v = model::validate_root_sections(present.iter().map(String::as_str));
    CoreRootSectionValidation {
        missing: v.missing.into_iter().map(Into::into).collect(),
        error: v.error.map(Into::into),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_codec_keeps_signed_metrics_partial_content_and_bound_errors() {
        let parsed = core_parse_probe_sections(CoreProbeFrameKind::Root,
            "__VPNHIDE_ROOT_SECTION_BEGIN__:x\n{}__VPNHIDE_ROOT_SECTION_END__:x\n__VPNHIDE_ROOT_TIMING__:phase=-1".into());
        assert_eq!(
            parsed.complete,
            [CoreProbeSection {
                name: "x".into(),
                body: "{}".into()
            }]
        );
        assert_eq!(
            parsed.timings,
            [CoreProbeTiming {
                name: "phase".into(),
                duration_ms: -1
            }]
        );
        assert!(parsed.error.is_none());
        let parsed = core_parse_probe_sections(
            CoreProbeFrameKind::Debug,
            format!(
                "__VPNHIDE_DEBUG_SECTION_BEGIN__:partial\nkept\n__VPNHIDE_DEBUG_SECTION_BEGIN__:{}",
                "x".repeat(model::MAX_NAME_BYTES + 1)
            ),
        );
        assert_eq!(
            parsed.incomplete,
            Some(CoreProbeSection {
                name: "partial".into(),
                body: "kept".into()
            })
        );
        assert_eq!(parsed.error, Some(CoreProbeFrameError::NameTooLong));
    }
    #[test]
    fn validation_boundary_keeps_missing_identity_distinct_from_bad_names() {
        let present: Vec<_> = core_required_root_snapshot_sections()
            .into_iter()
            .filter(|s| s != "snapshot_shell_uid")
            .collect();
        let result = core_validate_root_snapshot_sections(present);
        assert_eq!(result.missing, ["snapshot_shell_uid"]);
        assert!(result.error.is_none());
        assert_eq!(
            core_validate_root_snapshot_sections(vec!["x".repeat(model::MAX_NAME_BYTES + 1)]).error,
            Some(CoreProbeFrameError::NameTooLong)
        );
    }
}
