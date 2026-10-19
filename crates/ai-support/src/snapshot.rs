//! Compact diagnostic snapshot the APK sends with a support question.
//!
//! This is a projection of the debug bundle, not a second copy of it. The
//! bundle (`VpnHideState`, schema in `debug/VpnHideState.kt`) carries logcat,
//! dmesg, raw shell sections and the canonical config. Those stay in the
//! private archive. What crosses this wire is the typed state a triage needs
//! in order to tell a setup problem from a measured leak.
//!
//! Field names here are snake_case. Kotlin kotlinx names are called out on
//! the types they project. The assistant schema is not `VPNHIDE_STATE_SCHEMA`:
//! the bundle can grow without this contract moving, and the other way around.
//!
//! No design note named "AI Support, диагностика и GitHub Issue Gateway" is
//! checked in under `docs/`. The shape below follows `docs/diagnostics.md`,
//! `docs/debug-bundle.md`, and the Kotlin types those documents point at.

use serde::{Deserialize, Serialize};

/// Assistant snapshot schema. Bump when a field is removed, renamed, or changes
/// meaning. Adding a field does not require a bump; readers use `#[serde(default)]`
/// on anything that may be absent.
pub const ASSISTANT_SNAPSHOT_SCHEMA: u32 = 1;

/// `VpnHideState.captureKind`. Anything else is a different export.
pub const CAPTURE_KINDS: [&str; 3] = ["debug", "full_system_logcat", "kernel_images"];

const MAX_SHORT: usize = 128;
const MAX_ERRORS: usize = 20;
const MAX_ERROR_CHARS: usize = 200;
const MAX_CHECKS: usize = 64;
const MAX_ID_CHARS: usize = 80;
const MAX_HOOKS: usize = 32;
const MAX_ABIS: usize = 4;

/// Why a snapshot was refused before it reaches a model or an issue draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotError {
    Schema,
    GeneratedAt,
    CaptureKind,
    App,
    Device,
    RootShell,
    ActiveBackend,
    VerdictWithoutMeasurement,
    GateMismatch,
    Errors,
    Checks,
    Summary,
    KmodLoad,
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let reason = match self {
            Self::Schema => "unsupported snapshot schema",
            Self::GeneratedAt => "generated_at is missing or not a single line",
            Self::CaptureKind => "capture_kind is not a known export kind",
            Self::App => "app identity is missing or not a single line",
            Self::Device => "device identity is missing or not a single line",
            Self::RootShell => "root_shell is not a single line",
            Self::ActiveBackend => "active_backend id and state disagree",
            Self::VerdictWithoutMeasurement => "a verdict requires gate routed",
            Self::GateMismatch => "report.gate and the top-level gate disagree",
            Self::Errors => "errors exceed the compact snapshot cap",
            Self::Checks => "a check list exceeds the compact snapshot cap",
            Self::Summary => "diagnostics summary is internally inconsistent",
            Self::KmodLoad => "kmod_load text is not a single line",
        };
        f.write_str(reason)
    }
}

impl std::error::Error for SnapshotError {}

/// Top-level body of `POST /api/v1/assistant/chat` and the issue-draft stub.
///
/// `version` selects the source checkout. It is the version the user is
/// running, and it is not taken from the snapshot: a snapshot can describe a
/// build whose tree this server does not have.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantRequest {
    pub version: String,
    pub snapshot: DiagnosticSnapshot,
    pub question: String,
    /// BCP-47 hint for the reply language. The server still owns the prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
}

/// Projection of `VpnHideState` plus the measured `DiagnosticReport`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticSnapshot {
    pub schema: u32,
    /// `VpnHideState.schema`, when the client still knows which bundle shape
    /// it projected. Absent is fine; this wire does not parse `state.json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_schema: Option<u32>,
    /// `VpnHideState.generatedAt` (ISO-8601, stamped by the app).
    pub generated_at: String,
    /// `VpnHideState.captureKind`.
    pub capture_kind: String,
    /// `VpnHideState.app`.
    pub app: AppInfo,
    /// `VpnHideState.device`.
    pub device: DeviceInfo,
    /// `VpnHideState.selfNeedsRestart`.
    pub self_needs_restart: bool,
    /// `VpnHideState.gate`. Null when the capture did not run the suite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<DiagnosticGate>,
    /// `VpnHideState.selfTestRunId`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub self_test_run_id: Option<i64>,
    /// Compact `VpnHideState.diagnostics` (`DiagnosticSummaryInfo`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<DiagnosticSummary>,
    /// `VpnHideState.nativeVerdict`. Null unless the run was measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_verdict: Option<Verdict>,
    /// `VpnHideState.javaVerdict`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub java_verdict: Option<Verdict>,
    /// Compact `VpnHideState.report`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<DiagnosticReport>,
    /// `VpnHideState.backends`, presence only.
    pub backends: BackendSet,
    /// `VpnHideState.activeBackend`.
    pub active_backend: ActiveBackend,
    /// `VpnHideState.ports`.
    pub ports: ModulePresence,
    /// `VpnHideState.kmodLoadStatus` without `dmesgTail` or `insmodStderr`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kmod_load: Option<KmodLoad>,
    /// `VpnHideState.rootShell` without the raw `id` line.
    pub root_shell: RootShell,
    /// `VpnHideState.errors`, already the short list. Capped again on ingest.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

impl DiagnosticSnapshot {
    /// `DiagnosticGate.ROUTED` is the only gate that measured anything.
    /// A report has to agree; a capture with no report did not measure.
    pub fn measurement_is_meaningful(&self) -> bool {
        self.gate == Some(DiagnosticGate::Routed)
            && self
                .report
                .as_ref()
                .is_some_and(|report| report.gate == DiagnosticGate::Routed)
    }

    /// The only reading that may be described as a successful measurement.
    ///
    /// `currentSuccess` is the app's positive claim (`DiagnosticSummaryInfo`).
    /// It counts only together with a routed run, a measurement that still
    /// applies, and an evidence conclusion of no owned leak. A true flag next
    /// to any other conclusion is an inconsistent snapshot, not a success.
    pub fn allows_positive_claim(&self) -> bool {
        let Some(summary) = &self.diagnostics else {
            return false;
        };
        self.measurement_is_meaningful()
            && summary.current_success
            && !summary.probe_unavailable
            && summary.applicability == MeasurementApplicability::MatchesLastObservation
            && summary
                .evidence
                .as_ref()
                .is_some_and(|evidence| evidence.conclusion == EvidenceConclusion::NoObservedLeak)
    }
}

/// `VpnHideState.app`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppInfo {
    pub package_name: String,
    pub version: String,
}

/// `VpnHideState.device`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceInfo {
    pub manufacturer: String,
    pub model: String,
    pub android_release: String,
    pub sdk: u32,
    pub abis: Vec<String>,
}

/// `DiagnosticGate`. Only `routed` produces verdicts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticGate {
    VpnOff,
    SelfNotRouted,
    NeedsRestart,
    Routed,
}

impl DiagnosticGate {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::VpnOff => "vpn_off",
            Self::SelfNotRouted => "self_not_routed",
            Self::NeedsRestart => "needs_restart",
            Self::Routed => "routed",
        }
    }
}

/// `Verdict`. Serialized in snake_case (`ok`, `partial`, `broken`), not the
/// Kotlin enum spelling (`Ok`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Ok,
    Partial,
    Broken,
}

impl Verdict {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Partial => "partial",
            Self::Broken => "broken",
        }
    }
}

/// `CheckLayer`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckLayer {
    Native,
    Java,
}

/// `CheckOutcome`. `kind` matches the bundle discriminator. Reasons match the
/// wire tokens in `docs/diagnostics.md` (`not_measured_no_network` and the
/// rest) with the shared `not_measured_` prefix dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CheckOutcome {
    Leak,
    HiddenBackend,
    HiddenSelinux,
    NothingToLeak,
    NotMeasured { reason: NotMeasuredReason },
}

impl CheckOutcome {
    pub const fn kind(self) -> &'static str {
        match self {
            Self::Leak => "leak",
            Self::HiddenBackend => "hidden_backend",
            Self::HiddenSelinux => "hidden_selinux",
            Self::NothingToLeak => "nothing_to_leak",
            Self::NotMeasured { .. } => "not_measured",
        }
    }
}

/// `NotMeasuredReason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotMeasuredReason {
    NoNetwork,
    NoGroundTruth,
    UnknownNativeStatus,
}

/// `LayerStatus`. Presence is decided before any leak count, so `inactive`
/// and `unverified` are not verdicts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LayerStatus {
    Absent,
    Inactive,
    Unverified,
    Active { hidden: u32, leaks: u32 },
}

/// `DiagnosticCheck` without localized labels or probe sentences. Those
/// strings live in the archive; the id is stable across locales.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticCheck {
    pub id: String,
    pub layer: CheckLayer,
    pub outcome: CheckOutcome,
    pub owned: bool,
    /// Hook names the active kernel backend reported as not installed
    /// (`DiagnosticCheck.missingHooks`). Empty on a healthy device.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_hooks: Vec<String>,
}

/// `LayerReport`. `backend` is the native backend id, or null for the Java layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerReport {
    pub backend: Option<NativeBackendId>,
    pub status: LayerStatus,
    pub unowned_leaks: u32,
    pub checks: Vec<DiagnosticCheck>,
}

/// `DiagnosticReport`, minus the per-check prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticReport {
    pub gate: DiagnosticGate,
    pub complete: bool,
    pub native: LayerReport,
    pub java: LayerReport,
}

/// `NativeBackendId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeBackendId {
    Kmod,
    Builtin,
    Kpm,
    Zygisk,
}

impl NativeBackendId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Kmod => "kmod",
            Self::Builtin => "builtin",
            Self::Kpm => "kpm",
            Self::Zygisk => "zygisk",
        }
    }
}

/// `ModuleBrokenReason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleBrokenReason {
    WrongVariant,
    UnsupportedKernel,
    MissingKprobes,
    UnknownVariantInactive,
    AmbiguousLoadFailed,
    SignatureEnforced,
    ActivatorMissing,
    ActivatorNotExecutable,
}

/// `ModuleState`, with the install metadata the dashboard already shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModulePresence {
    NotInstalled,
    Installed {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<String>,
        active: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        gki_variant: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        broken_reason: Option<ModuleBrokenReason>,
        pending_reboot: bool,
        runtime_checkable: bool,
    },
}

/// `NativeBackendStates` reduced to the four presence values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendSet {
    pub kmod: ModulePresence,
    pub builtin: ModulePresence,
    pub kpm: ModulePresence,
    pub zygisk: ModulePresence,
}

/// `DisplayNativeBackend`. `id` is null only when nothing is installed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveBackend {
    pub id: Option<NativeBackendId>,
    pub state: ModulePresence,
}

/// Selected `KmodLoadStatus` fields. The dmesg tail and insmod stderr are
/// archive material, not part of the compact snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KmodLoad {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uname_r: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gki_variant: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kmod_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_manager: Option<String>,
    pub loaded: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub insmod_exit: Option<i32>,
    pub fresh_for_current_boot: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filesystem_hiding: Option<bool>,
}

/// `RootShellDiag` without `idLine` and `context` (both can be long and
/// identifying). `runtime_checkable == false` means an inactive reading is
/// not evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RootShell {
    pub uid: Option<i32>,
    pub errno_ctl: String,
    pub runtime_checkable: bool,
}

/// Compact `DiagnosticSummaryInfo`. Routing-read timers and the domain-packet
/// revision stay out; eligibility, the retained measurement, and evidence do not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticSummary {
    pub eligibility: DiagnosticEligibility,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_run_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_stage: Option<DiagnosticStage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_attempt: Option<AttemptInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measurement: Option<MeasurementInfo>,
    pub applicability: MeasurementApplicability,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<MeasurementEvidence>,
    pub current_success: bool,
    pub probe_unavailable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_known_routing: Option<SelfRouting>,
}

/// `DiagnosticEligibility`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticEligibility {
    Initializing,
    RestartApp,
    RestartDevice,
    Applying,
    ApplicationUnknown,
    ApplicationFailed,
    Checking,
    Unknown,
    VpnOff,
    SelfExcluded,
    Eligible,
}

/// `DiagnosticStage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticStage {
    Waiting,
    Checking,
    Core,
    Slow,
    Verifying,
    Draining,
}

/// `RunOutcome`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    NotStarted,
    Completed,
    Interrupted,
    Failed,
}

/// `TransitionFailure`. Safe categories only; root output does not belong here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionFailure {
    InitializationPending,
    ReadFailed,
    ValidationFailed,
    ExecutionFailed,
    DeadlineExceeded,
    ApplicationUnknown,
    ApplicationFailed,
    ContextChanged,
    ContextUnknown,
    ResourceUnavailable,
    Cancelled,
    Busy,
    UiEditConflict,
    MutationPaused,
}

/// `DiagnosticAttemptInfo`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptInfo {
    pub run_id: i64,
    pub outcome: RunOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<TransitionFailure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eligibility: Option<DiagnosticEligibility>,
}

/// `DiagnosticMeasurementInfo`. Timestamps are the measurement's own epoch
/// milliseconds, not `generated_at`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementInfo {
    pub run_id: i64,
    pub started_at: i64,
    pub ended_at: i64,
    pub completed: bool,
    pub interrupted: bool,
    pub observation_id: i64,
}

/// `MeasurementApplicability`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementApplicability {
    Absent,
    Changed,
    Unverified,
    MatchesLastObservation,
}

impl MeasurementApplicability {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Changed => "changed",
            Self::Unverified => "unverified",
            Self::MatchesLastObservation => "matches_last_observation",
        }
    }
}

/// `EvidenceConclusion`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceConclusion {
    OwnedLeak,
    Insufficient,
    Partial,
    NoObservedLeak,
}

impl EvidenceConclusion {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OwnedLeak => "owned_leak",
            Self::Insufficient => "insufficient",
            Self::Partial => "partial",
            Self::NoObservedLeak => "no_observed_leak",
        }
    }
}

/// `MeasurementEvidence`. `system_blocked` is the Kotlin `systemBlocked` count
/// (reads hidden by SELinux, not by a backend hook).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementEvidence {
    pub hidden: u32,
    pub system_blocked: u32,
    pub nothing_to_leak: u32,
    pub leaks: u32,
    pub not_measured: u32,
    pub not_run: u32,
    pub uncovered_leaks: u32,
    pub conclusion: EvidenceConclusion,
}

/// `SelfRouting`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelfRouting {
    VpnOff,
    Excluded,
    Routed,
}

/// Reject a snapshot that would let a model treat an unmeasured run as a verdict.
pub fn validate_snapshot(snapshot: &DiagnosticSnapshot) -> Result<(), SnapshotError> {
    if snapshot.schema != ASSISTANT_SNAPSHOT_SCHEMA {
        return Err(SnapshotError::Schema);
    }
    if !single_line(&snapshot.generated_at, 40) {
        return Err(SnapshotError::GeneratedAt);
    }
    if !CAPTURE_KINDS.contains(&snapshot.capture_kind.as_str()) {
        return Err(SnapshotError::CaptureKind);
    }
    if !valid_app(&snapshot.app) {
        return Err(SnapshotError::App);
    }
    if !valid_device(&snapshot.device) {
        return Err(SnapshotError::Device);
    }
    if !valid_root_shell(&snapshot.root_shell) {
        return Err(SnapshotError::RootShell);
    }
    if !valid_active_backend(&snapshot.active_backend) {
        return Err(SnapshotError::ActiveBackend);
    }
    if !valid_module_tree(&snapshot.backends, &snapshot.ports) {
        return Err(SnapshotError::ActiveBackend);
    }
    if let Some(load) = &snapshot.kmod_load {
        if !valid_kmod_load(load) {
            return Err(SnapshotError::KmodLoad);
        }
    }
    if !valid_errors(&snapshot.errors) {
        return Err(SnapshotError::Errors);
    }
    validate_measurement(snapshot)?;
    if let Some(summary) = &snapshot.diagnostics {
        validate_summary(summary)?;
    }
    Ok(())
}

fn validate_measurement(snapshot: &DiagnosticSnapshot) -> Result<(), SnapshotError> {
    let measured = snapshot.gate == Some(DiagnosticGate::Routed);
    if !measured && (snapshot.native_verdict.is_some() || snapshot.java_verdict.is_some()) {
        return Err(SnapshotError::VerdictWithoutMeasurement);
    }
    let Some(report) = &snapshot.report else {
        return Ok(());
    };
    if snapshot.gate != Some(report.gate) {
        return Err(SnapshotError::GateMismatch);
    }
    if report.gate != DiagnosticGate::Routed
        && (snapshot.native_verdict.is_some() || snapshot.java_verdict.is_some())
    {
        return Err(SnapshotError::VerdictWithoutMeasurement);
    }
    validate_layer(&report.native, CheckLayer::Native)?;
    validate_layer(&report.java, CheckLayer::Java)?;
    if report.java.backend.is_some() {
        return Err(SnapshotError::Checks);
    }
    Ok(())
}

fn validate_layer(layer: &LayerReport, expected: CheckLayer) -> Result<(), SnapshotError> {
    if layer.checks.len() > MAX_CHECKS {
        return Err(SnapshotError::Checks);
    }
    for check in &layer.checks {
        if check.layer != expected || !valid_check(check) {
            return Err(SnapshotError::Checks);
        }
    }
    Ok(())
}

fn validate_summary(summary: &DiagnosticSummary) -> Result<(), SnapshotError> {
    if let Some(measurement) = &summary.measurement {
        if measurement.started_at < 0
            || measurement.ended_at < measurement.started_at
            || measurement.run_id < 0
            || measurement.observation_id < 0
        {
            return Err(SnapshotError::Summary);
        }
    }
    if let Some(attempt) = &summary.last_attempt {
        if attempt.run_id < 0 {
            return Err(SnapshotError::Summary);
        }
    }
    if summary.active_run_id.is_some_and(|run_id| run_id < 0) {
        return Err(SnapshotError::Summary);
    }
    Ok(())
}

fn valid_app(app: &AppInfo) -> bool {
    single_line(&app.package_name, MAX_SHORT)
        && app.package_name.contains('.')
        && single_line(&app.version, 64)
        && !app.version.contains('/')
        && !app.version.contains('\\')
}

fn valid_device(device: &DeviceInfo) -> bool {
    single_line(&device.manufacturer, MAX_SHORT)
        && single_line(&device.model, MAX_SHORT)
        && single_line(&device.android_release, 32)
        && (1..100).contains(&device.sdk)
        && (1..=MAX_ABIS).contains(&device.abis.len())
        && device.abis.iter().all(|abi| single_line(abi, 32))
}

fn valid_root_shell(shell: &RootShell) -> bool {
    single_line(&shell.errno_ctl, 64) && shell.uid.is_none_or(|uid| (0..1_000_000).contains(&uid))
}

fn valid_active_backend(active: &ActiveBackend) -> bool {
    match (&active.id, &active.state) {
        (None, ModulePresence::NotInstalled) => true,
        (Some(_), ModulePresence::Installed { .. }) => valid_installed(&active.state),
        _ => false,
    }
}

fn valid_module_tree(backends: &BackendSet, ports: &ModulePresence) -> bool {
    [
        &backends.kmod,
        &backends.builtin,
        &backends.kpm,
        &backends.zygisk,
        ports,
    ]
    .into_iter()
    .all(valid_installed)
}

fn valid_installed(module: &ModulePresence) -> bool {
    match module {
        ModulePresence::NotInstalled => true,
        ModulePresence::Installed {
            version,
            gki_variant,
            ..
        } => {
            version.as_ref().is_none_or(|value| single_line(value, 64))
                && gki_variant
                    .as_ref()
                    .is_none_or(|value| single_line(value, 64))
        }
    }
}

fn valid_kmod_load(load: &KmodLoad) -> bool {
    optional_line(load.uname_r.as_deref(), MAX_SHORT)
        && optional_line(load.gki_variant.as_deref(), 64)
        && optional_line(load.kmod_version.as_deref(), 64)
        && optional_line(load.root_manager.as_deref(), 64)
}

fn valid_errors(errors: &[String]) -> bool {
    errors.len() <= MAX_ERRORS
        && errors
            .iter()
            .all(|error| single_line(error, MAX_ERROR_CHARS))
}

fn valid_check(check: &DiagnosticCheck) -> bool {
    ident(&check.id)
        && check.missing_hooks.len() <= MAX_HOOKS
        && check.missing_hooks.iter().all(|hook| ident(hook))
}

fn ident(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    value.chars().count() <= MAX_ID_CHARS
        && (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn single_line(value: &str, max_chars: usize) -> bool {
    !value.is_empty()
        && value.chars().count() <= max_chars
        && !value
            .chars()
            .any(|c| c.is_control() || c == '/' || c == '\\')
}

fn optional_line(value: Option<&str>, max_chars: usize) -> bool {
    value.is_none_or(|value| single_line(value, max_chars))
}

#[cfg(test)]
pub(crate) fn routed_fixture() -> DiagnosticSnapshot {
    DiagnosticSnapshot {
        schema: ASSISTANT_SNAPSHOT_SCHEMA,
        bundle_schema: Some(2),
        generated_at: "2026-10-19T18:00:00Z".to_string(),
        capture_kind: "debug".to_string(),
        app: AppInfo {
            package_name: "dev.okhsunrog.vpnhide".to_string(),
            version: "1.3.1".to_string(),
        },
        device: DeviceInfo {
            manufacturer: "Google".to_string(),
            model: "Pixel 8 Pro".to_string(),
            android_release: "16".to_string(),
            sdk: 36,
            abis: vec!["arm64-v8a".to_string()],
        },
        self_needs_restart: false,
        gate: Some(DiagnosticGate::Routed),
        self_test_run_id: Some(41),
        diagnostics: Some(DiagnosticSummary {
            eligibility: DiagnosticEligibility::Eligible,
            active_run_id: None,
            active_stage: None,
            last_attempt: Some(AttemptInfo {
                run_id: 41,
                outcome: RunOutcome::Completed,
                failure: None,
                eligibility: None,
            }),
            measurement: Some(MeasurementInfo {
                run_id: 41,
                started_at: 1_700_000_000_000,
                ended_at: 1_700_000_000_500,
                completed: true,
                interrupted: false,
                observation_id: 9,
            }),
            applicability: MeasurementApplicability::MatchesLastObservation,
            evidence: Some(MeasurementEvidence {
                hidden: 4,
                system_blocked: 0,
                nothing_to_leak: 1,
                leaks: 1,
                not_measured: 0,
                not_run: 0,
                uncovered_leaks: 0,
                conclusion: EvidenceConclusion::OwnedLeak,
            }),
            current_success: false,
            probe_unavailable: false,
            last_known_routing: Some(SelfRouting::Routed),
        }),
        native_verdict: Some(Verdict::Partial),
        java_verdict: Some(Verdict::Ok),
        report: Some(DiagnosticReport {
            gate: DiagnosticGate::Routed,
            complete: true,
            native: LayerReport {
                backend: Some(NativeBackendId::Kmod),
                status: LayerStatus::Active {
                    hidden: 4,
                    leaks: 1,
                },
                unowned_leaks: 0,
                checks: vec![
                    DiagnosticCheck {
                        id: "ioctl_flags".to_string(),
                        layer: CheckLayer::Native,
                        outcome: CheckOutcome::Leak,
                        owned: true,
                        missing_hooks: vec!["ioctl_flags".to_string()],
                    },
                    DiagnosticCheck {
                        id: "getifaddrs".to_string(),
                        layer: CheckLayer::Native,
                        outcome: CheckOutcome::HiddenBackend,
                        owned: true,
                        missing_hooks: Vec::new(),
                    },
                ],
            },
            java: LayerReport {
                backend: None,
                status: LayerStatus::Active {
                    hidden: 2,
                    leaks: 0,
                },
                unowned_leaks: 0,
                checks: vec![DiagnosticCheck {
                    id: "network_capabilities".to_string(),
                    layer: CheckLayer::Java,
                    outcome: CheckOutcome::NotMeasured {
                        reason: NotMeasuredReason::NoGroundTruth,
                    },
                    owned: true,
                    missing_hooks: Vec::new(),
                }],
            },
        }),
        backends: BackendSet {
            kmod: installed(true),
            builtin: ModulePresence::NotInstalled,
            kpm: ModulePresence::NotInstalled,
            zygisk: ModulePresence::NotInstalled,
        },
        active_backend: ActiveBackend {
            id: Some(NativeBackendId::Kmod),
            state: installed(true),
        },
        ports: ModulePresence::NotInstalled,
        kmod_load: Some(KmodLoad {
            uname_r: Some("6.1.99-android14-11".to_string()),
            gki_variant: Some("android14-6.1".to_string()),
            kmod_version: Some("1.3.1".to_string()),
            root_manager: Some("kernelsu".to_string()),
            loaded: true,
            insmod_exit: Some(0),
            fresh_for_current_boot: true,
            filesystem_hiding: Some(false),
        }),
        root_shell: RootShell {
            uid: Some(0),
            errno_ctl: "ok".to_string(),
            runtime_checkable: true,
        },
        errors: Vec::new(),
    }
}

#[cfg(test)]
fn installed(active: bool) -> ModulePresence {
    ModulePresence::Installed {
        version: Some("1.3.1".to_string()),
        active,
        gki_variant: Some("android14-6.1".to_string()),
        broken_reason: None,
        pending_reboot: false,
        runtime_checkable: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_round_trips_and_uses_snake_case_kinds() {
        let snapshot = routed_fixture();
        let json = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(json["schema"], 1);
        assert_eq!(json["gate"], "routed");
        assert_eq!(json["native_verdict"], "partial");
        assert_eq!(json["java_verdict"], "ok");
        assert_eq!(json["report"]["native"]["status"]["kind"], "active");
        assert_eq!(json["report"]["native"]["status"]["hidden"], 4);
        assert_eq!(json["report"]["native"]["status"]["leaks"], 1);
        assert_eq!(
            json["report"]["native"]["checks"][0]["outcome"]["kind"],
            "leak"
        );
        assert_eq!(
            json["report"]["native"]["checks"][1]["outcome"]["kind"],
            "hidden_backend"
        );
        assert_eq!(
            json["report"]["java"]["checks"][0]["outcome"],
            serde_json::json!({"kind": "not_measured", "reason": "no_ground_truth"})
        );
        assert_eq!(json["active_backend"]["id"], "kmod");
        assert_eq!(json["active_backend"]["state"]["kind"], "installed");
        assert_eq!(json["backends"]["zygisk"]["kind"], "not_installed");
        assert_eq!(json["diagnostics"]["eligibility"], "eligible");
        assert_eq!(json["diagnostics"]["evidence"]["conclusion"], "owned_leak");
        assert_eq!(
            json["diagnostics"]["applicability"],
            "matches_last_observation"
        );
        assert!(json.get("dmesg").is_none());
        assert!(json.get("logcat").is_none());
        assert!(json.get("sections").is_none());

        let parsed: DiagnosticSnapshot = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, snapshot);
        validate_snapshot(&parsed).unwrap();
        assert!(parsed.measurement_is_meaningful());
        assert!(!parsed.allows_positive_claim());
    }

    #[test]
    fn verdict_without_a_routed_gate_is_rejected() {
        let mut snapshot = routed_fixture();
        snapshot.gate = Some(DiagnosticGate::VpnOff);
        snapshot.report = None;
        snapshot.native_verdict = Some(Verdict::Ok);
        snapshot.java_verdict = None;
        assert_eq!(
            validate_snapshot(&snapshot),
            Err(SnapshotError::VerdictWithoutMeasurement)
        );
        assert!(!snapshot.measurement_is_meaningful());
    }

    #[test]
    fn report_gate_must_match_the_top_level_gate() {
        let mut snapshot = routed_fixture();
        snapshot.gate = Some(DiagnosticGate::NeedsRestart);
        snapshot.native_verdict = None;
        snapshot.java_verdict = None;
        assert_eq!(
            validate_snapshot(&snapshot),
            Err(SnapshotError::GateMismatch)
        );
    }

    #[test]
    fn unknown_field_and_wrong_schema_do_not_parse_as_this_contract() {
        let mut value = serde_json::to_value(routed_fixture()).unwrap();
        value["schema"] = serde_json::json!(2);
        let newer: DiagnosticSnapshot = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(validate_snapshot(&newer), Err(SnapshotError::Schema));

        value["schema"] = serde_json::json!(1);
        value["logcat"] = serde_json::json!("tun0");
        assert!(serde_json::from_value::<DiagnosticSnapshot>(value).is_err());
    }

    #[test]
    fn check_cap_and_active_backend_shape_are_enforced() {
        let mut snapshot = routed_fixture();
        snapshot.report.as_mut().unwrap().native.checks = (0..MAX_CHECKS + 1)
            .map(|index| DiagnosticCheck {
                id: format!("check_{index}"),
                layer: CheckLayer::Native,
                outcome: CheckOutcome::NothingToLeak,
                owned: true,
                missing_hooks: Vec::new(),
            })
            .collect();
        assert_eq!(validate_snapshot(&snapshot), Err(SnapshotError::Checks));

        snapshot = routed_fixture();
        snapshot.active_backend = ActiveBackend {
            id: Some(NativeBackendId::Kmod),
            state: ModulePresence::NotInstalled,
        };
        assert_eq!(
            validate_snapshot(&snapshot),
            Err(SnapshotError::ActiveBackend)
        );
    }

    #[test]
    fn positive_claim_requires_a_matching_measurement_with_no_owned_leak() {
        let mut snapshot = routed_fixture();
        snapshot.diagnostics.as_mut().unwrap().current_success = true;
        assert!(!snapshot.allows_positive_claim());

        {
            let evidence = snapshot
                .diagnostics
                .as_mut()
                .unwrap()
                .evidence
                .as_mut()
                .unwrap();
            evidence.conclusion = EvidenceConclusion::NoObservedLeak;
            evidence.leaks = 0;
        }
        snapshot.native_verdict = Some(Verdict::Ok);
        snapshot.report.as_mut().unwrap().native.status = LayerStatus::Active {
            hidden: 4,
            leaks: 0,
        };
        assert!(snapshot.allows_positive_claim());

        snapshot.gate = Some(DiagnosticGate::SelfNotRouted);
        snapshot.native_verdict = None;
        snapshot.java_verdict = None;
        snapshot.report = None;
        assert!(!snapshot.allows_positive_claim());
    }

    #[test]
    fn enum_labels_match_their_json() {
        let gates = [
            DiagnosticGate::VpnOff,
            DiagnosticGate::SelfNotRouted,
            DiagnosticGate::NeedsRestart,
            DiagnosticGate::Routed,
        ];
        for gate in gates {
            assert_eq!(
                serde_json::to_value(gate).unwrap(),
                serde_json::Value::String(gate.as_str().to_string())
            );
        }
        assert_eq!(
            CheckOutcome::NotMeasured {
                reason: NotMeasuredReason::NoNetwork
            }
            .kind(),
            "not_measured"
        );
    }
}
