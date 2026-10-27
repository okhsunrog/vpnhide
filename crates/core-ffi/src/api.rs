//! Closed app commands and channel-tagged publications. No domain payload crosses
//! UniFFI as serialized text: malformed command shapes are unrepresentable in Kotlin.
use crate::types::*;
use vpnhide_core_client::{ClientError, ClientReply};
use vpnhide_core_rpc as rpc;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct StateReply {
    pub connection_generation: u64,
    pub state: CorePublication,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CorePublication {
    pub startup_error: Option<String>,
    pub epoch: u64,
    pub revision: u64,
    pub config_revision: u64,
    pub config: Option<CoreCanonicalConfig>,
    pub operations_in_flight: Vec<u64>,
    pub mutation_held: bool,
    pub routing: CoreRoutingObservation,
    pub backend: CoreNativeBackendStatus,
    pub debug_lease: bool,
    pub debug_leases: Vec<CoreDebugLease>,
    pub operations: Vec<CoreOperationRecord>,
    pub apply_evidence: Vec<CoreApplyEvidence>,
    pub held_resources: Vec<CoreConfigPhase>,
}
impl From<ClientReply<rpc::Published>> for StateReply {
    fn from(reply: ClientReply<rpc::Published>) -> Self {
        let rpc::Published {
            epoch,
            revision,
            snapshot,
        } = reply.value;
        let rpc::StateSnapshot {
            startup_error,
            config,
            config_revision,
            operations_in_flight,
            mutation_held,
            routing,
            backend,
            debug_lease,
            debug_leases,
            operations,
            apply_evidence,
            held_resources,
        } = snapshot;
        Self {
            connection_generation: reply.generation,
            state: CorePublication {
                startup_error,
                epoch,
                revision,
                config_revision,
                config: config.map(Into::into),
                operations_in_flight: operations_in_flight.into_iter().map(|op| op.0).collect(),
                mutation_held,
                routing: routing.into(),
                backend: backend.into(),
                debug_lease,
                debug_leases: debug_leases.into_iter().map(Into::into).collect(),
                operations: operations.into_iter().map(Into::into).collect(),
                apply_evidence: apply_evidence.into_iter().map(Into::into).collect(),
                held_resources: held_resources.into_iter().map(Into::into).collect(),
            },
        }
    }
}

/// A batch's element type intentionally cannot contain another batch or a whole import.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreFieldEdit {
    Toggle {
        field: CoreConfigToggle,
        enabled: bool,
    },
    SettingSet {
        field: CoreConfigSetField,
        values: Vec<String>,
    },
    App {
        pkg: String,
        field: CoreConfigAppField,
        value: CoreAppConfig,
    },
    RemoveApp {
        pkg: String,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreEdit {
    Batch { edits: Vec<CoreFieldEdit> },
    Replace { config: CoreCanonicalConfig },
    RepairActivation,
}
impl From<CoreFieldEdit> for rpc::ConfigEdit {
    fn from(value: CoreFieldEdit) -> Self {
        match value {
            CoreFieldEdit::Toggle { field, enabled } => Self::Toggle {
                field: field.into(),
                enabled,
            },
            CoreFieldEdit::SettingSet { field, values } => Self::SettingSet {
                field: field.into(),
                values: values.into_iter().collect(),
            },
            CoreFieldEdit::App { pkg, field, value } => Self::App {
                pkg,
                field: field.into(),
                value: value.into(),
            },
            CoreFieldEdit::RemoveApp { pkg } => Self::RemoveApp { pkg },
        }
    }
}
impl From<CoreEdit> for rpc::ConfigEdit {
    fn from(value: CoreEdit) -> Self {
        match value {
            CoreEdit::Batch { edits } => Self::Batch {
                edits: edits.into_iter().map(Into::into).collect(),
            },
            CoreEdit::Replace { config } => Self::Replace {
                config: config.into(),
            },
            CoreEdit::RepairActivation => Self::RepairActivation,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreCommand {
    Edit {
        base_revision: u64,
        edit: CoreEdit,
    },
    SetSuperkey {
        base_revision: u64,
        remember: bool,
        superkey: String,
    },
    ClearSuperkey {
        base_revision: u64,
    },
    Import {
        base_revision: u64,
        config: CoreCanonicalConfig,
        cleanup_legacy: bool,
    },
    CleanupLegacy,
    FullReset,
}
impl From<CoreCommand> for rpc::CoreCommand {
    fn from(value: CoreCommand) -> Self {
        match value {
            CoreCommand::Edit {
                base_revision,
                edit,
            } => Self::Edit {
                base: rpc::BaseRef {
                    revision: base_revision,
                },
                edit: edit.into(),
            },
            CoreCommand::SetSuperkey {
                base_revision,
                remember,
                superkey,
            } => Self::SetSuperkey {
                base: rpc::BaseRef {
                    revision: base_revision,
                },
                remember,
                superkey,
            },
            CoreCommand::ClearSuperkey { base_revision } => Self::ClearSuperkey {
                base: rpc::BaseRef {
                    revision: base_revision,
                },
            },
            CoreCommand::Import {
                base_revision,
                config,
                cleanup_legacy,
            } => Self::Import {
                base: rpc::BaseRef {
                    revision: base_revision,
                },
                config: config.into(),
                cleanup_legacy,
            },
            CoreCommand::CleanupLegacy => Self::CleanupLegacy,
            CoreCommand::FullReset => Self::FullReset,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreSubmitRefusal {
    Conflict { field: String },
    IdReused,
    Expired,
    Busy,
    Unavailable,
    Rejected { reason: String },
    NativeCapacityExceeded { report: CoreNativeCapacityReport },
}
impl From<rpc::SubmitError> for CoreSubmitRefusal {
    fn from(value: rpc::SubmitError) -> Self {
        match value {
            rpc::SubmitError::Conflict { field } => Self::Conflict { field },
            rpc::SubmitError::IdReused => Self::IdReused,
            rpc::SubmitError::Expired => Self::Expired,
            rpc::SubmitError::Busy => Self::Busy,
            rpc::SubmitError::Unavailable => Self::Unavailable,
            rpc::SubmitError::Rejected { reason } => Self::Rejected { reason },
            rpc::SubmitError::NativeCapacityExceeded(report) => Self::NativeCapacityExceeded {
                report: report.into(),
            },
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum SubmitReply {
    Accepted {
        op: u64,
        record: Option<CoreOperationRecord>,
        state: Option<Box<StateReply>>,
    },
    Refused {
        reason: CoreSubmitRefusal,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OperationReply {
    pub record: Option<CoreOperationRecord>,
    pub state: Option<StateReply>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum CoreError {
    Unavailable {
        generation: u64,
        detail: String,
    },
    /// A command may have run. The caller queries its receipt; it never allocates a retry ID.
    Transport {
        generation: u64,
        detail: String,
    },
    ProtocolSkew {
        generation: u64,
        core_protocol: u32,
        app_protocol: u32,
    },
    InvalidInput {
        detail: String,
    },
    Refused {
        reason: CoreSubmitRefusal,
    },
}
impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable { detail, .. }
            | Self::Transport { detail, .. }
            | Self::InvalidInput { detail } => f.write_str(detail),
            Self::ProtocolSkew {
                core_protocol,
                app_protocol,
                ..
            } => write!(f, "protocol skew: core={core_protocol}, app={app_protocol}"),
            Self::Refused { reason } => write!(f, "core refused the request: {reason:?}"),
        }
    }
}
impl std::error::Error for CoreError {}
impl From<ClientError> for CoreError {
    fn from(value: ClientError) -> Self {
        match value {
            ClientError::Unavailable { generation, detail } => {
                Self::Unavailable { generation, detail }
            }
            ClientError::Transport { generation, detail } => Self::Transport { generation, detail },
            ClientError::ProtocolSkew {
                generation,
                core_protocol,
                app_protocol,
            } => Self::ProtocolSkew {
                generation,
                core_protocol,
                app_protocol,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreLogcatRequest {
    Start { session: u64 },
    Stop { session: u64 },
    Status,
    Ack { session: u64 },
    Discard { session: u64 },
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreLogcatError {
    Busy { running: u64 },
    UnknownSession,
    Disabled,
    Invalid { detail: String },
    Failed { detail: String },
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum LogcatReply {
    Ok {
        session: Option<CoreLogcatSessionStatus>,
    },
    Refused {
        reason: CoreLogcatError,
    },
}
impl From<rpc::LogcatRecordingError> for CoreLogcatError {
    fn from(value: rpc::LogcatRecordingError) -> Self {
        match value {
            rpc::LogcatRecordingError::Busy { running } => Self::Busy { running: running.0 },
            rpc::LogcatRecordingError::UnknownSession => Self::UnknownSession,
            rpc::LogcatRecordingError::Disabled => Self::Disabled,
            rpc::LogcatRecordingError::Invalid(detail) => Self::Invalid { detail },
            rpc::LogcatRecordingError::Failed(detail) => Self::Failed { detail },
        }
    }
}
