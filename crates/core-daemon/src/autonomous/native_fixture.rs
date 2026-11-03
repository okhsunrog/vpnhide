//! Real nonpersistent, exclusively created TUN. It remains DOWN, with bounded
//! owned addresses and no prefix routes, policy rules or packet IO. Close owns cleanup; never
//! delete a possibly-replaced interface by its pathname/name.

use std::fs::{File, OpenOptions};
use std::io;
use std::mem::zeroed;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};

use vpnhide_checks::fixture;
use vpnhide_diagnostic_model::{
    InterfaceId, NativeFixtureSpec, NativeMeasurement, OrdinaryControl, ProbeError, Sample,
    ScopedNativeResult, evaluate_native_fixture,
};

const FIXTURE_MTU: i32 = 1543;
mod addresses;
const TUNSETIFF: u32 = 0x4004_54ca;
const TUN_FLAGS: u16 = 0x0001 | 0x1000 | 0x8000; // TUN | NO_PI | TUN_EXCL

fn invalid(message: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

pub fn fixture_name(session: u64) -> io::Result<String> {
    if session == 0 {
        return Err(invalid("missing diagnostic session"));
    }
    Ok(format!("tun{:012x}", session & 0xffff_ffff_ffff))
}

fn tun_device() -> io::Result<File> {
    // Android's VPN JNI uses /dev/tun; ordinary Linux uses /dev/net/tun.
    // https://android.googlesource.com/platform/frameworks/base/+/31b731c2de27/services/core/jni/com_android_server_connectivity_Vpn.cpp
    for path in ["/dev/tun", "/dev/net/tun"] {
        match OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
        {
            Ok(file) => {
                validate_device(&file)?;
                return Ok(file);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "TUN device is unavailable",
    ))
}

fn validate_device(file: &File) -> io::Result<()> {
    let metadata = file.metadata()?;
    // Linux miscellaneous character device 10:200, equal in both stat ABIs.
    if !metadata.file_type().is_char_device() || metadata.rdev() != 0x0ac8 {
        return Err(invalid("not the TUN character device"));
    }
    Ok(())
}

fn request(name: &str) -> libc::ifreq {
    let mut request: libc::ifreq = unsafe { zeroed() };
    for (target, byte) in request.ifr_name.iter_mut().zip(name.bytes()) {
        *target = byte as libc::c_char;
    }
    request
}

fn value(sample: &Sample<u32>) -> io::Result<u32> {
    sample
        .value()
        .copied()
        .ok_or_else(|| invalid(format!("native control unavailable: {sample:?}")))
}

/// Setup can fail after allocation. Preserve enough identity to verify cleanup
/// after local FDs have dropped; unknown identity holds the diagnostic resource.
#[derive(Default)]
struct SetupOwnership {
    created: bool,
    spec: Option<NativeFixtureSpec>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum SetupFault {
    None,
    #[cfg(feature = "fixture-oracle")]
    AfterAllocation,
    #[cfg(feature = "fixture-oracle")]
    AfterIpv4,
}
pub struct FixtureCreateFailure {
    pub error: io::Error,
    pub resource_created: bool,
    pub cleanup: Option<(NativeFixtureSpec, bool)>,
}

pub struct TunFixture {
    pub address_setup: Result<(), ProbeError>,
    tun: Option<File>,
    pub spec: NativeFixtureSpec,
    pub negative: NativeMeasurement,
    pub root_start: NativeMeasurement,
}

pub struct NativeCompletion {
    pub addresses: vpnhide_diagnostic_model::ScopedAddressResult,
    pub root_end: NativeMeasurement,
    pub cleanup: NativeMeasurement,
    pub result: ScopedNativeResult,
    pub binding: vpnhide_diagnostic_model::ScopedBindResult,
}

impl TunFixture {
    pub fn create(session: u64) -> io::Result<Self> {
        Self::create_detailed(session).map_err(|failure| failure.error)
    }
    pub fn create_detailed(session: u64) -> Result<Self, FixtureCreateFailure> {
        Self::create_detailed_inner(session, SetupFault::None)
    }
    fn create_detailed_inner(
        session: u64,
        fault: SetupFault,
    ) -> Result<Self, FixtureCreateFailure> {
        let mut ownership = SetupOwnership::default();
        Self::create_owned(session, &mut ownership, fault).map_err(|error| {
            let cleanup = ownership.spec.map(|spec| {
                let observed = fixture::observe(&spec.interface.name);
                let cleaned = vpnhide_diagnostic_model::cleanup_confirmed(&spec, &observed);
                (spec, cleaned)
            });
            FixtureCreateFailure {
                error,
                resource_created: ownership.created,
                cleanup,
            }
        })
    }
    #[cfg(feature = "fixture-oracle")]
    pub fn failed_setup_for_oracle(session: u64) -> FixtureCreateFailure {
        match Self::create_detailed_inner(session, SetupFault::AfterAllocation) {
            Err(failure) => failure,
            Ok(_) => panic!("setup fault did not fire"),
        }
    }
    #[cfg(feature = "fixture-oracle")]
    pub fn partial_address_setup_for_oracle(session: u64) -> io::Result<Self> {
        Self::create_detailed_inner(session, SetupFault::AfterIpv4).map_err(|v| v.error)
    }
    fn create_owned(
        session: u64,
        ownership: &mut SetupOwnership,
        fault: SetupFault,
    ) -> io::Result<Self> {
        let name = fixture_name(session)?;
        let negative = fixture::observe(&name);
        let absent = |v: &Sample<u32>| {
            matches!(
                v,
                Sample::Error(ProbeError::Errno(libc::ENODEV | libc::ENXIO))
            )
        };
        if !absent(&negative.flags)
            || !absent(&negative.mtu)
            || [&negative.ifaddrs, &negative.links].into_iter().any(|s| {
                s.value()
                    .is_none_or(|entries| entries.iter().any(|i| i.name == name))
            })
        {
            return Err(invalid(
                "fixture negative control unavailable or name already exists",
            ));
        }
        let ordinary = OrdinaryControl {
            interface: InterfaceId {
                name: "lo".into(),
                index: fixture::index("lo")
                    .map_err(|e| invalid(format!("loopback index unavailable: {e:?}")))?,
            },
            flags: value(&negative.ordinary_flags)?,
            mtu: value(&negative.ordinary_mtu)?,
        };
        let tun = tun_device()?;
        if unsafe { libc::fcntl(tun.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC == 0 {
            return Err(invalid("TUN descriptor is not close-on-exec"));
        }
        let mut allocation = request(&name);
        allocation.ifr_ifru.ifru_flags = i16::from_ne_bytes(TUN_FLAGS.to_ne_bytes());
        if unsafe { libc::ioctl(tun.as_raw_fd(), TUNSETIFF as _, &mut allocation) } != 0 {
            return Err(io::Error::last_os_error());
        }
        ownership.created = true;
        // Record ifindex before later MTU/positive-control setup can fail.
        // Cleanup tests use absence, not these placeholder fixture attributes.
        let allocated_index = fixture::index(&name)
            .map_err(|e| invalid(format!("allocated fixture identity unavailable: {e:?}")))?;
        ownership.spec = Some(NativeFixtureSpec {
            session,
            interface: InterfaceId {
                name: name.clone(),
                index: allocated_index,
            },
            flags: 0,
            mtu: FIXTURE_MTU as u32,
            ordinary: ordinary.clone(),
        });
        #[cfg(feature = "fixture-oracle")]
        if fault == SetupFault::AfterAllocation {
            return Err(invalid("injected post-allocation setup failure"));
        }
        let actual_name: Vec<_> = allocation
            .ifr_name
            .iter()
            .map(|b| b.to_ne_bytes()[0])
            .take_while(|b| *b != 0)
            .collect();
        if actual_name != name.as_bytes() {
            return Err(invalid("kernel changed the exclusive fixture name"));
        }
        let raw = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        let control = unsafe { OwnedFd::from_raw_fd(raw) };
        let mut mtu = request(&name);
        mtu.ifr_ifru.ifru_mtu = FIXTURE_MTU;
        if unsafe { libc::ioctl(control.as_raw_fd(), libc::SIOCSIFMTU as _, &mut mtu) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let index = fixture::index(&name)
            .map_err(|e| invalid(format!("fixture index unavailable: {e:?}")))?;
        let address_setup = addresses::install(
            ownership.spec.as_ref().expect("allocated identity"),
            &negative,
            fault,
        );
        let root_start = fixture::observe(&name);
        let spec = NativeFixtureSpec {
            session,
            interface: InterfaceId { name, index },
            flags: value(&root_start.flags)?,
            mtu: value(&root_start.mtu)?,
            ordinary,
        };
        if !spec.valid()
            || spec.mtu != FIXTURE_MTU as u32
            || root_start.ordinary_flags.value() != Some(&spec.ordinary.flags)
            || root_start.ordinary_mtu.value() != Some(&spec.ordinary.mtu)
            || [&root_start.ifaddrs, &root_start.links]
                .into_iter()
                .any(|s| {
                    s.value().is_none_or(|entries| {
                        !entries.iter().any(|e| e.matches(&spec.interface))
                            || !entries.iter().any(|e| e.matches(&spec.ordinary.interface))
                    })
                })
        {
            return Err(invalid(format!(
                "TUN positive/ordinary control failed; closing owned fixture; spec={spec:?}; observation={root_start:?}"
            )));
        }
        Ok(Self {
            address_setup,
            tun: Some(tun),
            spec,
            negative,
            root_start,
        })
    }

    /// Always close before evaluating. A failed/unknown report cannot keep a
    /// native fixture alive or silently claim that cleanup succeeded.
    pub fn finish(mut self, worker: &NativeMeasurement) -> NativeCompletion {
        let root_end = fixture::observe(&self.spec.interface.name);
        drop(self.tun.take());
        let cleanup = fixture::observe(&self.spec.interface.name);
        let result = evaluate_native_fixture(
            &self.spec,
            &self.negative,
            &self.root_start,
            worker,
            &root_end,
            &cleanup,
        );
        let binding = vpnhide_diagnostic_model::evaluate_bind_fixture(
            &self.spec,
            &self.negative,
            &self.root_start,
            worker,
            &root_end,
            &cleanup,
        );
        NativeCompletion {
            addresses: match [
                &self.negative,
                &self.root_start,
                worker,
                &root_end,
                &cleanup,
            ]
            .map(|m| m.addresses.as_ref())
            {
                [Some(n), Some(s), Some(w), Some(e), Some(c)] => {
                    vpnhide_diagnostic_model::evaluate_address_fixture(&self.spec, n, s, w, e, c)
                }
                _ => vpnhide_diagnostic_model::ScopedAddressResult {
                    controls: vec![vpnhide_diagnostic_model::ControlFailure::ProbeUnavailable],
                    ..Default::default()
                },
            },
            root_end,
            cleanup,
            result,
            binding,
        }
    }

    #[cfg(feature = "fixture-oracle")]
    pub fn duplicate_for_oracle(&self) -> io::Result<File> {
        self.tun
            .as_ref()
            .ok_or_else(|| invalid("fixture already closed"))?
            .try_clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exclusive_name_is_bounded_and_zero_session_never_dispatches() {
        assert!(fixture_name(0).is_err());
        for session in [1, 0x1234_5678_90ab, u64::MAX] {
            let name = fixture_name(session).unwrap();
            assert_eq!(name.len(), 15);
            assert!(vpnhide_checks::is_vpn_interface_name(&name));
            assert!(!name.contains('%'));
        }
        assert!(TunFixture::create(0).is_err());
    }
    #[test]
    fn regular_file_is_not_a_fixture_device() {
        assert!(validate_device(&tempfile::tempfile().unwrap()).is_err());
    }
}
