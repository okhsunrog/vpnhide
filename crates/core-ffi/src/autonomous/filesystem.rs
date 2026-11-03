//! Typed lookup observations and retained Core assessment; no APK verdict.
use super::*;
use crate::CoreBooleanSample;
enum_map!(
    CoreFilesystemVector,
    FilesystemVector,
    Stat,
    Access,
    OpenDirectory
);
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreFilesystemCheck {
    pub actor: CoreNativeReadActor,
    pub surface: CoreSysInterfaceSurface,
    pub vector: CoreFilesystemVector,
}
impl From<model::FilesystemCheck> for CoreFilesystemCheck {
    fn from(v: model::FilesystemCheck) -> Self {
        Self {
            actor: v.actor.into(),
            surface: v.surface.into(),
            vector: v.vector.into(),
        }
    }
}
impl From<CoreFilesystemCheck> for model::FilesystemCheck {
    fn from(v: CoreFilesystemCheck) -> Self {
        Self {
            actor: v.actor.into(),
            surface: v.surface.into(),
            vector: v.vector.into(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreFilesystemObservation {
    pub check: CoreFilesystemCheck,
    pub fixture: CoreBooleanSample,
    pub ordinary: CoreBooleanSample,
}
impl From<model::FilesystemObservation> for CoreFilesystemObservation {
    fn from(v: model::FilesystemObservation) -> Self {
        Self {
            check: v.check.into(),
            fixture: v.fixture.into(),
            ordinary: v.ordinary.into(),
        }
    }
}
impl From<CoreFilesystemObservation> for model::FilesystemObservation {
    fn from(v: CoreFilesystemObservation) -> Self {
        Self {
            check: v.check.into(),
            fixture: v.fixture.into(),
            ordinary: v.ordinary.into(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreFilesystemMeasurement {
    pub entries: Vec<CoreFilesystemObservation>,
}
impl From<model::FilesystemMeasurement> for CoreFilesystemMeasurement {
    fn from(v: model::FilesystemMeasurement) -> Self {
        Self {
            entries: v.entries.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<CoreFilesystemMeasurement> for model::FilesystemMeasurement {
    fn from(v: CoreFilesystemMeasurement) -> Self {
        Self {
            entries: v.entries.into_iter().map(Into::into).collect(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreScopedFilesystemResult {
    pub proven: Vec<CoreFilesystemCheck>,
    pub leaked: Vec<CoreFilesystemCheck>,
    pub controls: Vec<CoreControlFailure>,
}
impl From<model::ScopedFilesystemResult> for CoreScopedFilesystemResult {
    fn from(v: model::ScopedFilesystemResult) -> Self {
        Self {
            proven: v.proven.into_iter().map(Into::into).collect(),
            leaked: v.leaked.into_iter().map(Into::into).collect(),
            controls: v.controls.into_iter().map(Into::into).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_lookup_boundary_preserves_actor_vector_and_denied_ordinary_control() {
        let input = model::FilesystemMeasurement {
            entries: vec![model::FilesystemObservation {
                check: model::FilesystemCheck {
                    actor: model::NativeReadActor::Syscall,
                    surface: model::SysInterfaceSurface::ProcIpv6Conf,
                    vector: model::FilesystemVector::Stat,
                },
                fixture: model::Sample::Value(true),
                ordinary: model::Sample::Error(model::ProbeError::Errno(13)),
            }],
        };
        let restored: model::FilesystemMeasurement =
            CoreFilesystemMeasurement::from(input.clone()).into();
        assert_eq!(restored, input);
        assert!(restored.bounded());
        let result = CoreScopedFilesystemResult::from(model::ScopedFilesystemResult {
            proven: vec![],
            leaked: vec![input.entries[0].check],
            controls: vec![model::ControlFailure::CleanupUnconfirmed],
        });
        assert!(result.proven.is_empty());
        assert_eq!(result.leaked[0].vector, CoreFilesystemVector::Stat);
        assert_eq!(result.controls, [CoreControlFailure::CleanupUnconfirmed]);
    }
}
