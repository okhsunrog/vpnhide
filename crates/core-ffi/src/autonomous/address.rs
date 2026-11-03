//! Generated observation records only. Core evaluates every required actor.
use super::*;
enum_map!(CoreAddressActor, AddressActor, Libc, Syscall);
enum_map!(
    CoreAddressIoctl,
    AddressIoctl,
    Address,
    Netmask,
    Broadcast,
    Destination
);
enum_map!(
    CoreAddressVector,
    AddressVector,
    IoctlAddress,
    IoctlNetmask,
    IoctlBroadcast,
    IoctlDestination,
    IfconfData,
    IfconfSize,
    GetifaddrsIpv4,
    GetifaddrsIpv6,
    NetlinkIpv4,
    NetlinkIpv6
);

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreAddressCheck {
    pub actor: CoreAddressActor,
    pub vector: CoreAddressVector,
}
impl From<model::AddressCheck> for CoreAddressCheck {
    fn from(v: model::AddressCheck) -> Self {
        Self {
            actor: v.actor.into(),
            vector: v.vector.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreObservedAddress {
    pub name: Option<String>,
    pub index: Option<u32>,
    pub address: Vec<u8>,
    pub prefix: Option<u8>,
}
impl From<model::ObservedAddress> for CoreObservedAddress {
    fn from(v: model::ObservedAddress) -> Self {
        Self {
            name: v.name,
            index: v.index,
            address: v.address,
            prefix: v.prefix,
        }
    }
}
impl From<CoreObservedAddress> for model::ObservedAddress {
    fn from(v: CoreObservedAddress) -> Self {
        Self {
            name: v.name,
            index: v.index,
            address: v.address,
            prefix: v.prefix,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreBytesSample {
    Value { value: Vec<u8> },
    Error { error: CoreProbeError },
}
impl From<model::Sample<Vec<u8>>> for CoreBytesSample {
    fn from(v: model::Sample<Vec<u8>>) -> Self {
        match v {
            model::Sample::Value(value) => Self::Value { value },
            model::Sample::Error(error) => Self::Error {
                error: error.into(),
            },
        }
    }
}
impl From<CoreBytesSample> for model::Sample<Vec<u8>> {
    fn from(v: CoreBytesSample) -> Self {
        match v {
            CoreBytesSample::Value { value } => Self::Value(value),
            CoreBytesSample::Error { error } => Self::Error(error.into()),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreAddressesSample {
    Value { addresses: Vec<CoreObservedAddress> },
    Error { error: CoreProbeError },
}
impl From<model::Sample<Vec<model::ObservedAddress>>> for CoreAddressesSample {
    fn from(v: model::Sample<Vec<model::ObservedAddress>>) -> Self {
        match v {
            model::Sample::Value(addresses) => Self::Value {
                addresses: addresses.into_iter().map(Into::into).collect(),
            },
            model::Sample::Error(error) => Self::Error {
                error: error.into(),
            },
        }
    }
}
impl From<CoreAddressesSample> for model::Sample<Vec<model::ObservedAddress>> {
    fn from(v: CoreAddressesSample) -> Self {
        match v {
            CoreAddressesSample::Value { addresses } => {
                Self::Value(addresses.into_iter().map(Into::into).collect())
            }
            CoreAddressesSample::Error { error } => Self::Error(error.into()),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreAddressIoctlObservation {
    pub kind: CoreAddressIoctl,
    pub fixture: CoreBytesSample,
    pub ordinary: CoreBytesSample,
}
impl From<model::AddressIoctlObservation> for CoreAddressIoctlObservation {
    fn from(v: model::AddressIoctlObservation) -> Self {
        Self {
            kind: v.kind.into(),
            fixture: v.fixture.into(),
            ordinary: v.ordinary.into(),
        }
    }
}
impl From<CoreAddressIoctlObservation> for model::AddressIoctlObservation {
    fn from(v: CoreAddressIoctlObservation) -> Self {
        Self {
            kind: v.kind.into(),
            fixture: v.fixture.into(),
            ordinary: v.ordinary.into(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreIfconfObservation {
    pub entries: Vec<CoreObservedAddress>,
    pub record_bytes: u32,
    pub used_bytes: u32,
    pub size_before: u32,
    pub size_after: u32,
}
impl From<model::IfconfObservation> for CoreIfconfObservation {
    fn from(v: model::IfconfObservation) -> Self {
        Self {
            entries: v.entries.into_iter().map(Into::into).collect(),
            record_bytes: v.record_bytes,
            used_bytes: v.used_bytes,
            size_before: v.size_before,
            size_after: v.size_after,
        }
    }
}
impl From<CoreIfconfObservation> for model::IfconfObservation {
    fn from(v: CoreIfconfObservation) -> Self {
        Self {
            entries: v.entries.into_iter().map(Into::into).collect(),
            record_bytes: v.record_bytes,
            used_bytes: v.used_bytes,
            size_before: v.size_before,
            size_after: v.size_after,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreIfconfSample {
    Value { value: CoreIfconfObservation },
    Error { error: CoreProbeError },
}
impl From<model::Sample<model::IfconfObservation>> for CoreIfconfSample {
    fn from(v: model::Sample<model::IfconfObservation>) -> Self {
        match v {
            model::Sample::Value(value) => Self::Value {
                value: value.into(),
            },
            model::Sample::Error(error) => Self::Error {
                error: error.into(),
            },
        }
    }
}
impl From<CoreIfconfSample> for model::Sample<model::IfconfObservation> {
    fn from(v: CoreIfconfSample) -> Self {
        match v {
            CoreIfconfSample::Value { value } => Self::Value(value.into()),
            CoreIfconfSample::Error { error } => Self::Error(error.into()),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreAddressActorMeasurement {
    pub actor: CoreAddressActor,
    pub ioctls: Vec<CoreAddressIoctlObservation>,
    pub ifconf: CoreIfconfSample,
    pub netlink: CoreAddressesSample,
}
impl From<model::AddressActorMeasurement> for CoreAddressActorMeasurement {
    fn from(v: model::AddressActorMeasurement) -> Self {
        Self {
            actor: v.actor.into(),
            ioctls: v.ioctls.into_iter().map(Into::into).collect(),
            ifconf: v.ifconf.into(),
            netlink: v.netlink.into(),
        }
    }
}
impl From<CoreAddressActorMeasurement> for model::AddressActorMeasurement {
    fn from(v: CoreAddressActorMeasurement) -> Self {
        Self {
            actor: v.actor.into(),
            ioctls: v.ioctls.into_iter().map(Into::into).collect(),
            ifconf: v.ifconf.into(),
            netlink: v.netlink.into(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreAddressMeasurement {
    pub actors: Vec<CoreAddressActorMeasurement>,
    pub ifaddrs: CoreAddressesSample,
}
impl From<model::AddressMeasurement> for CoreAddressMeasurement {
    fn from(v: model::AddressMeasurement) -> Self {
        Self {
            actors: v.actors.into_iter().map(Into::into).collect(),
            ifaddrs: v.ifaddrs.into(),
        }
    }
}
impl From<CoreAddressMeasurement> for model::AddressMeasurement {
    fn from(v: CoreAddressMeasurement) -> Self {
        Self {
            actors: v.actors.into_iter().map(Into::into).collect(),
            ifaddrs: v.ifaddrs.into(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreScopedAddressResult {
    pub proven: Vec<CoreAddressCheck>,
    pub leaked: Vec<CoreAddressCheck>,
    pub controls: Vec<CoreControlFailure>,
}
impl From<model::ScopedAddressResult> for CoreScopedAddressResult {
    fn from(v: model::ScopedAddressResult) -> Self {
        Self {
            proven: v.proven.into_iter().map(Into::into).collect(),
            leaked: v.leaked.into_iter().map(Into::into).collect(),
            controls: v.controls.into_iter().map(Into::into).collect(),
        }
    }
}
