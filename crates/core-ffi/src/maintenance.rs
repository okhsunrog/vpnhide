//! Mechanical local projection of the Core maintenance contract.
use vpnhide_core_rpc as rpc;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreReplacementRequest {
    pub descriptor: Vec<u8>,
    pub expected_commit: String,
    pub expected_package_sha256: String,
    pub target_rpc: u32,
}
impl From<CoreReplacementRequest> for rpc::CoreReplacementRequest {
    fn from(v: CoreReplacementRequest) -> Self {
        Self {
            descriptor: v.descriptor,
            expected_commit: v.expected_commit,
            expected_package_sha256: v.expected_package_sha256,
            target_rpc: v.target_rpc,
        }
    }
}
macro_rules! project_enum {
    ($name:ident,$($variant:ident),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
        pub enum $name { $($variant),+ }
        impl From<rpc::$name> for $name {
            fn from(v:rpc::$name)->Self {match v {$(rpc::$name::$variant=>Self::$variant),+}}
        }
    };
}
project_enum!(
    CoreMaintenanceStage,
    Accepted,
    Dispatching,
    VerifyingStaged,
    RetiringLegacy,
    PendingRestart,
    Verified,
    FailedKnown,
    Unknown
);
project_enum!(
    CoreMaintenanceFailure,
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
    Compatibility
);
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreMaintenanceStatus {
    pub operation: u64,
    pub target_commit: String,
    pub stage: CoreMaintenanceStage,
    pub failure: Option<CoreMaintenanceFailure>,
}
impl From<rpc::CoreMaintenanceStatus> for CoreMaintenanceStatus {
    fn from(v: rpc::CoreMaintenanceStatus) -> Self {
        Self {
            operation: v.operation.0,
            target_commit: v.target_commit,
            stage: v.stage.into(),
            failure: v.failure.map(Into::into),
        }
    }
}

/// A signed APK target is a local byte adapter, never permission to install.
/// Core repeats authentication, anti-rollback, namespace and real-byte checks.
#[uniffi::export]
pub fn core_replacement_target(descriptor: Vec<u8>) -> Option<CoreReplacementTarget> {
    let (trust, minimum) = vpnhide_package_trust::TrustStore::compiled_install_authority().ok()?;
    let verified = trust.authenticate(&descriptor).ok()?;
    verified.require_build_at_least(minimum.get()).ok()?;
    let d = verified.descriptor();
    Some(CoreReplacementTarget {
        package_bytes: d.package_bytes,
        request: CoreReplacementRequest {
            descriptor,
            expected_commit: d.source_commit.clone(),
            expected_package_sha256: d.package_sha256.clone(),
            target_rpc: d.rpc,
        },
    })
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreReplacementTarget {
    pub request: CoreReplacementRequest,
    pub package_bytes: u64,
}

#[uniffi::export]
pub fn core_replacement_envelope_limit() -> u32 {
    vpnhide_package_trust::MAX_ENVELOPE_BYTES as u32
}
