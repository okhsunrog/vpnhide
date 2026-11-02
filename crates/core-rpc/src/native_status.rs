//! Core-owned native read model. Availability, selection, delivery and observed
//! hook installation are separate; none constitutes a measured protection PASS.
use crate::{NativeBackend, ObservationFreshness, OperationId, PhaseOutcome};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeImplementationSource {
    Builtin,
    Bundled,
    Legacy,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeImplementation {
    pub backend: NativeBackend,
    pub source: NativeImplementationSource,
    pub manifest_sha256: Option<String>,
    pub source_commit: Option<String>,
    pub payload_sha256: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeAttemptDisposition {
    Pending,
    Selected,
    RejectedNoEffect,
    Refused,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeAttemptView {
    pub boot_id: String,
    pub config_revision: u64,
    pub operation: Option<OperationId>,
    pub implementation: Option<NativeImplementation>,
    pub disposition: NativeAttemptDisposition,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeDeliveryView {
    pub backend: Option<NativeBackend>,
    pub operation: OperationId,
    pub config_revision: u64,
    pub outcome: PhaseOutcome,
    /// Unknown after old receipt pruning is honest; never reconstruct from telemetry.
    pub wire_sha256: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeRuntimeStatus {
    pub backend: NativeBackend,
    pub kernel_version: u32,
    pub installed_hooks: u32,
    pub status_error: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeSetupHealth {
    #[default]
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
    /// Installation/delivery setup only; diagnostic HookHealth is independent.
    SetupReady,
    /// Authorization/publication is not global process consumption or hook proof.
    AwaitingProcessEvidence,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeFilesystemSetup {
    #[default]
    Unavailable,
    Unknown,
    Disabled,
    Enabled,
    PendingEnable,
    PendingDisable,
    SetupError,
}
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct NativeReadModel {
    pub boot: Option<crate::NativeBootStatus>,
    pub selected: Option<NativeImplementation>,
    pub applied: Option<NativeBackend>,
    pub observed: Option<NativeBackend>,
    /// Core chooses this for display; legacy installation order cannot override it.
    pub displayed: Option<NativeImplementation>,
    pub observation: ObservationFreshness,
    pub runtime: Option<NativeRuntimeStatus>,
    pub observation_error: Option<String>,
    pub attempts: Vec<NativeAttemptView>,
    pub delivery: Option<NativeDeliveryView>,
    /// Desired supported policy, independent of which hooks happened to install.
    pub required_hooks: Option<u32>,
    pub installed_hooks: Option<u32>,
    pub missing_hooks: Option<u32>,
    pub setup: NativeSetupHealth,
    pub filesystem: NativeFilesystemSetup,
    /// No claim of loaded-game process coverage is made by this setup model.
    pub process_coverage_complete: bool,
    /// Standalone legacy artifacts only; bundled alternatives are not conflicts.
    pub legacy_installed: Vec<NativeBackend>,
}
