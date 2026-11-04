//! Device-local backend intent is separate from portable canonical settings.
use crate::{NativeBackend, OperationId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendChoice {
    #[default]
    Auto,
    Fixed {
        backend: NativeBackend,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZygiskChoiceWarning {
    ProcessInjectionV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendChoiceRequest {
    pub expected_revision: u64,
    pub choice: BackendChoice,
    pub acknowledged_warning: Option<ZygiskChoiceWarning>,
}

impl BackendChoiceRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        match (self.choice, self.acknowledged_warning) {
            (
                BackendChoice::Fixed {
                    backend: NativeBackend::Zygisk,
                },
                Some(ZygiskChoiceWarning::ProcessInjectionV1),
            ) => Ok(()),
            (
                BackendChoice::Fixed {
                    backend: NativeBackend::Zygisk,
                },
                None,
            ) => Err("zygisk_injection_warning_required"),
            (_, None) => Ok(()),
            (_, Some(_)) => Err("unexpected_zygisk_warning_acknowledgement"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackendChoiceStage {
    Current,
    PendingRestart,
    Unverified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendChoiceStatus {
    pub revision: u64,
    pub operation: Option<OperationId>,
    pub desired: BackendChoice,
    pub requested_in_boot: Option<String>,
    /// Confirmed delivery family; runtime and behavioral health remain separate.
    pub effective: Option<NativeBackend>,
    /// The shared local picker preview also enforces the staged desired family.
    pub capacity_backends: Vec<NativeBackend>,
    pub stage: BackendChoiceStage,
    /// Implemented staging paths, distinct from point-in-time applicability.
    pub options: Vec<BackendChoice>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injection_warning_is_specific_to_an_explicit_zygisk_choice() {
        for choice in [
            BackendChoice::Auto,
            BackendChoice::Fixed {
                backend: NativeBackend::Builtin,
            },
            BackendChoice::Fixed {
                backend: NativeBackend::Kmod,
            },
            BackendChoice::Fixed {
                backend: NativeBackend::Kpm,
            },
            BackendChoice::Fixed {
                backend: NativeBackend::Zygisk,
            },
        ] {
            for warning in [None, Some(ZygiskChoiceWarning::ProcessInjectionV1)] {
                let request = BackendChoiceRequest {
                    expected_revision: 0,
                    choice,
                    acknowledged_warning: warning,
                };
                assert_eq!(
                    request.validate().is_ok(),
                    matches!(
                        choice,
                        BackendChoice::Fixed {
                            backend: NativeBackend::Zygisk
                        }
                    ) == warning.is_some()
                );
                assert_eq!(
                    serde_json::from_slice::<BackendChoiceRequest>(
                        &serde_json::to_vec(&request).unwrap()
                    )
                    .unwrap(),
                    request
                );
            }
        }
    }
}
