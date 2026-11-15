//! Typed route observations carried by the existing native worker exchange.
use super::*;

enum_map!(
    CoreRouteSurface,
    RouteSurface,
    NetlinkIpv4,
    NetlinkIpv6,
    ProcIpv4,
    ProcIpv6
);

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreRouteCheck {
    pub actor: CoreNativeReadActor,
    pub surface: CoreRouteSurface,
}
impl From<model::RouteCheck> for CoreRouteCheck {
    fn from(v: model::RouteCheck) -> Self {
        Self {
            actor: v.actor.into(),
            surface: v.surface.into(),
        }
    }
}
impl From<CoreRouteCheck> for model::RouteCheck {
    fn from(v: CoreRouteCheck) -> Self {
        Self {
            actor: v.actor.into(),
            surface: v.surface.into(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreObservedRoute {
    pub destination: Vec<u8>,
    pub prefix: u8,
    pub source: Option<Vec<u8>>,
    pub source_prefix: u8,
    pub output_index: Option<u32>,
    pub output_name: Option<String>,
    pub table: Option<u32>,
    pub protocol: Option<u8>,
    pub scope: Option<u8>,
    pub kind: Option<u8>,
    pub metric: u32,
    pub gateway: Option<Vec<u8>>,
    pub flags: u32,
}
impl From<model::ObservedRoute> for CoreObservedRoute {
    fn from(v: model::ObservedRoute) -> Self {
        Self {
            destination: v.destination,
            prefix: v.prefix,
            source: v.source,
            source_prefix: v.source_prefix,
            output_index: v.output_index,
            output_name: v.output_name,
            table: v.table,
            protocol: v.protocol,
            scope: v.scope,
            kind: v.kind,
            metric: v.metric,
            gateway: v.gateway,
            flags: v.flags,
        }
    }
}
impl From<CoreObservedRoute> for model::ObservedRoute {
    fn from(v: CoreObservedRoute) -> Self {
        Self {
            destination: v.destination,
            prefix: v.prefix,
            source: v.source,
            source_prefix: v.source_prefix,
            output_index: v.output_index,
            output_name: v.output_name,
            table: v.table,
            protocol: v.protocol,
            scope: v.scope,
            kind: v.kind,
            metric: v.metric,
            gateway: v.gateway,
            flags: v.flags,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreRouteObservation {
    pub check: CoreRouteCheck,
    pub routes: Vec<CoreObservedRoute>,
    pub error: Option<CoreProbeError>,
}
impl From<model::RouteObservation> for CoreRouteObservation {
    fn from(v: model::RouteObservation) -> Self {
        Self {
            check: v.check.into(),
            routes: v.routes.into_iter().map(Into::into).collect(),
            error: v.error.map(Into::into),
        }
    }
}
impl From<CoreRouteObservation> for model::RouteObservation {
    fn from(v: CoreRouteObservation) -> Self {
        Self {
            check: v.check.into(),
            routes: v.routes.into_iter().map(Into::into).collect(),
            error: v.error.map(Into::into),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreRouteMeasurement {
    pub observations: Vec<CoreRouteObservation>,
}
impl From<model::RouteMeasurement> for CoreRouteMeasurement {
    fn from(v: model::RouteMeasurement) -> Self {
        Self {
            observations: v.observations.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<CoreRouteMeasurement> for model::RouteMeasurement {
    fn from(v: CoreRouteMeasurement) -> Self {
        Self {
            observations: v.observations.into_iter().map(Into::into).collect(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreFixtureInterface {
    pub name: String,
    pub index: u32,
}
impl From<model::InterfaceId> for CoreFixtureInterface {
    fn from(v: model::InterfaceId) -> Self {
        Self {
            name: v.name,
            index: v.index,
        }
    }
}
impl From<CoreFixtureInterface> for model::InterfaceId {
    fn from(v: CoreFixtureInterface) -> Self {
        Self {
            name: v.name,
            index: v.index,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreRouteFixtureSpec {
    pub session: u64,
    pub vpn: CoreFixtureInterface,
    pub ordinary: CoreFixtureInterface,
}
impl From<model::RouteFixtureSpec> for CoreRouteFixtureSpec {
    fn from(v: model::RouteFixtureSpec) -> Self {
        Self {
            session: v.session,
            vpn: v.vpn.into(),
            ordinary: v.ordinary.into(),
        }
    }
}
impl From<CoreRouteFixtureSpec> for model::RouteFixtureSpec {
    fn from(v: CoreRouteFixtureSpec) -> Self {
        Self {
            session: v.session,
            vpn: v.vpn.into(),
            ordinary: v.ordinary.into(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreScopedRouteResult {
    pub proven: Vec<CoreRouteCheck>,
    pub leaked: Vec<CoreRouteCheck>,
    pub controls: Vec<CoreControlFailure>,
}
impl From<model::ScopedRouteResult> for CoreScopedRouteResult {
    fn from(v: model::ScopedRouteResult) -> Self {
        Self {
            proven: v.proven.into_iter().map(Into::into).collect(),
            leaked: v.leaked.into_iter().map(Into::into).collect(),
            controls: v.controls.into_iter().map(Into::into).collect(),
        }
    }
}
