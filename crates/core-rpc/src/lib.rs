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
mod edit;

pub use config::{
    AppConfig, CanonicalConfig, NativeRole, PortPolicy, PortProtocol, PortRule, Settings,
};
pub use edit::{ConfigAppField, ConfigEdit, ConfigSetField, ConfigToggle};

/// Monotonic IPC protocol version. Bump on any change to the method set or wire types.
/// The client checks it in [`VpnhideCore::hello`] before any other call.
pub const PROTOCOL_VERSION: u32 = 2;

/// Filesystem path of the core's UDS socket. A path socket (never abstract, §9): its
/// SELinux label + owner are what gate access. Kept in sync with the app-side constant.
pub const SOCKET_NAME: &str = "vpnhide-core.sock";

/// How long the core holds a [`state_since`](VpnhideCore::state_since) call open waiting
/// for something newer. A long poll mapped onto request/response, not a subscription.
pub const STATE_HOLD: Duration = Duration::from_secs(20);

/// The client's per-call deadline for the state long poll. **Must exceed [`STATE_HOLD`]**:
/// tarpc's default deadline is 10 s, and a call held longer than the caller waits fails on
/// a healthy connection. Every client call sets an explicit deadline in its `Context`.
pub const STATE_POLL_DEADLINE: Duration = Duration::from_secs(30);

/// The client's per-call deadline for ordinary request/response calls.
pub const CALL_DEADLINE: Duration = Duration::from_secs(15);

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

/// The core's own view of real routing, from its netlink observer (§8). Independent of the
/// app's own (filterable) callbacks — that independence is the reason the daemon exists.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct RoutingObservation {
    /// `false` until the observer has read routing at least once (never-yet ≠ nothing).
    pub observed: bool,
    /// VPN-candidate interfaces currently up, by kernel name-match. TODO(§8): cross-check
    /// the framework's `TRANSPORT_VPN` set — name-matching alone counts an IMS `ipsec`
    /// tunnel as a VPN (proven on device).
    pub vpn_interfaces: Vec<String>,
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
    /// Last backend selected by this core instance for an apply attempt. Its
    /// operation record describes publication/application success independently.
    /// In particular, publishing a Zygisk file is not proof of per-process hooks.
    pub selected: Option<NativeBackend>,
    /// The live native backend, from runtime identity (kernel control-node id, module
    /// liveness). `None` also covers unavailable evidence; Zygisk file publication
    /// alone never sets this field. Per-process hook measurements stay separate.
    pub active: Option<NativeBackend>,
    /// Backends whose module is present on disk (installed, not necessarily running).
    pub installed: Vec<NativeBackend>,
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

/// An operation's durable record: its phases and, once settled, its outcome (§5).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OperationRecord {
    pub op: OperationId,
    pub phases: Vec<PhaseRecord>,
    /// `None` while still running; `Some` once the operation settled.
    pub settled: Option<PhaseOutcome>,
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
                            observed: true,
                            vpn_interfaces: vec!["tun0".into(), "wg0".into()],
                        },
                        backend: NativeBackendStatus {
                            selected: Some(NativeBackend::Zygisk),
                            active: Some(NativeBackend::Builtin),
                            installed: vec![NativeBackend::Builtin, NativeBackend::Zygisk],
                        },
                    },
                };
                assert_eq!(survives("Published", &published), published);
            }
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
            ] {
                let refused: Result<Accepted, SubmitError> = Err(err.clone());
                assert_eq!(survives("SubmitError", &refused), refused);
            }
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
            });
            assert_eq!(survives("OperationRecord", &record), record);
        }
    }
}
