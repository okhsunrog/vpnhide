//! Typed framework observations only. All admission, authority and verdicts are Core-owned.
use crate::{CoreDiagnosticError, CoreDiagnosticWorkerTask, CoreProbeError, CoreSession};
use vpnhide_diagnostic_model::{Sample, framework as model};
macro_rules! sample {
    ($name:ident, $ty:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
        pub enum $name {
            Value { value: $ty },
            Error { error: CoreProbeError },
        }
        impl From<$name> for Sample<$ty> {
            fn from(value: $name) -> Self {
                match value {
                    $name::Value { value } => Self::Value(value),
                    $name::Error { error } => Self::Error(error.into()),
                }
            }
        }
    };
}
sample!(CoreBooleanSample, bool);
sample!(CoreHandlesSample, Vec<u64>);
sample!(CoreTextSample, String);
sample!(CoreOptionalTextSample, Option<String>);
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreLegacyCallWindow {
    pub started_boot_ns: u64,
    pub finished_boot_ns: u64,
}
impl From<CoreLegacyCallWindow> for vpnhide_diagnostic_model::framework_legacy::LegacyCallWindow {
    fn from(v: CoreLegacyCallWindow) -> Self {
        Self {
            started_boot_ns: v.started_boot_ns,
            finished_boot_ns: v.finished_boot_ns,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreLegacyNetworkInfo {
    pub network_type: i32,
    pub state: String,
    pub detailed_state: String,
    pub fixture: bool,
}
impl From<CoreLegacyNetworkInfo> for vpnhide_diagnostic_model::framework_legacy::LegacyNetworkInfo {
    fn from(v: CoreLegacyNetworkInfo) -> Self {
        Self {
            network_type: v.network_type,
            state: v.state,
            detailed_state: v.detailed_state,
            fixture: v.fixture,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreLegacyOrdinary {
    pub count: u32,
    pub sha256: String,
    pub fixture_present: bool,
}
impl From<CoreLegacyOrdinary> for vpnhide_diagnostic_model::framework_legacy::LegacyOrdinary {
    fn from(v: CoreLegacyOrdinary) -> Self {
        Self {
            count: v.count,
            sha256: v.sha256,
            fixture_present: v.fixture_present,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreLegacyInfoSample {
    Value {
        value: Option<CoreLegacyNetworkInfo>,
    },
    Error {
        error: CoreProbeError,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreLegacyInfosSample {
    Value { value: Vec<CoreLegacyNetworkInfo> },
    Error { error: CoreProbeError },
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreLegacyOrdinarySample {
    Value { value: CoreLegacyOrdinary },
    Error { error: CoreProbeError },
}
impl From<CoreLegacyInfoSample>
    for Sample<Option<vpnhide_diagnostic_model::framework_legacy::LegacyNetworkInfo>>
{
    fn from(v: CoreLegacyInfoSample) -> Self {
        match v {
            CoreLegacyInfoSample::Value { value } => Self::Value(value.map(Into::into)),
            CoreLegacyInfoSample::Error { error } => Self::Error(error.into()),
        }
    }
}
impl From<CoreLegacyInfosSample>
    for Sample<Vec<vpnhide_diagnostic_model::framework_legacy::LegacyNetworkInfo>>
{
    fn from(v: CoreLegacyInfosSample) -> Self {
        match v {
            CoreLegacyInfosSample::Value { value } => {
                Self::Value(value.into_iter().map(Into::into).collect())
            }
            CoreLegacyInfosSample::Error { error } => Self::Error(error.into()),
        }
    }
}
impl From<CoreLegacyOrdinarySample>
    for Sample<vpnhide_diagnostic_model::framework_legacy::LegacyOrdinary>
{
    fn from(v: CoreLegacyOrdinarySample) -> Self {
        match v {
            CoreLegacyOrdinarySample::Value { value } => Self::Value(value.into()),
            CoreLegacyOrdinarySample::Error { error } => Self::Error(error.into()),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreFrameworkObservation {
    pub networks: CoreHandlesSample,
    pub vpn_capabilities_present: CoreBooleanSample,
    pub vpn_transport: CoreBooleanSample,
    pub vpn_properties_present: CoreBooleanSample,
    pub vpn_interface: CoreOptionalTextSample,
    pub ordinary_capabilities_sha256: CoreTextSample,
    pub ordinary_properties_sha256: CoreTextSample,
    pub legacy_type_window: Option<CoreLegacyCallWindow>,
    pub legacy_enumeration_window: Option<CoreLegacyCallWindow>,
    pub legacy_type: CoreLegacyInfoSample,
    pub legacy_enumeration: CoreLegacyInfosSample,
    pub legacy_ordinary: CoreLegacyOrdinarySample,
}
impl From<CoreFrameworkObservation> for model::FrameworkObservation {
    fn from(v: CoreFrameworkObservation) -> Self {
        Self {
            networks: v.networks.into(),
            vpn_capabilities_present: v.vpn_capabilities_present.into(),
            vpn_transport: v.vpn_transport.into(),
            vpn_properties_present: v.vpn_properties_present.into(),
            vpn_interface: v.vpn_interface.into(),
            ordinary_capabilities_sha256: v.ordinary_capabilities_sha256.into(),
            ordinary_properties_sha256: v.ordinary_properties_sha256.into(),
            legacy: vpnhide_diagnostic_model::framework_legacy::LegacyObservation {
                type_window: v.legacy_type_window.map(Into::into),
                enumeration_window: v.legacy_enumeration_window.map(Into::into),
                type_info: v.legacy_type.into(),
                enumeration: v.legacy_enumeration.into(),
                ordinary: v.legacy_ordinary.into(),
            },
        }
    }
}
macro_rules! simple_enum {
    ($name:ident, $source:ident, $($variant:ident),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
        pub enum $name { $($variant),+ }
        impl From<model::$source> for $name { fn from(v: model::$source) -> Self { match v { $(model::$source::$variant => Self::$variant),+ } } }
        impl From<$name> for model::$source { fn from(v: $name) -> Self { match v { $($name::$variant => Self::$variant),+ } } }
    }
}
simple_enum!(
    CoreFrameworkPhase,
    FrameworkWorkerPhase,
    Prepared,
    Raw,
    Protected,
    Revoked,
    Finished
);
simple_enum!(
    CoreFrameworkCheck,
    FrameworkCheck,
    Networks,
    Capabilities,
    LinkProperties,
    LegacyTypeVpn,
    LegacyEnumeration
);
simple_enum!(
    CoreFrameworkControlFailure,
    FrameworkControlFailure,
    InvalidEnvelope,
    IdentityChanged,
    MissingNegativeControl,
    MissingPositiveControl,
    SourceChanged,
    ProductionPathNotObserved,
    OrdinaryObjectsChanged,
    ProbeUnavailable,
    CleanupUnconfirmed
);
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreFrameworkTask {
    pub phase: CoreFrameworkPhase,
    pub vpn_handle: u64,
    pub ordinary_handle: u64,
    pub vpn_marker: String,
    pub ordinary_marker: String,
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreFrameworkAttribution {
    pub session: u64,
    pub core_epoch: u64,
    pub boot_id: String,
    pub worker_uid: u32,
    pub worker_pid: u32,
    pub worker_start_ticks: u64,
    pub system_server_pid: u32,
    pub system_server_start_ticks: u64,
    pub hook_source_sha256: String,
    pub source_profile_sha256: String,
}
impl From<model::FrameworkAttribution> for CoreFrameworkAttribution {
    fn from(v: model::FrameworkAttribution) -> Self {
        Self {
            session: v.session,
            core_epoch: v.core_epoch,
            boot_id: v.boot_id,
            worker_uid: v.worker_uid,
            worker_pid: v.worker_pid,
            worker_start_ticks: v.worker_start_ticks,
            system_server_pid: v.system_server_pid,
            system_server_start_ticks: v.system_server_start_ticks,
            hook_source_sha256: v.hook_source_sha256,
            source_profile_sha256: v.source_profile_sha256,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreScopedFrameworkResult {
    pub attribution: Option<CoreFrameworkAttribution>,
    pub proven: Vec<CoreFrameworkCheck>,
    pub leaked: Vec<CoreFrameworkCheck>,
    pub controls: Vec<CoreFrameworkControlFailure>,
}
impl From<model::ScopedFrameworkResult> for CoreScopedFrameworkResult {
    fn from(v: model::ScopedFrameworkResult) -> Self {
        Self {
            attribution: v.attribution.map(Into::into),
            proven: v.proven.into_iter().map(Into::into).collect(),
            leaked: v.leaked.into_iter().map(Into::into).collect(),
            controls: v.controls.into_iter().map(Into::into).collect(),
        }
    }
}
#[uniffi::export(async_runtime = "tokio")]
impl CoreSession {
    pub async fn framework_worker_step(
        &self,
        task: CoreDiagnosticWorkerTask,
        hook_sha256: String,
        phase: CoreFrameworkPhase,
        previous: Option<CoreFrameworkObservation>,
    ) -> Result<Option<CoreFrameworkTask>, CoreDiagnosticError> {
        self.client
            .framework_worker_step(model::FrameworkWorkerRequest {
                session: task.session,
                core_epoch: task.core_epoch,
                hook_sha256,
                phase: phase.into(),
                previous: previous.map(Into::into),
            })
            .await?
            .map(|v| {
                v.map(|v| CoreFrameworkTask {
                    phase: v.phase.into(),
                    vpn_handle: v.vpn_handle,
                    ordinary_handle: v.ordinary_handle,
                    vpn_marker: v.vpn_marker,
                    ordinary_marker: v.ordinary_marker,
                })
            })
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_worker_samples_keep_null_distinct_from_error_and_preserve_bounds() {
        let value = CoreFrameworkObservation {
            legacy_type_window: None,
            legacy_enumeration_window: None,
            legacy_type: CoreLegacyInfoSample::Value { value: None },
            legacy_enumeration: CoreLegacyInfosSample::Error {
                error: CoreProbeError::Unsupported,
            },
            legacy_ordinary: CoreLegacyOrdinarySample::Error {
                error: CoreProbeError::Unsupported,
            },
            networks: CoreHandlesSample::Value {
                value: (1..=257).collect(),
            },
            vpn_capabilities_present: CoreBooleanSample::Value { value: false },
            vpn_transport: CoreBooleanSample::Error {
                error: CoreProbeError::Errno { code: 13 },
            },
            vpn_properties_present: CoreBooleanSample::Value { value: false },
            vpn_interface: CoreOptionalTextSample::Value { value: None },
            ordinary_capabilities_sha256: CoreTextSample::Error {
                error: CoreProbeError::Unsupported,
            },
            ordinary_properties_sha256: CoreTextSample::Value {
                value: "a".repeat(64),
            },
        };
        let mapped: model::FrameworkObservation = value.into();
        assert!(!mapped.bounded());
        assert_eq!(mapped.vpn_interface, Sample::Value(None));
        assert_eq!(
            mapped.vpn_transport,
            Sample::Error(vpnhide_diagnostic_model::ProbeError::Errno(13))
        );
        assert_eq!(
            mapped.ordinary_capabilities_sha256,
            Sample::Error(vpnhide_diagnostic_model::ProbeError::Unsupported)
        );
    }
}
