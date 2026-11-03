//! Typed worker handshake and retained Core-owned RAW attribution. An app reports
//! observations, never a grant/cleanup verdict or a replacement UID policy.
use crate::NativeMeasurement;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticThreadIdentity {
    pub uid: u32,
    pub pid: u32,
    pub process_start_ticks: u64,
    pub tid: u32,
    pub thread_start_ticks: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeWorkerPhase {
    Prepared,
    Registered,
    RawMeasured { native: Box<NativeMeasurement> },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeWorkerRequest {
    pub session: u64,
    pub core_epoch: u64,
    pub tid: u32,
    pub phase: NativeWorkerPhase,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "task", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeWorkerInstruction {
    Register { nonce: u64 },
    ObserveRaw,
    // Only issued after confirmed revoke, or when no RAW grant was attempted.
    ObserveProtected,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeRawStage {
    Arm,
    Registration,
    Activation,
    Measurement,
    Revocation,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeRawReceipt {
    UnsupportedBackend {
        identity: DiagnosticThreadIdentity,
    },
    CapabilityAbsent {
        identity: DiagnosticThreadIdentity,
    },
    Failed {
        identity: DiagnosticThreadIdentity,
        stage: NativeRawStage,
        // Preserve a completed observation even when its interval or cleanup
        // cannot be qualified. It must never become a protected observation.
        sample: Option<NativeMeasurement>,
    },
    Paired {
        identity: DiagnosticThreadIdentity,
        sample: NativeMeasurement,
    },
}
impl NativeRawReceipt {
    pub fn qualified_sample(&self) -> Option<&NativeMeasurement> {
        match self {
            Self::Paired { identity, sample }
                if identity.uid >= 10_000
                    && identity.pid > 0
                    && identity.tid > 0
                    && identity.process_start_ticks > 0
                    && identity.thread_start_ticks > 0
                    && sample.bounded() =>
            {
                Some(sample)
            }
            _ => None,
        }
    }
}
