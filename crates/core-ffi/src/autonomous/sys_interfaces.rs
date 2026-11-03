//! Typed directory evidence; evaluation remains exclusively in Core.
use super::*;
enum_map!(
    CoreSysInterfaceSurface,
    SysInterfaceSurface,
    SysClassNet,
    ProcIpv4Conf,
    ProcIpv6Conf
);
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreSysInterfaceCheck {
    pub actor: CoreNativeReadActor,
    pub surface: CoreSysInterfaceSurface,
}
impl From<model::SysInterfaceCheck> for CoreSysInterfaceCheck {
    fn from(v: model::SysInterfaceCheck) -> Self {
        Self {
            actor: v.actor.into(),
            surface: v.surface.into(),
        }
    }
}
impl From<CoreSysInterfaceCheck> for model::SysInterfaceCheck {
    fn from(v: CoreSysInterfaceCheck) -> Self {
        Self {
            actor: v.actor.into(),
            surface: v.surface.into(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreSysInterfaceObservation {
    pub check: CoreSysInterfaceCheck,
    pub names: CoreNamesSample,
}
impl From<model::SysInterfaceObservation> for CoreSysInterfaceObservation {
    fn from(v: model::SysInterfaceObservation) -> Self {
        Self {
            check: v.check.into(),
            names: v.names.into(),
        }
    }
}
impl From<CoreSysInterfaceObservation> for model::SysInterfaceObservation {
    fn from(v: CoreSysInterfaceObservation) -> Self {
        Self {
            check: v.check.into(),
            names: v.names.into(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreSysInterfaceMeasurement {
    pub entries: Vec<CoreSysInterfaceObservation>,
}
impl From<model::SysInterfaceMeasurement> for CoreSysInterfaceMeasurement {
    fn from(v: model::SysInterfaceMeasurement) -> Self {
        Self {
            entries: v.entries.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<CoreSysInterfaceMeasurement> for model::SysInterfaceMeasurement {
    fn from(v: CoreSysInterfaceMeasurement) -> Self {
        Self {
            entries: v.entries.into_iter().map(Into::into).collect(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreScopedSysInterfaceResult {
    pub proven: Vec<CoreSysInterfaceCheck>,
    pub leaked: Vec<CoreSysInterfaceCheck>,
    pub controls: Vec<CoreControlFailure>,
}
impl From<model::ScopedSysInterfaceResult> for CoreScopedSysInterfaceResult {
    fn from(v: model::ScopedSysInterfaceResult) -> Self {
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
    fn typed_boundary_preserves_independent_names_denial_and_late_control_failure() {
        let input = model::SysInterfaceMeasurement {
            entries: vec![
                model::SysInterfaceObservation {
                    check: model::SysInterfaceCheck {
                        actor: model::NativeReadActor::Libc,
                        surface: model::SysInterfaceSurface::SysClassNet,
                    },
                    names: model::Sample::Value(vec!["lo".into(), "tun000000000001".into()]),
                },
                model::SysInterfaceObservation {
                    check: model::SysInterfaceCheck {
                        actor: model::NativeReadActor::Syscall,
                        surface: model::SysInterfaceSurface::ProcIpv6Conf,
                    },
                    names: model::Sample::Error(model::ProbeError::Errno(13)),
                },
            ],
        };
        let restored: model::SysInterfaceMeasurement =
            CoreSysInterfaceMeasurement::from(input.clone()).into();
        assert_eq!(restored, input);
        assert!(restored.bounded());
        let result = CoreScopedSysInterfaceResult::from(model::ScopedSysInterfaceResult {
            proven: vec![],
            leaked: vec![input.entries[0].check],
            controls: vec![model::ControlFailure::CleanupUnconfirmed],
        });
        assert!(result.proven.is_empty());
        assert_eq!(result.leaked[0].actor, CoreNativeReadActor::Libc);
        assert_eq!(
            result.leaked[0].surface,
            CoreSysInterfaceSurface::SysClassNet
        );
        assert_eq!(result.controls, [CoreControlFailure::CleanupUnconfirmed]);
    }
}
