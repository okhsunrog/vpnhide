//! User-requested Core replacement. Staging and running-generation proof are
//! separate facts; none of these states authorizes an automatic reboot.
use crate::OperationId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreReplacementRequest {
    pub descriptor: Vec<u8>,
    pub expected_commit: String,
    pub expected_package_sha256: String,
    pub target_rpc: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreMaintenanceStage {
    Accepted,
    Dispatching,
    VerifyingStaged,
    RetiringLegacy,
    PendingRestart,
    Verified,
    FailedKnown,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreMaintenanceFailure {
    InvalidPackage,
    ManagerUnavailable,
    ManagerChanged,
    LiveImageChanged,
    StagedImageMissing,
    StagedImageInvalid,
    LegacyChanged,
    DispatchUncertain,
    StorageUncertain,
    InterruptedBeforeDispatch,
    BootMismatch,
    RunningImageMismatch,
    Compatibility,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreMaintenanceStatus {
    pub operation: OperationId,
    pub target_commit: String,
    pub stage: CoreMaintenanceStage,
    pub failure: Option<CoreMaintenanceFailure>,
}
