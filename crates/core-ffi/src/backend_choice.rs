//! Typed device-local intent; no private journal or root engine linked into APK.
use crate::CoreNativeBackend;
use vpnhide_core_rpc as rpc;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreBackendChoice {
    Auto,
    Fixed { backend: CoreNativeBackend },
}
impl From<CoreBackendChoice> for rpc::BackendChoice {
    fn from(value: CoreBackendChoice) -> Self {
        match value {
            CoreBackendChoice::Auto => Self::Auto,
            CoreBackendChoice::Fixed { backend } => Self::Fixed {
                backend: backend.into(),
            },
        }
    }
}
impl From<rpc::BackendChoice> for CoreBackendChoice {
    fn from(value: rpc::BackendChoice) -> Self {
        match value {
            rpc::BackendChoice::Auto => Self::Auto,
            rpc::BackendChoice::Fixed { backend } => Self::Fixed {
                backend: backend.into(),
            },
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CoreZygiskChoiceWarning {
    ProcessInjectionV1,
}
impl From<CoreZygiskChoiceWarning> for rpc::ZygiskChoiceWarning {
    fn from(value: CoreZygiskChoiceWarning) -> Self {
        match value {
            CoreZygiskChoiceWarning::ProcessInjectionV1 => Self::ProcessInjectionV1,
        }
    }
}
impl From<rpc::ZygiskChoiceWarning> for CoreZygiskChoiceWarning {
    fn from(value: rpc::ZygiskChoiceWarning) -> Self {
        match value {
            rpc::ZygiskChoiceWarning::ProcessInjectionV1 => Self::ProcessInjectionV1,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreBackendChoiceRequest {
    pub expected_revision: u64,
    pub choice: CoreBackendChoice,
    pub acknowledged_warning: Option<CoreZygiskChoiceWarning>,
}
impl From<CoreBackendChoiceRequest> for rpc::BackendChoiceRequest {
    fn from(value: CoreBackendChoiceRequest) -> Self {
        Self {
            expected_revision: value.expected_revision,
            choice: value.choice.into(),
            acknowledged_warning: value.acknowledged_warning.map(Into::into),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CoreBackendChoiceStage {
    Current,
    PendingRestart,
    Unverified,
}
impl From<rpc::BackendChoiceStage> for CoreBackendChoiceStage {
    fn from(value: rpc::BackendChoiceStage) -> Self {
        match value {
            rpc::BackendChoiceStage::Current => Self::Current,
            rpc::BackendChoiceStage::PendingRestart => Self::PendingRestart,
            rpc::BackendChoiceStage::Unverified => Self::Unverified,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreBackendChoiceStatus {
    pub revision: u64,
    pub operation: Option<u64>,
    pub desired: CoreBackendChoice,
    pub requested_in_boot: Option<String>,
    pub effective: Option<CoreNativeBackend>,
    pub capacity_backends: Vec<CoreNativeBackend>,
    pub stage: CoreBackendChoiceStage,
    pub options: Vec<CoreBackendChoice>,
}
impl From<rpc::BackendChoiceStatus> for CoreBackendChoiceStatus {
    fn from(value: rpc::BackendChoiceStatus) -> Self {
        Self {
            revision: value.revision,
            operation: value.operation.map(|op| op.0),
            desired: value.desired.into(),
            requested_in_boot: value.requested_in_boot,
            effective: value.effective.map(Into::into),
            capacity_backends: value
                .capacity_backends
                .into_iter()
                .map(Into::into)
                .collect(),
            stage: value.stage.into(),
            options: value.options.into_iter().map(Into::into).collect(),
        }
    }
}

#[uniffi::export]
pub fn backend_choice_request_confirmed(
    status: CoreBackendChoiceStatus,
    operation: u64,
    request: CoreBackendChoiceRequest,
) -> bool {
    let valid = rpc::BackendChoiceRequest::from(request.clone())
        .validate()
        .is_ok();
    valid
        && status.operation == Some(operation)
        && request.expected_revision.checked_add(1) == Some(status.revision)
        && status.desired == request.choice
}
