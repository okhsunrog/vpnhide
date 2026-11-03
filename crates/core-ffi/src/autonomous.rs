//! Typed unprivileged worker observations and the same Core-owned run view used
//! by UI/export/agent callers. Kotlin neither evaluates evidence nor writes fixtures.
use crate::{CoreError, CoreSession};
use vpnhide_diagnostic_model as model;

macro_rules! enum_map {
    ($name:ident, $source:ident, $($variant:ident),+ $(,)?) => {
        #[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
        pub enum $name { $($variant),+ }
        impl From<model::$source> for $name { fn from(v: model::$source) -> Self { match v { $(model::$source::$variant => Self::$variant),+ } } }
        impl From<$name> for model::$source { fn from(v: $name) -> Self { match v { $($name::$variant => Self::$variant),+ } } }
    }
}
enum_map!(CoreNativeReadActor, NativeReadActor, Libc, Syscall);
mod native_raw;
pub use native_raw::*;
mod address;
pub use address::*;
mod proc_interfaces;
pub use proc_interfaces::*;
mod sys_interfaces;
pub use sys_interfaces::*;
enum_map!(
    CoreRequiredCheck,
    RequiredCheck,
    IoctlFlags,
    IoctlMtu,
    Getifaddrs,
    NetlinkLinks,
    IoctlAddresses,
    BindToDevice,
    NetlinkAddresses,
    NetlinkRoutes,
    NetlinkRules,
    ProcRoutes,
    ProcInterfaces,
    SysInterfaces,
    FrameworkNetworks,
    FrameworkDefaultNetwork,
    FrameworkCapabilities,
    FrameworkLinkProperties,
    FrameworkLegacyInfo,
    FrameworkCallbacks,
    FrameworkPendingIntent,
    PackageVisibility,
    Ports,
    Filesystem
);
enum_map!(
    CoreNativeCheck,
    NativeCheck,
    IoctlFlags,
    IoctlMtu,
    Getifaddrs,
    NetlinkGetlink
);
enum_map!(
    CoreControlFailure,
    ControlFailure,
    InvalidSpec,
    MissingNegativeControl,
    MissingPositiveControl,
    FixtureChanged,
    OrdinaryObjectsChanged,
    ProbeUnavailable,
    CleanupUnconfirmed
);

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreProbeError {
    Errno { code: i32 },
    Timeout,
    Malformed,
    TooLarge,
    Unsupported,
}
impl From<model::ProbeError> for CoreProbeError {
    fn from(v: model::ProbeError) -> Self {
        match v {
            model::ProbeError::Errno(code) => Self::Errno { code },
            model::ProbeError::Timeout => Self::Timeout,
            model::ProbeError::Malformed => Self::Malformed,
            model::ProbeError::TooLarge => Self::TooLarge,
            model::ProbeError::Unsupported => Self::Unsupported,
        }
    }
}
impl From<CoreProbeError> for model::ProbeError {
    fn from(v: CoreProbeError) -> Self {
        match v {
            CoreProbeError::Errno { code } => Self::Errno(code),
            CoreProbeError::Timeout => Self::Timeout,
            CoreProbeError::Malformed => Self::Malformed,
            CoreProbeError::TooLarge => Self::TooLarge,
            CoreProbeError::Unsupported => Self::Unsupported,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreIfaddrMetadata {
    pub families: Vec<u16>,
    pub flags: Vec<u32>,
    pub has_null_address: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreObservedInterface {
    pub name: String,
    pub index: Option<u32>,
    pub ifaddrs: Option<CoreIfaddrMetadata>,
}
impl From<model::ObservedInterface> for CoreObservedInterface {
    fn from(v: model::ObservedInterface) -> Self {
        Self {
            name: v.name,
            index: v.index,
            ifaddrs: v.ifaddrs.map(|m| CoreIfaddrMetadata {
                families: m.families,
                flags: m.flags,
                has_null_address: m.has_null_address,
            }),
        }
    }
}
impl From<CoreObservedInterface> for model::ObservedInterface {
    fn from(v: CoreObservedInterface) -> Self {
        Self {
            name: v.name,
            index: v.index,
            ifaddrs: v.ifaddrs.map(|m| model::IfaddrMetadata {
                families: m.families,
                flags: m.flags,
                has_null_address: m.has_null_address,
            }),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreNumberSample {
    Value { value: u32 },
    Error { error: CoreProbeError },
}
impl From<model::Sample<u32>> for CoreNumberSample {
    fn from(v: model::Sample<u32>) -> Self {
        match v {
            model::Sample::Value(value) => Self::Value { value },
            model::Sample::Error(e) => Self::Error { error: e.into() },
        }
    }
}
impl From<CoreNumberSample> for model::Sample<u32> {
    fn from(v: CoreNumberSample) -> Self {
        match v {
            CoreNumberSample::Value { value } => Self::Value(value),
            CoreNumberSample::Error { error } => Self::Error(error.into()),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreInterfacesSample {
    Value {
        interfaces: Vec<CoreObservedInterface>,
    },
    Error {
        error: CoreProbeError,
    },
}
impl From<model::Sample<Vec<model::ObservedInterface>>> for CoreInterfacesSample {
    fn from(v: model::Sample<Vec<model::ObservedInterface>>) -> Self {
        match v {
            model::Sample::Value(v) => Self::Value {
                interfaces: v.into_iter().map(Into::into).collect(),
            },
            model::Sample::Error(e) => Self::Error { error: e.into() },
        }
    }
}
impl From<CoreInterfacesSample> for model::Sample<Vec<model::ObservedInterface>> {
    fn from(v: CoreInterfacesSample) -> Self {
        match v {
            CoreInterfacesSample::Value { interfaces } => {
                Self::Value(interfaces.into_iter().map(Into::into).collect())
            }
            CoreInterfacesSample::Error { error } => Self::Error(error.into()),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CoreBindActor {
    Libc,
    Syscall,
}
impl From<model::BindActor> for CoreBindActor {
    fn from(v: model::BindActor) -> Self {
        match v {
            model::BindActor::Libc => Self::Libc,
            model::BindActor::Syscall => Self::Syscall,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreBindAttempt {
    pub initial: crate::CoreTextSample,
    pub set: CoreNumberSample,
    pub final_device: crate::CoreTextSample,
}
impl From<model::BindAttempt> for CoreBindAttempt {
    fn from(v: model::BindAttempt) -> Self {
        Self {
            initial: v.initial.into(),
            set: v.set.into(),
            final_device: v.final_device.into(),
        }
    }
}
impl From<CoreBindAttempt> for model::BindAttempt {
    fn from(v: CoreBindAttempt) -> Self {
        Self {
            initial: v.initial.into(),
            set: v.set.into(),
            final_device: v.final_device.into(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreBindMeasurement {
    pub libc_fixture: CoreBindAttempt,
    pub libc_ordinary: CoreBindAttempt,
    pub syscall_fixture: CoreBindAttempt,
    pub syscall_ordinary: CoreBindAttempt,
}
impl From<model::BindMeasurement> for CoreBindMeasurement {
    fn from(v: model::BindMeasurement) -> Self {
        Self {
            libc_fixture: v.libc_fixture.into(),
            libc_ordinary: v.libc_ordinary.into(),
            syscall_fixture: v.syscall_fixture.into(),
            syscall_ordinary: v.syscall_ordinary.into(),
        }
    }
}
impl From<CoreBindMeasurement> for model::BindMeasurement {
    fn from(v: CoreBindMeasurement) -> Self {
        Self {
            libc_fixture: v.libc_fixture.into(),
            libc_ordinary: v.libc_ordinary.into(),
            syscall_fixture: v.syscall_fixture.into(),
            syscall_ordinary: v.syscall_ordinary.into(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreScopedBindResult {
    pub proven: Vec<CoreBindActor>,
    pub leaked: Vec<CoreBindActor>,
    pub controls: Vec<CoreControlFailure>,
}
impl From<model::ScopedBindResult> for CoreScopedBindResult {
    fn from(v: model::ScopedBindResult) -> Self {
        Self {
            proven: v.proven.into_iter().map(Into::into).collect(),
            leaked: v.leaked.into_iter().map(Into::into).collect(),
            controls: v.controls.into_iter().map(Into::into).collect(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreNativeMeasurement {
    pub sys_interfaces: Option<CoreSysInterfaceMeasurement>,
    pub proc_interfaces: Option<CoreProcInterfaceMeasurement>,
    pub addresses: Option<CoreAddressMeasurement>,
    pub binding: Option<CoreBindMeasurement>,
    pub flags: CoreNumberSample,
    pub mtu: CoreNumberSample,
    pub ifaddrs: CoreInterfacesSample,
    pub links: CoreInterfacesSample,
    pub ordinary_flags: CoreNumberSample,
    pub ordinary_mtu: CoreNumberSample,
}
impl From<model::NativeMeasurement> for CoreNativeMeasurement {
    fn from(v: model::NativeMeasurement) -> Self {
        Self {
            sys_interfaces: v.sys_interfaces.map(Into::into),
            proc_interfaces: v.proc_interfaces.map(Into::into),
            addresses: v.addresses.map(Into::into),
            binding: v.binding.map(Into::into),
            flags: v.flags.into(),
            mtu: v.mtu.into(),
            ifaddrs: v.ifaddrs.into(),
            links: v.links.into(),
            ordinary_flags: v.ordinary_flags.into(),
            ordinary_mtu: v.ordinary_mtu.into(),
        }
    }
}
impl From<CoreNativeMeasurement> for model::NativeMeasurement {
    fn from(v: CoreNativeMeasurement) -> Self {
        Self {
            sys_interfaces: v.sys_interfaces.map(Into::into),
            proc_interfaces: v.proc_interfaces.map(Into::into),
            addresses: v.addresses.map(Into::into),
            binding: v.binding.map(Into::into),
            flags: v.flags.into(),
            mtu: v.mtu.into(),
            ifaddrs: v.ifaddrs.into(),
            links: v.links.into(),
            ordinary_flags: v.ordinary_flags.into(),
            ordinary_mtu: v.ordinary_mtu.into(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreDiagnosticWorkerTask {
    pub session: u64,
    pub core_epoch: u64,
    pub interface_name: String,
}
impl From<model::DiagnosticWorkerTask> for CoreDiagnosticWorkerTask {
    fn from(v: model::DiagnosticWorkerTask) -> Self {
        Self {
            session: v.session,
            core_epoch: v.core_epoch,
            interface_name: v.fixture.interface.name,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreScopedNativeResult {
    Proven {
        checks: Vec<CoreNativeCheck>,
    },
    Leaked {
        checks: Vec<CoreNativeCheck>,
        incomplete: Vec<CoreControlFailure>,
    },
    Unverified {
        reason: CoreControlFailure,
        observed_leaks: Vec<CoreNativeCheck>,
    },
}
impl From<model::ScopedNativeResult> for CoreScopedNativeResult {
    fn from(v: model::ScopedNativeResult) -> Self {
        match v {
            model::ScopedNativeResult::Proven { checks } => Self::Proven {
                checks: checks.into_iter().map(Into::into).collect(),
            },
            model::ScopedNativeResult::Leaked { checks, incomplete } => Self::Leaked {
                checks: checks.into_iter().map(Into::into).collect(),
                incomplete: incomplete.into_iter().map(Into::into).collect(),
            },
            model::ScopedNativeResult::Unverified {
                reason,
                observed_leaks,
            } => Self::Unverified {
                reason: reason.into(),
                observed_leaks: observed_leaks.into_iter().map(Into::into).collect(),
            },
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreDiagnosticOutcome {
    Running,
    Incomplete {
        native: CoreScopedNativeResult,
        missing: Vec<CoreRequiredCheck>,
    },
    Infrastructure {
        detail: String,
        native: Option<CoreScopedNativeResult>,
        cleanup_confirmed: Option<bool>,
    },
}
enum_map!(
    CoreAssessmentPhase,
    AssessmentPhase,
    Running,
    Incomplete,
    Infrastructure
);
enum_map!(
    CoreReuseBlocker,
    ReuseBlocker,
    ConditionsChanged,
    LoadedGenerationsIncomplete,
    PlatformUnknown,
    LoadedGenerationsUnknown,
    InvalidFingerprint,
    UnsupportedPlan
);

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreDiagnosticAssessment {
    pub phase: CoreAssessmentPhase,
    pub required: Vec<CoreRequiredCheck>,
    pub proven: Vec<CoreRequiredCheck>,
    pub leaked: Vec<CoreRequiredCheck>,
    pub unverified: Vec<CoreRequiredCheck>,
    pub native_controls: Vec<CoreControlFailure>,
    pub framework_controls: Vec<crate::CoreFrameworkControlFailure>,
    pub reuse_blockers: Vec<CoreReuseBlocker>,
    pub infrastructure_detail: Option<String>,
    pub cleanup_confirmed: Option<bool>,
}
impl From<model::DiagnosticAssessment> for CoreDiagnosticAssessment {
    fn from(v: model::DiagnosticAssessment) -> Self {
        Self {
            phase: v.phase.into(),
            required: v.required.into_iter().map(Into::into).collect(),
            proven: v.proven.into_iter().map(Into::into).collect(),
            leaked: v.leaked.into_iter().map(Into::into).collect(),
            unverified: v.unverified.into_iter().map(Into::into).collect(),
            native_controls: v.native_controls.into_iter().map(Into::into).collect(),
            framework_controls: v.framework_controls.into_iter().map(Into::into).collect(),
            reuse_blockers: v.reuse_blockers.into_iter().map(Into::into).collect(),
            infrastructure_detail: v.infrastructure_detail,
            cleanup_confirmed: v.cleanup_confirmed,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreDiagnosticRun {
    pub sys_interfaces: Option<CoreScopedSysInterfaceResult>,
    pub native_raw: Option<CoreNativeRawReceipt>,
    pub session: u64,
    pub core_epoch: u64,
    pub boot_id: String,
    pub policy_sha256: String,
    pub runtime_sha256: String,
    pub platform_sha256: Option<String>,
    pub loaded_generations_sha256: Option<String>,
    pub loaded_generations_complete: bool,
    pub contract: u32,
    pub required: Vec<CoreRequiredCheck>,
    pub outcome: CoreDiagnosticOutcome,
    pub framework: Option<crate::CoreScopedFrameworkResult>,
    pub stale: bool,
    pub binding: Option<CoreScopedBindResult>,
    pub addresses: Option<CoreScopedAddressResult>,
    pub proc_interfaces: Option<CoreScopedProcInterfaceResult>,
}
impl From<model::DiagnosticRun> for CoreDiagnosticRun {
    fn from(v: model::DiagnosticRun) -> Self {
        let outcome = match v.outcome {
            model::DiagnosticOutcome::Running => CoreDiagnosticOutcome::Running,
            model::DiagnosticOutcome::Incomplete { native, missing } => {
                CoreDiagnosticOutcome::Incomplete {
                    native: native.into(),
                    missing: missing.into_iter().map(Into::into).collect(),
                }
            }
            model::DiagnosticOutcome::Infrastructure {
                detail,
                native,
                cleanup_confirmed,
            } => CoreDiagnosticOutcome::Infrastructure {
                detail,
                native: native.map(Into::into),
                cleanup_confirmed,
            },
        };
        Self {
            native_raw: v.native_raw.map(Into::into),
            sys_interfaces: v.sys_interfaces.map(Into::into),
            proc_interfaces: v.proc_interfaces.map(Into::into),
            addresses: v.addresses.map(Into::into),
            session: v.session,
            core_epoch: v.fingerprint.core_epoch,
            boot_id: v.fingerprint.boot_id,
            policy_sha256: v.fingerprint.policy_sha256,
            runtime_sha256: v.fingerprint.runtime_sha256,
            platform_sha256: v.fingerprint.platform_sha256,
            loaded_generations_sha256: v.fingerprint.loaded_generations_sha256,
            loaded_generations_complete: v.fingerprint.loaded_generations_complete,
            contract: v.plan.contract,
            required: v.plan.required.into_iter().map(Into::into).collect(),
            outcome,
            binding: v.binding.map(Into::into),
            framework: v.framework.map(Into::into),
            stale: v.stale,
        }
    }
}
enum_map!(
    CoreDiagnosticRefusal,
    DiagnosticError,
    Disabled,
    Busy,
    WrongProcess,
    Expired,
    StaleSession,
    Malformed
);
#[derive(Debug, uniffi::Error)]
pub enum CoreDiagnosticError {
    Transport { error: CoreError },
    Refused { reason: CoreDiagnosticRefusal },
}
impl std::fmt::Display for CoreDiagnosticError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for CoreDiagnosticError {}
impl From<vpnhide_core_client::ClientError> for CoreDiagnosticError {
    fn from(error: vpnhide_core_client::ClientError) -> Self {
        Self::Transport {
            error: error.into(),
        }
    }
}
impl From<model::DiagnosticError> for CoreDiagnosticError {
    fn from(reason: model::DiagnosticError) -> Self {
        Self::Refused {
            reason: reason.into(),
        }
    }
}

#[uniffi::export]
pub fn observe_diagnostic_fixture(
    task: CoreDiagnosticWorkerTask,
) -> Result<CoreNativeMeasurement, CoreError> {
    let expected = format!("tun{:012x}", task.session & 0xffff_ffff_ffff);
    if task.session == 0 || task.core_epoch == 0 || task.interface_name != expected {
        return Err(CoreError::InvalidInput {
            detail: "invalid diagnostic worker task".into(),
        });
    }
    Ok(vpnhide_checks::fixture::observe(&task.interface_name).into())
}
#[uniffi::export(async_runtime = "tokio")]
impl CoreSession {
    pub async fn request_diagnostic_run(
        &self,
        force: bool,
    ) -> Result<CoreDiagnosticRun, CoreDiagnosticError> {
        self.client
            .request_diagnostic_run(force)
            .await?
            .map(Into::into)
            .map_err(Into::into)
    }
    pub async fn join_diagnostic_worker(
        &self,
    ) -> Result<Option<CoreDiagnosticWorkerTask>, CoreDiagnosticError> {
        self.client
            .join_diagnostic_worker()
            .await?
            .map(|t| t.map(Into::into))
            .map_err(Into::into)
    }
    pub async fn report_diagnostic_worker(
        &self,
        task: CoreDiagnosticWorkerTask,
        native: CoreNativeMeasurement,
    ) -> Result<(), CoreDiagnosticError> {
        self.client
            .report_diagnostic_worker(model::DiagnosticWorkerReport {
                session: task.session,
                core_epoch: task.core_epoch,
                native: native.into(),
            })
            .await?
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_boundary_preserves_real_observations_and_rejects_unbound_tasks() {
        let measurement = vpnhide_checks::fixture::observe("tun000000000001");
        assert!(measurement.bounded());
        let app: CoreNativeMeasurement = measurement.clone().into();
        assert_eq!(model::NativeMeasurement::from(app), measurement);
        for (session, epoch, name) in [
            (0, 1, "tun000000000000"),
            (1, 0, "tun000000000001"),
            (1, 2, "/proc/net/route"),
            (1, 2, "tun000000000002"),
        ] {
            assert!(
                observe_diagnostic_fixture(CoreDiagnosticWorkerTask {
                    session,
                    core_epoch: epoch,
                    interface_name: name.into()
                })
                .is_err()
            );
        }
    }
}
