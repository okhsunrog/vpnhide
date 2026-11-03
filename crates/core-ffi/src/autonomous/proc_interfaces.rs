//! Typed proc observations; only Core evaluates differential evidence.
use super::*;
enum_map!(
    CoreProcInterfaceSurface,
    ProcInterfaceSurface,
    DeviceStats,
    Ipv6Addresses
);

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreNamesSample {
    Value { names: Vec<String> },
    Error { error: CoreProbeError },
}
impl From<model::Sample<Vec<String>>> for CoreNamesSample {
    fn from(v: model::Sample<Vec<String>>) -> Self {
        match v {
            model::Sample::Value(names) => Self::Value { names },
            model::Sample::Error(e) => Self::Error { error: e.into() },
        }
    }
}
impl From<CoreNamesSample> for model::Sample<Vec<String>> {
    fn from(v: CoreNamesSample) -> Self {
        match v {
            CoreNamesSample::Value { names } => Self::Value(names),
            CoreNamesSample::Error { error } => Self::Error(error.into()),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreProcInterfaceCheck {
    pub actor: CoreNativeReadActor,
    pub surface: CoreProcInterfaceSurface,
}
impl From<model::ProcInterfaceCheck> for CoreProcInterfaceCheck {
    fn from(v: model::ProcInterfaceCheck) -> Self {
        Self {
            actor: v.actor.into(),
            surface: v.surface.into(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreProcInterfaceActorMeasurement {
    pub actor: CoreNativeReadActor,
    pub devices: CoreNamesSample,
    pub ipv6: CoreAddressesSample,
}
impl From<model::ProcInterfaceActorMeasurement> for CoreProcInterfaceActorMeasurement {
    fn from(v: model::ProcInterfaceActorMeasurement) -> Self {
        Self {
            actor: v.actor.into(),
            devices: v.devices.into(),
            ipv6: v.ipv6.into(),
        }
    }
}
impl From<CoreProcInterfaceActorMeasurement> for model::ProcInterfaceActorMeasurement {
    fn from(v: CoreProcInterfaceActorMeasurement) -> Self {
        Self {
            actor: v.actor.into(),
            devices: v.devices.into(),
            ipv6: v.ipv6.into(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreProcInterfaceMeasurement {
    pub actors: Vec<CoreProcInterfaceActorMeasurement>,
}
impl From<model::ProcInterfaceMeasurement> for CoreProcInterfaceMeasurement {
    fn from(v: model::ProcInterfaceMeasurement) -> Self {
        Self {
            actors: v.actors.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<CoreProcInterfaceMeasurement> for model::ProcInterfaceMeasurement {
    fn from(v: CoreProcInterfaceMeasurement) -> Self {
        Self {
            actors: v.actors.into_iter().map(Into::into).collect(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreScopedProcInterfaceResult {
    pub proven: Vec<CoreProcInterfaceCheck>,
    pub leaked: Vec<CoreProcInterfaceCheck>,
    pub controls: Vec<CoreControlFailure>,
}
impl From<model::ScopedProcInterfaceResult> for CoreScopedProcInterfaceResult {
    fn from(v: model::ScopedProcInterfaceResult) -> Self {
        Self {
            proven: v.proven.into_iter().map(Into::into).collect(),
            leaked: v.leaked.into_iter().map(Into::into).collect(),
            controls: v.controls.into_iter().map(Into::into).collect(),
        }
    }
}
