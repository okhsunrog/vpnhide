//! The wire between the app and the persistent root core (core-2.0 §5/§9).
//!
//! This is a **different boundary** from the frozen kernel control/telemetry protocol
//! (the `protocol` crate / docs/protocol.md); changing core IPC does not touch that.
//!
//! Shape and conventions are lifted from the on-device-proven tarpc-over-UDS setup in
//! floppa-vpn (`floppa-vpn-core/src/rpc.rs`), with one deliberate addition — a version
//! handshake (see [`hello`](VpnhideCore::hello)).
//!
//! # Codec: JSON, deliberately (not bincode)
//!
//! The transport is `tokio_serde::formats::Json`. bincode is not self-describing, so any
//! serde shape that needs the format to describe itself — internally/adjacently tagged
//! enums (`#[serde(tag = …)]`), `#[serde(flatten)]`, `#[serde(untagged)]`,
//! `deserialize_with`, `deserialize_any` — *encodes* fine and *fails to decode* inside
//! the framed transport, surfacing as "connection shut down" rather than a catchable
//! decode error. This wire carries the domain's own internally-tagged vocabulary
//! (config edits, operation outcomes, observation state), so JSON is required. JSON's one
//! hazard: `serde_json` writes a non-finite `f64` as `null` and refuses to read it back —
//! keep every float on the wire finite.
//!
//! # Rule: every wire type round-trips through the wire's codec
//!
//! Every argument and return of every [`VpnhideCore`] method round-trips through
//! `serde_json` in `tests::wire_coverage`. Adding a method or field means adding it there.
//!
//! # Versioning (the divergence from floppa-vpn)
//!
//! floppa-vpn needs no versioning: its helper and UI ship in one APK, always the same
//! build, and installing force-stops every process — two builds cannot be live at once.
//! vpnhide is different: the core is a root-manager module and the app is an APK, updated
//! on **separate channels**, so the two ends can be different builds. [`PROTOCOL_VERSION`]
//! is a single monotonic number, and [`VpnhideCore::hello`] must be the first call: on a
//! mismatch the client refuses and tells the user to update the lagging component, rather
//! than talking a format the peer does not share.

use std::time::Duration;

mod config;
mod diag;
mod edit;
mod logcat;

pub use config::{
    AppConfig, CanonicalConfig, NativeRole, PortPolicy, PortProtocol, PortRule, Settings,
};
pub use diag::{
    DiagnosticProbe, DiagnosticsReport, DiagnosticsRequest, EffectError, ProbeOutput, RootEffect,
};
pub use edit::{ConfigAppField, ConfigEdit, ConfigSetField, ConfigToggle};
pub use logcat::{
    LogcatAutoStopReason, LogcatRecordingError, LogcatSessionId, LogcatSessionState,
    LogcatSessionStatus,
};

/// Monotonic IPC protocol version. Bump on any change to the method set or wire types.
/// The client checks it in [`VpnhideCore::hello`] before any other call.
pub const PROTOCOL_VERSION: u32 = 8;

/// Filesystem path of the core's UDS socket. A path socket (never abstract, §9): its
/// SELinux label + owner are what gate access. Kept in sync with the app-side constant.
pub const SOCKET_NAME: &str = "vpnhide-core.sock";

/// Max length-delimited frame on the tarpc transport (bytes). Oversized frames are dropped.
pub const MAX_FRAME_BYTES: usize = 1 << 20; // 1 MiB

/// Soft cap on concurrent accepted client connections (slow consumers / DoS bound).
pub const MAX_CONCURRENT_CONNECTIONS: usize = 8;

/// How long the core holds a [`state_since`](VpnhideCore::state_since) call open waiting
/// for something newer. A long poll mapped onto request/response, not a subscription.
pub const STATE_HOLD: Duration = Duration::from_secs(20);

/// The client's per-call deadline for the state long poll. **Must exceed [`STATE_HOLD`]**:
/// tarpc's default deadline is 10 s, and a call held longer than the caller waits fails on
/// a healthy connection. Every client call sets an explicit deadline in its `Context`.
pub const STATE_POLL_DEADLINE: Duration = Duration::from_secs(30);

/// The client's per-call deadline for ordinary request/response calls.
pub const CALL_DEADLINE: Duration = Duration::from_secs(15);

/// Client deadline for [`VpnhideCore::collect_diagnostics`]: the slowest probe (kernel
/// images) is bounded by core at [`PROBE_MAX_RUNTIME`]; this leaves room on top.
pub const DIAGNOSTICS_DEADLINE: Duration = Duration::from_secs(210);

/// Client deadline for [`VpnhideCore::stop_logcat_recording`]: core waits a few seconds
/// for the child to exit, then copies up to its byte cap into the export dir.
pub const LOGCAT_STOP_DEADLINE: Duration = Duration::from_secs(60);

/// Core's own cap on any single probe's runtime.
pub const PROBE_MAX_RUNTIME: Duration = Duration::from_secs(180);

/// A stable identifier for a config-mutation operation. Reusing an id with the same
/// payload queries/deduplicates it; reuse with a different payload is rejected (§5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct OperationId(pub u64);

/// The client's identity in the handshake.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClientHello {
    pub protocol: u32,
    /// The app build, for diagnostics only — never a compatibility input.
    pub build: String,
}

/// The core's answer to [`VpnhideCore::hello`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Handshake {
    /// Compatible: the current core instance/epoch the caller should follow (§6).
    Compatible {
        core_protocol: u32,
        core_build: String,
        epoch: u64,
    },
    /// Incompatible skew: which side is behind. The app surfaces "update the lagging
    /// component"; it does not attempt to speak the other version.
    Incompatible { core_protocol: u32 },
}

/// A published state and which run of the core published it (§6 cross-process epoch).
///
/// `epoch` identifies the core instance; `revision` only ever grows *within one epoch*.
/// A restart is a new `epoch`, whose `revision` starts over — so the client adopts on an
/// epoch change rather than comparing revisions across runs (this is floppa-vpn's
/// `Published { boot, state }`, renamed to our vocabulary).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Published {
    pub epoch: u64,
    pub revision: u64,
    pub snapshot: StateSnapshot,
}

/// The read-only replica the app mirrors from the core.
///
/// The read-only replica the app mirrors (§3): the confirmed canonical config, the
/// operations in flight, and the root observation. `config` is `None` when the core holds
/// no usable config (absent ≠ empty — a distinct state, §3). TODO(§3): join in the
/// provenance-tagged app-context measurements when diagnostics move onto the wire.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct StateSnapshot {
    /// The confirmed canonical config, or `None` when the core has none.
    pub config: Option<CanonicalConfig>,
    /// A monotonic counter that bumps only when the config changes — the base a
    /// [`submit_edit`](VpnhideCore::submit_edit) conflict-checks against. Distinct from
    /// [`Published::revision`], which bumps on any snapshot change (including observations),
    /// so an observation update never spuriously conflicts a config edit.
    pub config_revision: u64,
    /// Ids of operations still in flight, so the app can show pending intent.
    pub operations_in_flight: Vec<OperationId>,
    /// What the root observation loop currently sees of real routing (§8).
    pub routing: RoutingObservation,
    /// Which native backend core observes as active/installed (§4). Centralizing this — one
    /// detected identity instead of each activator probing for itself — is the point of core
    /// owning all state: the family to project for, and what the app shows, come from here.
    pub backend: NativeBackendStatus,
}

/// Whether observation evidence is current, never obtained, or explicitly Unknown (§6/§8).
/// Last-good display data may still be present when [`ObservationFreshness::Unknown`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationFreshness {
    /// No successful observation yet (never-yet ≠ empty VPN).
    #[default]
    Never,
    /// Authoritative evidence is current; [`RoutingObservation::vpn_interfaces`] may be trusted.
    Current,
    /// Stale or unavailable evidence; do **not** present last-good as current.
    Unknown,
}

/// Framework `TRANSPORT_VPN` classification channel (not netlink name-match).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameworkVpnStatus {
    #[default]
    Never,
    /// Parsed VPN interface set from framework (dumpsys / hook / test stub).
    Classified,
    /// Framework signal not obtainable (host, SELinux, parse miss).
    Unavailable,
    /// Attempted and failed.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct FrameworkVpnEvidence {
    pub status: FrameworkVpnStatus,
    #[serde(default)]
    pub interfaces: Vec<String>,
    #[serde(default)]
    pub detail: String,
}

/// Per-UID VPN membership once framework VPN ifaces are known (§8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UidVpnRouteState {
    VpnOff,
    Excluded,
    Routed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct UidRoutingObservation {
    pub uid: u32,
    pub state: UidVpnRouteState,
    pub detail: String,
}

/// The core's own view of real routing (§8). Netlink triggers/membership; framework
/// classifies VPN. Independent of the app's filterable callbacks.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct RoutingObservation {
    pub freshness: ObservationFreshness,
    /// Bumps on each publish (Current or Unknown).
    pub generation: u64,
    /// Authoritative VPN interfaces when `freshness == Current`; empty otherwise.
    pub vpn_interfaces: Vec<String>,
    /// Retained for display when `freshness == Unknown` — never claimed current.
    #[serde(default)]
    pub last_good_vpn_interfaces: Vec<String>,
    /// Netlink name-match candidates currently up (IMS `ipsec` may appear here).
    #[serde(default)]
    pub netlink_candidates: Vec<String>,
    #[serde(default)]
    pub framework: FrameworkVpnEvidence,
    #[serde(default)]
    pub uid: Option<UidRoutingObservation>,
}

impl RoutingObservation {
    /// Convenience: ever obtained any evidence (Current or Unknown).
    pub fn has_evidence(&self) -> bool {
        !matches!(self.freshness, ObservationFreshness::Never)
    }
}

/// A native backend family. The natives are mutually exclusive — at most one is `active` at a
/// time — so this doubles as "which family to project the control wire for".
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NativeBackend {
    Kmod,
    Builtin,
    Kpm,
    Zygisk,
}

/// What core observes of the native backend (§4), detected from runtime identity rather than
/// from a module directory merely existing (§2). `installed` ≠ `active`: a module can be
/// present but not the running backend. This is observed state (like [`RoutingObservation`]),
/// published so the app reads the active backend from one place instead of joining per-backend
/// status files.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct NativeBackendStatus {
    /// Last backend selected by this core instance for an apply attempt.
    /// Distinct from [`Self::applied`]: selection/publication attempt ≠ confirmed apply.
    /// Zygisk file publication is not proof of per-process hooks.
    pub selected: Option<NativeBackend>,
    /// Last backend whose Native phase settled Confirmed this core epoch.
    /// Still not proof Zygisk processes loaded the library (app-context checks remain).
    #[serde(default)]
    pub applied: Option<NativeBackend>,
    /// Live runtime identity (kernel ctl id / module liveness). Zygisk file
    /// publication alone never sets this. `None` = unavailable evidence.
    pub active: Option<NativeBackend>,
    /// Installed artifacts on disk (module present), not necessarily running.
    pub installed: Vec<NativeBackend>,
    /// This boot's native selection, made once by core and fixed until reboot (v6).
    /// `None` until core has scanned (or when native management is disabled).
    #[serde(default)]
    pub boot: Option<NativeBootStatus>,
}

/// How a selected native backend came to be running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SelectionMode {
    /// Already live when core scanned: the built-in kernel, a backend core loaded
    /// earlier this boot, a KPM that APatch autoloaded or the user loaded by hand
    /// (a supported install method), or the Zygisk module that Zygisk itself injects.
    Adopted,
    /// Core loaded it during this boot's scan.
    Loaded,
}

/// One selected package's share of an over-capacity native target set (v7).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NativeCapacityPackage {
    pub package: String,
    /// The app uids (every profile and clone) the package adds to the target set.
    pub uids: Vec<u32>,
}

/// Why a native target set was refused (v7): it projects to `total` distinct uids and
/// the backends hold `capacity`. `packages` lists every native-selected package with
/// the uids it contributes, most uids first, so the user can choose what to deselect;
/// at least `total - capacity` uids must go. Core never truncates.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NativeCapacityReport {
    pub total: u32,
    pub capacity: u32,
    pub packages: Vec<NativeCapacityPackage>,
}

/// The once-per-boot native decision (v6). Exactly one backend may run; more than one
/// candidate is a [`NativeBootOutcome::Conflict`] and nothing is loaded or configured.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NativeBootOutcome {
    /// The single candidate is running and receives config.
    Selected {
        backend: NativeBackend,
        mode: SelectionMode,
        /// The load-time filesystem-hiding choice, when core knows it: it loaded the
        /// backend this boot, or read an adopted KPM's load args back from KernelPatch.
        /// `None` for an adopted backend whose load args are unknown.
        filesystem_hiding: Option<bool>,
    },
    /// More than one candidate: nothing was loaded and no config was delivered.
    Conflict {
        installed: Vec<NativeBackend>,
        live: Vec<NativeBackend>,
        staged: Vec<NativeBackend>,
    },
    /// The KPM is the candidate but KernelPatch refused authentication (superkey).
    AwaitingAuthentication { detail: String },
    /// KernelPatch cannot be queried and a VPN Hide KPM may be live, so no other
    /// backend can be loaded safely.
    KpmUnverifiable { detail: String },
    /// A VPN Hide KPM is live (loaded by core earlier, by APatch autoload or by hand)
    /// and is the only candidate, but core cannot drive it: its version is older than
    /// `minimum` or unrecognised, or it failed the status handshake. Nothing is
    /// configured and no second copy is loaded (v7).
    KpmIncompatible {
        /// The version KernelPatch reported, when it reported one.
        found: Option<String>,
        minimum: String,
        detail: String,
    },
    /// The config projects to more distinct target uids than the selected backend can
    /// hold. Core refused the whole projection: nothing was written to the backend and
    /// whatever it held before stays in effect. Reached by Repair or boot delivery of a
    /// config that no longer fits — for example after more profiles or clones of a
    /// selected app appeared (v7).
    CapacityExceeded {
        backend: NativeBackend,
        report: NativeCapacityReport,
    },
    /// The selected backend does not support the running kernel.
    UnsupportedKernel {
        backend: NativeBackend,
        uname: String,
        detail: String,
    },
    /// The selected module is installed but its artifact is missing.
    MissingArtifact {
        backend: NativeBackend,
        path: String,
    },
    /// Loading the selected backend failed.
    LoadFailed {
        backend: NativeBackend,
        /// Loader exit code (`insmod`): 124 timeout, 126 spawn failure, 127 missing,
        /// -1 refused before dispatch. `None` when the loader has no exit code (KPM).
        exit: Option<i32>,
        stderr: String,
        /// At most [`MAX_DMESG_TAIL`] matching kernel log lines.
        dmesg_tail: Vec<String>,
        kprobes: Option<bool>,
        kretprobes: Option<bool>,
        signature_rejected: bool,
    },
    /// The selected backend is running, but the last config delivery failed.
    DeliveryFailed {
        backend: NativeBackend,
        detail: String,
    },
    /// The kernel control node belongs to an unknown backend id.
    ForeignOwner { id: u32 },
    /// No native backend is installed or live.
    NoBackend,
    /// Core could not establish the facts it needs (unreadable control node, ...).
    Unknown { detail: String },
}

/// Most kernel-log lines a [`NativeBootOutcome::LoadFailed`] carries.
pub const MAX_DMESG_TAIL: usize = 40;

/// What happened to the Zygisk module's `targets.txt` this boot.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ZygiskTargets {
    /// No Zygisk module directory.
    NotInstalled,
    /// Zygisk is selected; config not delivered yet.
    Pending,
    /// Zygisk is selected and its targets were published.
    Published,
    /// Zygisk is not the selected backend: an empty snapshot replaced its targets.
    Retired,
    /// Retiring failed; Zygisk may still hook previously published targets.
    RetireFailed { detail: String },
}

/// Kernel facts recorded when core loads (or tries to load) the kmod.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct KmodBootReport {
    /// `CONFIG_KPROBES` / `CONFIG_KRETPROBES` from `/proc/config.gz`; `None` = unreadable.
    pub kprobes: Option<bool>,
    pub kretprobes: Option<bool>,
    pub gki_variant: String,
    pub version: String,
    pub uname: String,
}

/// Where a Ports apply came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PortsSource {
    /// The first boot pass after netd's baseline chains exist.
    Boot,
    /// The T+30 s boot safety pass.
    BootRetry,
    /// A config edit or activation repair.
    Edit,
}

/// The last Ports apply (v6), replacing the legacy Ports `load_status`/`load_log`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PortsStatus {
    pub source: PortsSource,
    pub ipv4: FamilyApplyOutcome,
    pub ipv6: FamilyApplyOutcome,
    pub target_count: u32,
    /// Whether netd's `bw_OUTPUT` existed before the boot pass (`None` outside boot).
    pub netd_ready: Option<bool>,
    pub detail: String,
    pub at_ms: u64,
}

/// The typed per-boot native status core publishes (v6).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NativeBootStatus {
    /// `/proc/sys/kernel/random/boot_id` the decision belongs to.
    pub boot_id: String,
    /// The core epoch that made the decision (a restart in the same boot keeps it).
    pub decided_epoch: u64,
    pub decided_at_ms: u64,
    pub updated_at_ms: u64,
    pub outcome: NativeBootOutcome,
    /// Installed native modules (module.prop, not disabled, not scheduled for removal).
    pub installed: Vec<NativeBackend>,
    /// Live runtime identities at the scan (built-in kernel, a live kmod or KPM).
    pub live: Vec<NativeBackend>,
    /// Modules staged only in `modules_update`: candidates after the next reboot.
    pub staged: Vec<NativeBackend>,
    #[serde(default)]
    pub kmod: Option<KmodBootReport>,
    pub zygisk_targets: ZygiskTargets,
    #[serde(default)]
    pub ports: Option<PortsStatus>,
    /// Installed 1.x modules that 2.0 replaced (`vpnhide_ports`, `vpnhide_builtin`).
    /// Their own boot scripts still run beside core, so they are reported even
    /// when the app is never opened (v7).
    #[serde(default)]
    pub obsolete_modules: Vec<String>,
}

/// The base a submitted edit is checked against, for conflict detection (§5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BaseRef {
    pub revision: u64,
}

/// The core accepted a durable mutation. It now owns the operation independently of the
/// caller's connection (§5): client death does not cancel it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Accepted {
    pub op: OperationId,
}

/// Why the core refused an edit at submission.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SubmitError {
    /// A stale change to a field another accepted change already moved (§5).
    Conflict { field: String },
    /// The same id was reused with a different payload.
    IdReused,
    /// The core has no usable canonical config and the edit is not a bootstrap/reset.
    Unavailable,
    /// The edit failed validation before any phase could be dispatched.
    Rejected { reason: String },
    /// The edited config projects to more native target uids than the running backend
    /// holds. Nothing was persisted or applied; the previous config stays in effect (v7).
    NativeCapacityExceeded(NativeCapacityReport),
}

/// The phases an operation runs, in the order the config coordinator dispatches them
/// (mirrors `ConfigPhase`): persist the config, run coupled secret/cleanup commands, then
/// activate the native backend and ports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ConfigPhase {
    Persist,
    Secret,
    Cleanup,
    Native,
    Ports,
}

/// One phase of an operation as the core executed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PhaseRecord {
    pub phase: ConfigPhase,
    pub outcome: PhaseOutcome,
}

/// A phase's outcome (mirrors `PhaseOutcome`): known success, known failure and unknown are
/// preserved as distinct (§3), as are not-yet-attempted and still-running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PhaseOutcome {
    NotAttempted,
    Running,
    Confirmed,
    FailedKnown,
    Unknown,
}

/// Per-address-family Ports apply outcome on the wire. IPv4 and IPv6 are not one
/// atomic transaction — callers must read both fields (§11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FamilyApplyOutcome {
    Confirmed,
    FailedKnown,
    Unknown,
    NotAttempted,
}

/// Honest Ports dual-family report attached to an operation when a Ports phase ran.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PortsFamilyReport {
    pub ipv4: FamilyApplyOutcome,
    pub ipv6: FamilyApplyOutcome,
    pub target_count: u32,
}

/// An operation's durable record: its phases and, once settled, its outcome (§5).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OperationRecord {
    pub op: OperationId,
    pub phases: Vec<PhaseRecord>,
    /// `None` while still running; `Some` once the operation settled.
    pub settled: Option<PhaseOutcome>,
    /// Present when a Ports phase was attempted; carries per-family detail so a
    /// single `PhaseOutcome` cannot pretend IPv4+IPv6 were atomic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ports: Option<PortsFamilyReport>,
}

/// The app <-> core boundary, spelled for a socket. Intent flows one way, published state
/// the other; nothing here carries in-process handles.
#[tarpc::service]
pub trait VpnhideCore {
    /// Version handshake. **Must be the first call** on a connection. On
    /// [`Handshake::Incompatible`] the client refuses and asks the user to update the
    /// lagging component (§9); it never falls back to a foreign wire format.
    async fn hello(client: ClientHello) -> Handshake;

    /// The first published state newer than what the caller holds (a `watch` over
    /// request/response). `epoch`/`revision` are what the caller last saw. An `epoch`
    /// that is not the core's current run is answered immediately — everything the caller
    /// knew came from a run that is over. Otherwise returns as soon as something strictly
    /// newer exists, and holds the call open up to [`STATE_HOLD`] before returning the
    /// current state unchanged. Nothing is missed and nothing is replayed.
    async fn state_since(epoch: u64, revision: u64) -> Published;

    /// Submit a typed field edit, checked against `base`. Accepted (the core now owns the
    /// operation) or rejected with an explicit reason (§5).
    async fn submit_edit(
        op: OperationId,
        base: BaseRef,
        edit: ConfigEdit,
    ) -> Result<Accepted, SubmitError>;

    /// Query an operation's phases and outcome. Survives the submitting client's death,
    /// so a reconnecting app can find out what happened. `None` if the id is unknown.
    async fn get_operation(op: OperationId) -> Option<OperationRecord>;

    /// Run a batch of fixed root probes (v5). No caller-chosen command or path exists.
    async fn collect_diagnostics(request: DiagnosticsRequest) -> DiagnosticsReport;

    /// Apply a fixed root effect outside the config (v5): superkey, legacy cleanup, full
    /// reset. Serialized with config operations; `op` makes a retry idempotent.
    async fn root_effect(op: OperationId, effect: RootEffect) -> Result<(), EffectError>;

    /// Start core's full-system logcat recording (v8). Idempotent per `session`: the same
    /// id while it records (or after it ended) returns its status; a different id while
    /// one records is [`LogcatRecordingError::Busy`]. Core stops it on its own caps.
    async fn start_logcat_recording(
        session: LogcatSessionId,
    ) -> Result<LogcatSessionStatus, LogcatRecordingError>;

    /// Stop `session` (if still running) and copy its file into the app's export dir.
    /// Idempotent: a repeated Stop returns the same status and re-delivers the file if the
    /// app already removed its copy.
    async fn stop_logcat_recording(
        session: LogcatSessionId,
    ) -> Result<LogcatSessionStatus, LogcatRecordingError>;

    /// The current or last session, if any (`file` is never set here).
    async fn logcat_recording_status() -> Option<LogcatSessionStatus>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip through the codec that is actually on the wire (`serde_json`), not
    /// "serde works in general" — the exact gap that let a JSON-only shape ship on a
    /// bincode transport in floppa-vpn and break connects on device.
    fn survives<T>(what: &str, value: &T) -> T
    where
        T: serde::Serialize + serde::de::DeserializeOwned,
    {
        let bytes = serde_json::to_vec(value).unwrap_or_else(|e| panic!("{what} encode: {e}"));
        serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("{what} decode: {e}"))
    }

    fn sample_capacity_report() -> NativeCapacityReport {
        NativeCapacityReport {
            total: 162,
            capacity: 160,
            packages: vec![
                NativeCapacityPackage {
                    package: "com.cloned".into(),
                    uids: vec![10123, 1010123],
                },
                NativeCapacityPackage {
                    package: "com.example".into(),
                    uids: vec![10200],
                },
            ],
        }
    }

    /// Every argument and return type of every `VpnhideCore` method, every variant.
    mod wire_coverage {
        use super::*;
        use std::collections::{BTreeMap, BTreeSet};

        #[test]
        fn hello_arguments_and_result() {
            let hello = ClientHello {
                protocol: PROTOCOL_VERSION,
                build: "1.2.3".into(),
            };
            assert_eq!(survives("ClientHello", &hello), hello);
            for h in [
                Handshake::Compatible {
                    core_protocol: PROTOCOL_VERSION,
                    core_build: "9".into(),
                    epoch: u64::MAX,
                },
                Handshake::Incompatible { core_protocol: 2 },
            ] {
                assert_eq!(survives("Handshake", &h), h);
            }
        }

        /// A config that exercises every wire config type in one value.
        fn sample_config() -> CanonicalConfig {
            let app = AppConfig {
                java: true,
                java_hooks: Some(vec!["hookA".into()]),
                native: NativeRole {
                    enabled: true,
                    kernel: Some(vec!["k1".into(), "k2".into()]),
                    zygisk: None,
                },
                app_hiding: true,
                ports: true,
                port_policy: Some(PortPolicy {
                    mode: "custom".into(),
                    preset: Some("dev".into()),
                    rules: vec![PortRule {
                        protocol: PortProtocol::Tcp,
                        start: 8080,
                        end: 8090,
                    }],
                }),
                hidden: false,
            };
            CanonicalConfig {
                version: 1,
                debug: true,
                debug_switch: false,
                apps: BTreeMap::from([
                    ("com.example".to_owned(), app),
                    ("com.other".to_owned(), AppConfig::default()),
                ]),
                settings: Settings {
                    remember_superkey: true,
                    optional_features: BTreeSet::from(["fs".to_owned()]),
                    auto_hide_vpn_services: true,
                    auto_hide_vpn_name: false,
                    auto_hide_excluded_packages: BTreeSet::from(["com.bank".to_owned()]),
                    auto_hidden_packages: BTreeSet::from(["com.x".to_owned()]),
                },
            }
        }

        #[test]
        fn state_since_arguments_and_result() {
            assert_eq!(survives("epoch", &u64::MAX), u64::MAX);
            for config in [None, Some(sample_config())] {
                let published = Published {
                    epoch: u64::MAX,
                    revision: 7,
                    snapshot: StateSnapshot {
                        config,
                        config_revision: 5,
                        operations_in_flight: vec![OperationId(1), OperationId(u64::MAX)],
                        routing: RoutingObservation {
                            freshness: ObservationFreshness::Current,
                            generation: 3,
                            vpn_interfaces: vec!["tun0".into(), "wg0".into()],
                            last_good_vpn_interfaces: vec!["tun0".into()],
                            netlink_candidates: vec!["tun0".into(), "ipsec20".into()],
                            framework: FrameworkVpnEvidence {
                                status: FrameworkVpnStatus::Classified,
                                interfaces: vec!["tun0".into()],
                                detail: "stub".into(),
                            },
                            uid: Some(UidRoutingObservation {
                                uid: 10123,
                                state: UidVpnRouteState::Excluded,
                                detail: "uid_not_in_vpn_table".into(),
                            }),
                        },
                        backend: NativeBackendStatus {
                            selected: Some(NativeBackend::Zygisk),
                            applied: Some(NativeBackend::Builtin),
                            active: Some(NativeBackend::Builtin),
                            installed: vec![NativeBackend::Builtin, NativeBackend::Zygisk],
                            boot: Some(sample_boot(sample_outcomes()[0].clone())),
                        },
                    },
                };
                assert_eq!(survives("Published", &published), published);
            }
        }

        fn sample_outcomes() -> Vec<NativeBootOutcome> {
            vec![
                NativeBootOutcome::Selected {
                    backend: NativeBackend::Kmod,
                    mode: SelectionMode::Loaded,
                    filesystem_hiding: Some(true),
                },
                NativeBootOutcome::Selected {
                    backend: NativeBackend::Builtin,
                    mode: SelectionMode::Adopted,
                    filesystem_hiding: None,
                },
                NativeBootOutcome::Conflict {
                    installed: vec![NativeBackend::Kmod, NativeBackend::Kpm],
                    live: vec![NativeBackend::Builtin],
                    staged: vec![NativeBackend::Zygisk],
                },
                NativeBootOutcome::AwaitingAuthentication { detail: "a".into() },
                NativeBootOutcome::KpmUnverifiable { detail: "b".into() },
                NativeBootOutcome::KpmIncompatible {
                    found: Some("1.1.1".into()),
                    minimum: "1.2.0".into(),
                    detail: "too old".into(),
                },
                NativeBootOutcome::KpmIncompatible {
                    found: None,
                    minimum: "1.2.0".into(),
                    detail: "no status".into(),
                },
                NativeBootOutcome::CapacityExceeded {
                    backend: NativeBackend::Kmod,
                    report: sample_capacity_report(),
                },
                NativeBootOutcome::UnsupportedKernel {
                    backend: NativeBackend::Kpm,
                    uname: "6.18".into(),
                    detail: "c".into(),
                },
                NativeBootOutcome::MissingArtifact {
                    backend: NativeBackend::Kmod,
                    path: "/x.ko".into(),
                },
                NativeBootOutcome::LoadFailed {
                    backend: NativeBackend::Kmod,
                    exit: Some(1),
                    stderr: "Key was rejected by service".into(),
                    dmesg_tail: vec!["vpnhide: x".into()],
                    kprobes: Some(true),
                    kretprobes: None,
                    signature_rejected: true,
                },
                NativeBootOutcome::DeliveryFailed {
                    backend: NativeBackend::Zygisk,
                    detail: "d".into(),
                },
                NativeBootOutcome::ForeignOwner { id: 0x99 },
                NativeBootOutcome::NoBackend,
                NativeBootOutcome::Unknown { detail: "e".into() },
            ]
        }

        fn sample_boot(outcome: NativeBootOutcome) -> NativeBootStatus {
            NativeBootStatus {
                boot_id: "b-1".into(),
                decided_epoch: 7,
                decided_at_ms: 1,
                updated_at_ms: 2,
                outcome,
                installed: vec![NativeBackend::Kmod],
                live: vec![NativeBackend::Kpm],
                staged: vec![NativeBackend::Zygisk],
                kmod: Some(KmodBootReport {
                    kprobes: Some(true),
                    kretprobes: Some(false),
                    gki_variant: "android14-6.1".into(),
                    version: "2.0.0".into(),
                    uname: "6.1.0".into(),
                }),
                zygisk_targets: ZygiskTargets::Retired,
                ports: Some(PortsStatus {
                    source: PortsSource::BootRetry,
                    ipv4: FamilyApplyOutcome::Confirmed,
                    ipv6: FamilyApplyOutcome::Unknown,
                    target_count: 2,
                    netd_ready: Some(false),
                    detail: "f".into(),
                    at_ms: 3,
                }),
                obsolete_modules: vec!["vpnhide_ports".into()],
            }
        }

        #[test]
        fn native_boot_status_every_variant() {
            for outcome in sample_outcomes() {
                let status = sample_boot(outcome);
                assert_eq!(survives("NativeBootStatus", &status), status);
            }
            for targets in [
                ZygiskTargets::NotInstalled,
                ZygiskTargets::Pending,
                ZygiskTargets::Published,
                ZygiskTargets::Retired,
                ZygiskTargets::RetireFailed { detail: "x".into() },
            ] {
                assert_eq!(survives("ZygiskTargets", &targets), targets);
            }
            for source in [PortsSource::Boot, PortsSource::BootRetry, PortsSource::Edit] {
                assert_eq!(survives("PortsSource", &source), source);
            }
            // v5 snapshots without `boot` still decode.
            let legacy: NativeBackendStatus =
                serde_json::from_str(r#"{"selected":null,"active":null,"installed":[]}"#).unwrap();
            assert_eq!(legacy.boot, None);
        }

        /// The app parses this exact JSON (Kotlin `NativeBootStatusTest`); a wire change
        /// fails here until the fixture is regenerated with `VPNHIDE_UPDATE_FIXTURES=1`.
        #[test]
        fn native_boot_status_matches_app_fixture() {
            let mut all: Vec<NativeBootStatus> =
                sample_outcomes().into_iter().map(sample_boot).collect();
            let mut retire = sample_boot(NativeBootOutcome::NoBackend);
            retire.zygisk_targets = ZygiskTargets::RetireFailed {
                detail: "read-only".into(),
            };
            retire.kmod = None;
            retire.ports = None;
            all.push(retire);
            let json = serde_json::to_string_pretty(&all).unwrap() + "\n";
            let path = concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../fixtures/core-rpc/native-boot-status.json"
            );
            if std::env::var_os("VPNHIDE_UPDATE_FIXTURES").is_some() {
                std::fs::create_dir_all(std::path::Path::new(path).parent().unwrap()).unwrap();
                std::fs::write(path, &json).unwrap();
            }
            assert_eq!(std::fs::read_to_string(path).unwrap(), json);
        }

        #[test]
        fn submit_edit_arguments_and_result() {
            let edits = [
                ConfigEdit::Toggle {
                    field: ConfigToggle::Debug,
                    enabled: true,
                },
                ConfigEdit::SettingSet {
                    field: ConfigSetField::AutoHideExcludedPackages,
                    values: BTreeSet::from(["com.bank".to_owned()]),
                },
                ConfigEdit::App {
                    pkg: "com.example".into(),
                    field: ConfigAppField::Native,
                    value: sample_config().apps["com.example"].clone(),
                },
                ConfigEdit::RemoveApp {
                    pkg: "com.example".into(),
                },
                ConfigEdit::Replace {
                    config: sample_config(),
                },
                ConfigEdit::RepairActivation,
            ];
            for edit in edits {
                assert_eq!(survives("ConfigEdit", &edit), edit);
            }
            assert_eq!(
                survives("BaseRef", &BaseRef { revision: 3 }),
                BaseRef { revision: 3 }
            );
            let ok: Result<Accepted, SubmitError> = Ok(Accepted { op: OperationId(5) });
            assert_eq!(survives("Accepted", &ok), ok);
            for err in [
                SubmitError::Conflict {
                    field: "apps.x.native".into(),
                },
                SubmitError::IdReused,
                SubmitError::Unavailable,
                SubmitError::Rejected {
                    reason: "oversized".into(),
                },
                SubmitError::NativeCapacityExceeded(sample_capacity_report()),
            ] {
                let refused: Result<Accepted, SubmitError> = Err(err.clone());
                assert_eq!(survives("SubmitError", &refused), refused);
            }
        }

        #[test]
        fn diagnostics_arguments_and_result() {
            let probes = vec![
                DiagnosticProbe::RootSnapshot { packages: true },
                DiagnosticProbe::DebugSnapshot,
                DiagnosticProbe::HookCounters,
                DiagnosticProbe::Dmesg,
                DiagnosticProbe::LogcatAppTags,
                DiagnosticProbe::LogcatLsposedBoot,
                DiagnosticProbe::KernelPartitions,
                DiagnosticProbe::KernelImages,
                DiagnosticProbe::LsposedModulesDb,
                DiagnosticProbe::GroundTruthChecks,
                DiagnosticProbe::AppVpnState { uid: 10123 },
            ];
            let request = DiagnosticsRequest {
                probes: probes.clone(),
                clear_dmesg: true,
            };
            assert_eq!(survives("DiagnosticsRequest", &request), request);
            let report = DiagnosticsReport {
                outputs: probes
                    .into_iter()
                    .map(|probe| ProbeOutput {
                        probe,
                        exit: 0,
                        output: "x".into(),
                        file: Some("/d/f".into()),
                        artifacts: vec!["/d/a".into()],
                        truncated: true,
                    })
                    .collect(),
                dmesg_cleared: true,
            };
            assert_eq!(survives("DiagnosticsReport", &report), report);
        }

        #[test]
        fn root_effect_arguments_and_result() {
            for effect in [
                RootEffect::SetSuperkey {
                    superkey: "k".into(),
                },
                RootEffect::ClearSuperkey,
                RootEffect::DeleteLegacyConfig,
                RootEffect::FullReset,
            ] {
                assert_eq!(survives("RootEffect", &effect), effect);
            }
            for result in [
                Ok(()),
                Err(EffectError::Invalid("x".into())),
                Err(EffectError::Busy),
                Err(EffectError::Failed("y".into())),
            ] {
                assert_eq!(survives("effect result", &result), result);
            }
        }

        #[test]
        fn logcat_recording_arguments_and_results() {
            let id = LogcatSessionId(u64::MAX);
            assert_eq!(survives("LogcatSessionId", &id), id);
            let states = [
                LogcatSessionState::Recording,
                LogcatSessionState::Stopped,
                LogcatSessionState::AutoStopped {
                    reason: LogcatAutoStopReason::SizeCap,
                },
                LogcatSessionState::AutoStopped {
                    reason: LogcatAutoStopReason::DurationCap,
                },
                LogcatSessionState::AutoStopped {
                    reason: LogcatAutoStopReason::LogcatExited,
                },
            ];
            for (n, state) in states.into_iter().enumerate() {
                let status = LogcatSessionStatus {
                    session: LogcatSessionId(n as u64 + 1),
                    state,
                    started_at_ms: 1,
                    duration_ms: 2,
                    bytes: 3,
                    truncated: n % 2 == 0,
                    max_bytes: 4,
                    max_duration_ms: 5,
                    logcat_exit: (n == 4).then_some(1),
                    file: (n > 0).then(|| "/d/logcat-session-1.txt".to_owned()),
                };
                let ok: Result<LogcatSessionStatus, LogcatRecordingError> = Ok(status.clone());
                assert_eq!(survives("logcat result", &ok), ok);
                assert_eq!(
                    survives("logcat status", &Some(status.clone())),
                    Some(status)
                );
            }
            for err in [
                LogcatRecordingError::Busy {
                    running: LogcatSessionId(7),
                },
                LogcatRecordingError::UnknownSession,
                LogcatRecordingError::Disabled,
                LogcatRecordingError::Invalid("zero".into()),
                LogcatRecordingError::Failed("spawn".into()),
            ] {
                let refused: Result<LogcatSessionStatus, LogcatRecordingError> = Err(err);
                assert_eq!(survives("logcat error", &refused), refused);
            }
            let none: Option<LogcatSessionStatus> = None;
            assert_eq!(survives("no session", &none), none);
        }

        /// The app parses this exact JSON (Kotlin `CoreLogcatRecordingTest`).
        #[test]
        fn logcat_session_status_matches_app_fixture() {
            let all = vec![
                LogcatSessionStatus {
                    session: LogcatSessionId(42),
                    state: LogcatSessionState::Recording,
                    started_at_ms: 1_760_000_000_000,
                    duration_ms: 1500,
                    bytes: 4096,
                    truncated: false,
                    max_bytes: 64 << 20,
                    max_duration_ms: 1_800_000,
                    logcat_exit: None,
                    file: None,
                },
                LogcatSessionStatus {
                    session: LogcatSessionId(43),
                    state: LogcatSessionState::AutoStopped {
                        reason: LogcatAutoStopReason::SizeCap,
                    },
                    started_at_ms: 1_760_000_000_000,
                    duration_ms: 600_000,
                    bytes: 64 << 20,
                    truncated: true,
                    max_bytes: 64 << 20,
                    max_duration_ms: 1_800_000,
                    logcat_exit: None,
                    file: Some(
                        "/data/user_de/0/x/vpnhide-core-export/logcat-session-43.txt".into(),
                    ),
                },
                LogcatSessionStatus {
                    session: LogcatSessionId(44),
                    state: LogcatSessionState::AutoStopped {
                        reason: LogcatAutoStopReason::LogcatExited,
                    },
                    started_at_ms: 1,
                    duration_ms: 2,
                    bytes: 3,
                    truncated: false,
                    max_bytes: 64 << 20,
                    max_duration_ms: 1_800_000,
                    logcat_exit: Some(1),
                    file: None,
                },
            ];
            let json = serde_json::to_string_pretty(&all).unwrap() + "\n";
            let path = concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../fixtures/core-rpc/logcat-session-status.json"
            );
            if std::env::var_os("VPNHIDE_UPDATE_FIXTURES").is_some() {
                std::fs::write(path, &json).unwrap();
            }
            assert_eq!(std::fs::read_to_string(path).unwrap(), json);
        }

        #[test]
        fn get_operation_result() {
            let none: Option<OperationRecord> = None;
            assert_eq!(survives("no record", &none), none);
            let phases = [
                ConfigPhase::Persist,
                ConfigPhase::Secret,
                ConfigPhase::Cleanup,
                ConfigPhase::Native,
                ConfigPhase::Ports,
            ];
            let outcomes = [
                PhaseOutcome::NotAttempted,
                PhaseOutcome::Running,
                PhaseOutcome::Confirmed,
                PhaseOutcome::FailedKnown,
                PhaseOutcome::Unknown,
            ];
            let record = Some(OperationRecord {
                op: OperationId(9),
                phases: phases
                    .into_iter()
                    .zip(outcomes)
                    .map(|(phase, outcome)| PhaseRecord { phase, outcome })
                    .collect(),
                settled: Some(PhaseOutcome::FailedKnown),
                ports: Some(PortsFamilyReport {
                    ipv4: FamilyApplyOutcome::Confirmed,
                    ipv6: FamilyApplyOutcome::FailedKnown,
                    target_count: 1,
                }),
            });
            assert_eq!(survives("OperationRecord", &record), record);
        }
    }
}
