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
mod filesystem;
pub use filesystem::*;
mod routes;
pub use routes::*;
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
    CleanupUnconfirmed,
    NetlinkLinkRequestDenied,
    ProcRouteRequestDenied,
    ProcInterfaceRequestDenied
);
enum_map!(CoreProcReadStage, ProcReadStage, Open, Read);

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreProbeError {
    Errno { code: i32 },
    GetlinkSendDenied { code: i32 },
    ProcPermissionDenied { stage: CoreProcReadStage, code: i32 },
    Timeout,
    Malformed,
    TooLarge,
    Unsupported,
}
impl From<model::ProbeError> for CoreProbeError {
    fn from(v: model::ProbeError) -> Self {
        match v {
            model::ProbeError::Errno(code) => Self::Errno { code },
            model::ProbeError::GetlinkSendDenied(code) => Self::GetlinkSendDenied { code },
            model::ProbeError::ProcPermissionDenied(denial) => Self::ProcPermissionDenied {
                stage: denial.stage.into(),
                code: denial.errno,
            },
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
            CoreProbeError::GetlinkSendDenied { code } => Self::GetlinkSendDenied(code),
            CoreProbeError::ProcPermissionDenied { stage, code } => {
                Self::ProcPermissionDenied(model::ProcReadDenial {
                    stage: stage.into(),
                    errno: code,
                })
            }
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
    pub routes: Option<CoreRouteMeasurement>,
    pub filesystem: Option<CoreFilesystemMeasurement>,
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
            routes: v.routes.map(Into::into),
            filesystem: v.filesystem.map(Into::into),
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
            routes: v.routes.map(Into::into),
            filesystem: v.filesystem.map(Into::into),
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
    pub routes: Option<CoreRouteFixtureSpec>,
}
impl From<model::DiagnosticWorkerTask> for CoreDiagnosticWorkerTask {
    fn from(v: model::DiagnosticWorkerTask) -> Self {
        Self {
            session: v.session,
            core_epoch: v.core_epoch,
            interface_name: v.fixture.interface.name,
            routes: v.routes.map(Into::into),
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
    Finished {
        native: CoreScopedNativeResult,
    },
    Infrastructure {
        failure: CoreDiagnosticFailure,
        native: Option<CoreScopedNativeResult>,
        cleanup_confirmed: Option<bool>,
    },
}
enum_map!(
    CoreAssessmentPhase,
    AssessmentPhase,
    Running,
    Finished,
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

enum_map!(
    CoreDiagnosticStage,
    DiagnosticStage,
    Coordinator,
    NativeFixture,
    WorkerStart,
    NativeMeasurement,
    FrameworkAndPorts,
    Cleanup
);
enum_map!(
    CoreDiagnosticFailureKind,
    DiagnosticFailureKind,
    Timeout,
    Busy,
    Cancelled,
    Unavailable,
    CleanupUnconfirmed
);
enum_map!(
    CoreDiagnosticVerdict,
    DiagnosticVerdict,
    Proven,
    Leaked,
    Unverified
);

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreDiagnosticFailure {
    pub stage: CoreDiagnosticStage,
    pub kind: CoreDiagnosticFailureKind,
    pub detail: String,
}

impl From<model::DiagnosticFailure> for CoreDiagnosticFailure {
    fn from(v: model::DiagnosticFailure) -> Self {
        Self {
            stage: v.stage.into(),
            kind: v.kind.into(),
            detail: v.detail,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreUnverifiedReason {
    AwaitingEvidence,
    EvidenceUnavailable,
    AndroidAccessDenied,
    NativeControls {
        controls: Vec<CoreControlFailure>,
    },
    FrameworkControls {
        failures: Vec<crate::CoreFrameworkFailure>,
    },
    Interrupted {
        stage: CoreDiagnosticStage,
        failure: CoreDiagnosticFailureKind,
    },
}

impl From<model::UnverifiedReason> for CoreUnverifiedReason {
    fn from(v: model::UnverifiedReason) -> Self {
        match v {
            model::UnverifiedReason::AwaitingEvidence => Self::AwaitingEvidence,
            model::UnverifiedReason::EvidenceUnavailable => Self::EvidenceUnavailable,
            model::UnverifiedReason::AndroidAccessDenied => Self::AndroidAccessDenied,
            model::UnverifiedReason::NativeControls { controls } => Self::NativeControls {
                controls: controls.into_iter().map(Into::into).collect(),
            },
            model::UnverifiedReason::FrameworkControls { failures } => Self::FrameworkControls {
                failures: failures.into_iter().map(Into::into).collect(),
            },
            model::UnverifiedReason::Interrupted { stage, failure } => Self::Interrupted {
                stage: stage.into(),
                failure: failure.into(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreCheckOutcome {
    Proven,
    Leaked,
    Unverified { reason: CoreUnverifiedReason },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreCheckAssessment {
    pub check: CoreRequiredCheck,
    pub outcome: CoreCheckOutcome,
    pub android_access_denied: bool,
}

impl From<model::CheckAssessment> for CoreCheckAssessment {
    fn from(v: model::CheckAssessment) -> Self {
        Self {
            check: v.check.into(),
            android_access_denied: v.android_access_denied,
            outcome: match v.outcome {
                model::CheckOutcome::Proven => CoreCheckOutcome::Proven,
                model::CheckOutcome::Leaked => CoreCheckOutcome::Leaked,
                model::CheckOutcome::Unverified { reason } => CoreCheckOutcome::Unverified {
                    reason: reason.into(),
                },
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreDiagnosticAssessment {
    pub phase: CoreAssessmentPhase,
    pub checks: Vec<CoreCheckAssessment>,
    pub verdict: CoreDiagnosticVerdict,
    pub native_controls: Vec<CoreControlFailure>,
    pub framework_controls: Vec<crate::CoreFrameworkControlFailure>,
    pub reuse_blockers: Vec<CoreReuseBlocker>,
    pub failure: Option<CoreDiagnosticFailure>,
    pub cleanup_confirmed: Option<bool>,
}
impl From<model::DiagnosticAssessment> for CoreDiagnosticAssessment {
    fn from(v: model::DiagnosticAssessment) -> Self {
        Self {
            phase: v.phase.into(),
            verdict: v.verdict().into(),
            checks: v.checks.into_iter().map(Into::into).collect(),
            native_controls: v.native_controls.into_iter().map(Into::into).collect(),
            framework_controls: v.framework_controls.into_iter().map(Into::into).collect(),
            reuse_blockers: v.reuse_blockers.into_iter().map(Into::into).collect(),
            failure: v.failure.map(Into::into),
            cleanup_confirmed: v.cleanup_confirmed,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreDiagnosticRun {
    pub routes: Option<CoreScopedRouteResult>,
    pub filesystem: Option<CoreScopedFilesystemResult>,
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
            model::DiagnosticOutcome::Finished { native } => CoreDiagnosticOutcome::Finished {
                native: native.into(),
            },
            model::DiagnosticOutcome::Infrastructure {
                failure,
                native,
                cleanup_confirmed,
            } => CoreDiagnosticOutcome::Infrastructure {
                failure: failure.into(),
                native: native.map(Into::into),
                cleanup_confirmed,
            },
        };
        Self {
            native_raw: v.native_raw.map(Into::into),
            routes: v.routes.map(Into::into),
            filesystem: v.filesystem.map(Into::into),
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
    let routes = task.routes.map(model::RouteFixtureSpec::from);
    if routes
        .as_ref()
        .is_some_and(|spec| spec.session != task.session || !spec.valid())
    {
        return Err(CoreError::InvalidInput {
            detail: "invalid route fixture in worker task".into(),
        });
    }
    let route_observation = routes
        .as_ref()
        .map(|spec| vpnhide_checks::route::observe_worker(spec.rule_uid.is_some()).scoped(spec));
    let mut observation = vpnhide_checks::fixture::observe(&task.interface_name);
    observation.routes = route_observation;
    Ok(bound_route_worker_report(observation).into())
}

fn bound_route_worker_report(
    mut observation: model::NativeMeasurement,
) -> model::NativeMeasurement {
    // The largest report travels in either the protected-worker RPC or its
    // RAW phase envelope. Leave room for tarpc framing and RPC metadata.
    const BUDGET: usize = vpnhide_core_rpc::MAX_FRAME_BYTES - 4096;
    if observation.routes.is_some()
        && serde_json::to_vec(&observation).is_ok_and(|bytes| bytes.len() > BUDGET)
    {
        // Preserve the scoped route rows. The other measurements were too
        // large to transport together and cannot qualify as empty samples.
        return model::NativeMeasurement {
            routes: observation.routes.take(),
            ..model::NativeMeasurement::unavailable(model::ProbeError::TooLarge)
        };
    }
    observation
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
    fn issued_route_task_survives_the_typed_worker_boundary() {
        let spec = model::RouteFixtureSpec {
            rule_uid: Some(10_000),
            session: 1,
            vpn: model::InterfaceId {
                name: "tun000000004001".into(),
                index: 86_001,
            },
            ordinary: model::InterfaceId {
                name: "vhr000000008001".into(),
                index: 86_002,
            },
        };
        let task = CoreDiagnosticWorkerTask {
            session: 1,
            core_epoch: 2,
            interface_name: "tun000000000001".into(),
            routes: Some(spec.clone().into()),
        };
        let mut measured =
            model::NativeMeasurement::from(observe_diagnostic_fixture(task.clone()).unwrap());
        assert!(measured.bounded_for_routes(Some(&spec)));
        let rule_view = measured
            .routes
            .as_mut()
            .unwrap()
            .observations
            .iter_mut()
            .find(|view| view.check.surface.rule())
            .expect("worker collected a rule view");
        let role = model::RuleRole::WorkerUid;
        rule_view.rules.push(model::ObservedRule {
            priority: Some(role.priority(&spec)),
            goto: None,
            output_name: None,
            uid_range: Some([10_000; 2]),
            table: role.table(),
            protocol: Some(model::DIAGNOSTIC_RULE_PROTOCOL),
            action: model::DIAGNOSTIC_RULE_ACTION,
            flags: 0,
            additional_selectors: false,
        });
        assert_eq!(
            model::NativeMeasurement::from(CoreNativeMeasurement::from(measured.clone())),
            measured
        );
        assert!(measured.bounded_for_routes(Some(&spec)));
        assert!(!measured.bounded_for_routes(None));
        let mut wrong = task;
        wrong.routes.as_mut().unwrap().session = 2;
        assert!(observe_diagnostic_fixture(wrong).is_err());

        let mut oversized = measured.clone();
        oversized.links = model::Sample::Value(vec![model::ObservedInterface {
            name: "x".repeat(vpnhide_core_rpc::MAX_FRAME_BYTES),
            index: None,
            ifaddrs: None,
        }]);
        let compact = bound_route_worker_report(oversized);
        assert_eq!(compact.routes, measured.routes);
        assert_eq!(
            compact.flags,
            model::Sample::Error(model::ProbeError::TooLarge)
        );
        assert!(compact.bounded_for_routes(Some(&spec)));
    }
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
                    routes: None,
                    session,
                    core_epoch: epoch,
                    interface_name: name.into()
                })
                .is_err()
            );
        }
    }
}
