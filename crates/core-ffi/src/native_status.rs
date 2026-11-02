//! Mechanical projection of Core-owned native selection, setup and coverage.
use crate::{CoreNativeBackend, CoreObservationFreshness, CorePhaseOutcome};
use vpnhide_core_rpc as rpc;

macro_rules! status_enum {
    ($name:ident, $source:ident, $($variant:ident),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
        pub enum $name { $($variant),+ }
        impl From<rpc::$source> for $name {
            fn from(value: rpc::$source) -> Self { match value { $(rpc::$source::$variant => Self::$variant),+ } }
        }
    };
}
status_enum!(
    CoreNativeImplementationSource,
    NativeImplementationSource,
    Builtin,
    Bundled,
    Legacy,
    Unknown
);
status_enum!(
    CoreNativeAttemptDisposition,
    NativeAttemptDisposition,
    Pending,
    Selected,
    RejectedNoEffect,
    Refused,
    Unknown
);
status_enum!(
    CoreNativeSetupHealth,
    NativeSetupHealth,
    Unavailable,
    NoSelection,
    AwaitingRuntime,
    RuntimeConflict,
    DeliveryPending,
    DeliveryFailed,
    DeliveryUnknown,
    PolicyUnknown,
    PartialHooks,
    BackendError,
    SetupReady,
    AwaitingProcessEvidence
);
status_enum!(
    CoreNativeFilesystemSetup,
    NativeFilesystemSetup,
    Unavailable,
    Unknown,
    Disabled,
    Enabled,
    PendingEnable,
    PendingDisable,
    SetupError
);
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreNativeImplementation {
    pub backend: CoreNativeBackend,
    pub source: CoreNativeImplementationSource,
    pub manifest_sha256: Option<String>,
    pub source_commit: Option<String>,
    pub payload_sha256: Option<String>,
}
impl From<rpc::NativeImplementation> for CoreNativeImplementation {
    fn from(v: rpc::NativeImplementation) -> Self {
        Self {
            backend: v.backend.into(),
            source: v.source.into(),
            manifest_sha256: v.manifest_sha256,
            source_commit: v.source_commit,
            payload_sha256: v.payload_sha256,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreNativeRuntimeStatus {
    pub backend: CoreNativeBackend,
    pub kernel_version: u32,
    pub installed_hooks: u32,
    pub status_error: u32,
}
impl From<rpc::NativeRuntimeStatus> for CoreNativeRuntimeStatus {
    fn from(v: rpc::NativeRuntimeStatus) -> Self {
        Self {
            backend: v.backend.into(),
            kernel_version: v.kernel_version,
            installed_hooks: v.installed_hooks,
            status_error: v.status_error,
        }
    }
}
impl From<CoreNativeRuntimeStatus> for rpc::NativeRuntimeStatus {
    fn from(v: CoreNativeRuntimeStatus) -> Self {
        Self {
            backend: v.backend.into(),
            kernel_version: v.kernel_version,
            installed_hooks: v.installed_hooks,
            status_error: v.status_error,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreNativeAttemptView {
    pub boot_id: String,
    pub config_revision: u64,
    pub operation: Option<u64>,
    pub implementation: Option<CoreNativeImplementation>,
    pub disposition: CoreNativeAttemptDisposition,
}
impl From<rpc::NativeAttemptView> for CoreNativeAttemptView {
    fn from(v: rpc::NativeAttemptView) -> Self {
        Self {
            boot_id: v.boot_id,
            config_revision: v.config_revision,
            operation: v.operation.map(|op| op.0),
            implementation: v.implementation.map(Into::into),
            disposition: v.disposition.into(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreNativeDeliveryView {
    pub backend: Option<CoreNativeBackend>,
    pub operation: u64,
    pub config_revision: u64,
    pub outcome: CorePhaseOutcome,
    pub wire_sha256: Option<String>,
}
impl From<rpc::NativeDeliveryView> for CoreNativeDeliveryView {
    fn from(v: rpc::NativeDeliveryView) -> Self {
        Self {
            backend: v.backend.map(Into::into),
            operation: v.operation.0,
            config_revision: v.config_revision,
            outcome: v.outcome.into(),
            wire_sha256: v.wire_sha256,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreNativeReadModel {
    pub boot: Option<crate::CoreNativeBootStatus>,
    pub selected: Option<CoreNativeImplementation>,
    pub applied: Option<CoreNativeBackend>,
    pub observed: Option<CoreNativeBackend>,
    pub displayed: Option<CoreNativeImplementation>,
    pub observation: CoreObservationFreshness,
    pub runtime: Option<CoreNativeRuntimeStatus>,
    pub observation_error: Option<String>,
    pub attempts: Vec<CoreNativeAttemptView>,
    pub delivery: Option<CoreNativeDeliveryView>,
    pub required_hooks: Option<u32>,
    pub installed_hooks: Option<u32>,
    pub missing_hooks: Option<u32>,
    pub setup: CoreNativeSetupHealth,
    pub filesystem: CoreNativeFilesystemSetup,
    pub process_coverage_complete: bool,
    pub legacy_installed: Vec<CoreNativeBackend>,
}
impl From<rpc::NativeReadModel> for CoreNativeReadModel {
    fn from(v: rpc::NativeReadModel) -> Self {
        Self {
            boot: v.boot.map(Into::into),
            selected: v.selected.map(Into::into),
            applied: v.applied.map(Into::into),
            observed: v.observed.map(Into::into),
            displayed: v.displayed.map(Into::into),
            observation: v.observation.into(),
            runtime: v.runtime.map(Into::into),
            observation_error: v.observation_error,
            attempts: v.attempts.into_iter().map(Into::into).collect(),
            delivery: v.delivery.map(Into::into),
            required_hooks: v.required_hooks,
            installed_hooks: v.installed_hooks,
            missing_hooks: v.missing_hooks,
            setup: v.setup.into(),
            filesystem: v.filesystem.into(),
            process_coverage_complete: v.process_coverage_complete,
            legacy_installed: v.legacy_installed.into_iter().map(Into::into).collect(),
        }
    }
}
