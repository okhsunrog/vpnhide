//! Local collector adapters and typed Core phase exchange, not a verdict owner.
use super::*;
enum_map!(
    CoreNativeRawStage,
    NativeRawStage,
    Arm,
    Registration,
    Activation,
    Measurement,
    Revocation
);
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreDiagnosticThreadIdentity {
    pub uid: u32,
    pub pid: u32,
    pub process_start_ticks: u64,
    pub tid: u32,
    pub thread_start_ticks: u64,
}
impl From<model::DiagnosticThreadIdentity> for CoreDiagnosticThreadIdentity {
    fn from(v: model::DiagnosticThreadIdentity) -> Self {
        Self {
            uid: v.uid,
            pid: v.pid,
            process_start_ticks: v.process_start_ticks,
            tid: v.tid,
            thread_start_ticks: v.thread_start_ticks,
        }
    }
}
impl From<CoreDiagnosticThreadIdentity> for model::DiagnosticThreadIdentity {
    fn from(v: CoreDiagnosticThreadIdentity) -> Self {
        Self {
            uid: v.uid,
            pid: v.pid,
            process_start_ticks: v.process_start_ticks,
            tid: v.tid,
            thread_start_ticks: v.thread_start_ticks,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreNativeRawReceipt {
    UnsupportedBackend {
        identity: CoreDiagnosticThreadIdentity,
    },
    CapabilityAbsent {
        identity: CoreDiagnosticThreadIdentity,
    },
    Failed {
        identity: CoreDiagnosticThreadIdentity,
        stage: CoreNativeRawStage,
        sample: Option<CoreNativeMeasurement>,
    },
    Paired {
        identity: CoreDiagnosticThreadIdentity,
        sample: CoreNativeMeasurement,
    },
}
impl From<model::NativeRawReceipt> for CoreNativeRawReceipt {
    fn from(v: model::NativeRawReceipt) -> Self {
        match v {
            model::NativeRawReceipt::UnsupportedBackend { identity } => Self::UnsupportedBackend {
                identity: identity.into(),
            },
            model::NativeRawReceipt::CapabilityAbsent { identity } => Self::CapabilityAbsent {
                identity: identity.into(),
            },
            model::NativeRawReceipt::Failed {
                identity,
                stage,
                sample,
            } => Self::Failed {
                identity: identity.into(),
                stage: stage.into(),
                sample: sample.map(Into::into),
            },
            model::NativeRawReceipt::Paired { identity, sample } => Self::Paired {
                identity: identity.into(),
                sample: sample.into(),
            },
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreNativeWorkerPhase {
    Prepared,
    Registered,
    RawMeasured { native: Box<CoreNativeMeasurement> },
}
impl From<CoreNativeWorkerPhase> for model::NativeWorkerPhase {
    fn from(v: CoreNativeWorkerPhase) -> Self {
        match v {
            CoreNativeWorkerPhase::Prepared => Self::Prepared,
            CoreNativeWorkerPhase::Registered => Self::Registered,
            CoreNativeWorkerPhase::RawMeasured { native } => Self::RawMeasured {
                native: Box::new((*native).into()),
            },
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreNativeWorkerInstruction {
    Register { nonce: u64 },
    ObserveRaw,
    ObserveProtected,
}
impl From<model::NativeWorkerInstruction> for CoreNativeWorkerInstruction {
    fn from(v: model::NativeWorkerInstruction) -> Self {
        match v {
            model::NativeWorkerInstruction::Register { nonce } => Self::Register { nonce },
            model::NativeWorkerInstruction::ObserveRaw => Self::ObserveRaw,
            model::NativeWorkerInstruction::ObserveProtected => Self::ObserveProtected,
        }
    }
}
#[uniffi::export]
pub fn register_diagnostic_raw(nonce: u64) -> Result<(), CoreError> {
    vpnhide_checks::diagnostic_raw::register(nonce).map_err(|error| CoreError::InvalidInput {
        detail: format!("native RAW registration failed: {error}"),
    })
}
#[uniffi::export(async_runtime = "tokio")]
impl CoreSession {
    pub async fn finish_diagnostic_worker(
        &self,
        task: CoreDiagnosticWorkerTask,
    ) -> Result<(), CoreDiagnosticError> {
        self.client
            .finish_diagnostic_worker(task.session, task.core_epoch)
            .await?
            .map_err(Into::into)
    }

    pub async fn native_worker_step(
        &self,
        task: CoreDiagnosticWorkerTask,
        tid: u32,
        phase: CoreNativeWorkerPhase,
    ) -> Result<CoreNativeWorkerInstruction, CoreDiagnosticError> {
        self.client
            .native_worker_step(model::NativeWorkerRequest {
                session: task.session,
                core_epoch: task.core_epoch,
                tid,
                phase: phase.into(),
            })
            .await?
            .map(Into::into)
            .map_err(Into::into)
    }
}

/// Forensic export only: preserve the complete retained Rust receipt without a
/// parallel Kotlin measurement DTO/codec. This is not a control/RPC JSON API.
#[uniffi::export]
pub fn native_raw_receipt_json(receipt: CoreNativeRawReceipt) -> String {
    let receipt = match receipt {
        CoreNativeRawReceipt::UnsupportedBackend { identity } => {
            model::NativeRawReceipt::UnsupportedBackend {
                identity: identity.into(),
            }
        }
        CoreNativeRawReceipt::CapabilityAbsent { identity } => {
            model::NativeRawReceipt::CapabilityAbsent {
                identity: identity.into(),
            }
        }
        CoreNativeRawReceipt::Failed {
            identity,
            stage,
            sample,
        } => model::NativeRawReceipt::Failed {
            identity: identity.into(),
            stage: stage.into(),
            sample: sample.map(Into::into),
        },
        CoreNativeRawReceipt::Paired { identity, sample } => model::NativeRawReceipt::Paired {
            identity: identity.into(),
            sample: sample.into(),
        },
    };
    serde_json::to_string(&receipt).expect("bounded diagnostic receipt codec")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn identity() -> model::DiagnosticThreadIdentity {
        model::DiagnosticThreadIdentity {
            uid: 10000,
            pid: 42,
            process_start_ticks: 100,
            tid: 43,
            thread_start_ticks: 101,
        }
    }
    #[test]
    fn forensic_receipt_uses_the_shared_schema_and_preserves_failure_observations() {
        assert_eq!(
            native_raw_receipt_json(CoreNativeRawReceipt::CapabilityAbsent {
                identity: identity().into()
            }),
            r#"{"state":"capability_absent","identity":{"uid":10000,"pid":42,"process_start_ticks":100,"tid":43,"thread_start_ticks":101}}"#
        );
        let sample = model::NativeMeasurement::unavailable(model::ProbeError::Timeout);
        let wire = native_raw_receipt_json(CoreNativeRawReceipt::Failed {
            identity: identity().into(),
            stage: CoreNativeRawStage::Revocation,
            sample: Some(sample.clone().into()),
        });
        let receipt: model::NativeRawReceipt = serde_json::from_str(&wire).unwrap();
        assert_eq!(
            receipt,
            model::NativeRawReceipt::Failed {
                identity: identity(),
                stage: model::NativeRawStage::Revocation,
                sample: Some(sample),
            }
        );
        assert!(receipt.qualified_sample().is_none());
    }
}
