//! Root-owned cleanup identity. No caller supplies these observations.
use serde::{Deserialize, Serialize};
use std::{io, os::unix::fs::MetadataExt, path::Path};
use vpnhide_diagnostic_model::{InterfaceId, ProbeError, RouteFixtureSpec, RouteMeasurement};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Environment {
    boot_id: String,
    namespace: (u64, u64),
}
impl Environment {
    pub(super) fn valid(&self) -> bool {
        self.boot_id.len() == 36
            && self.boot_id.bytes().enumerate().all(|(i, b)| {
                if [8, 13, 18, 23].contains(&i) {
                    b == b'-'
                } else {
                    b.is_ascii_hexdigit()
                }
            })
            && self
                .boot_id
                .bytes()
                .any(|b| b.is_ascii_hexdigit() && b != b'0')
            && self.namespace.0 != 0
            && self.namespace.1 != 0
    }
    pub(super) fn same_boot(&self, other: &Self) -> bool {
        self.boot_id == other.boot_id
    }
    fn observe_in(root: &Path) -> io::Result<Self> {
        let boot_id = crate::native_boot::read_boot_id(&root.join("sys/kernel/random/boot_id"));
        let meta = std::fs::metadata(root.join("thread-self/ns/net"))?;
        let identity = Self {
            boot_id,
            namespace: (meta.dev(), meta.ino()),
        };
        if !identity.valid() {
            return Err(io::Error::other(
                "boot or network namespace identity unavailable",
            ));
        }
        Ok(identity)
    }
    pub(super) fn observe() -> io::Result<Self> {
        Self::observe_in(Path::new("/proc"))
    }
    pub(super) fn current(&self) -> bool {
        Self::observe().is_ok_and(|v| v == *self)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteCleanupPlan {
    pub spec: RouteFixtureSpec,
    environment: Environment,
    aliases: Vec<InterfaceId>,
    pub created: bool,
    pub unidentified: bool,
}
impl RouteCleanupPlan {
    pub(super) fn new(spec: RouteFixtureSpec) -> io::Result<Self> {
        Ok(Self {
            spec,
            environment: Environment::observe()?,
            aliases: Vec::new(),
            created: false,
            unidentified: false,
        })
    }
    pub(super) fn record(&mut self, interface: InterfaceId, ordinary: bool) {
        self.created = true;
        if ordinary {
            self.spec.ordinary.index = interface.index;
        } else {
            self.spec.vpn.index = interface.index;
        }
        if !self.aliases.contains(&interface) {
            self.aliases.push(interface);
        }
    }
    pub fn valid(&self) -> bool {
        self.spec.valid()
            && self.created
            && !self.unidentified
            && self.aliases.len() <= 3
            && self
                .aliases
                .iter()
                .all(|i| i.index > 0 && i.index <= i32::MAX as u32 && i.name.len() == 15)
    }
    pub(super) fn current(&self) -> bool {
        self.environment.current()
    }
    pub fn observe_cleanup(&self) -> (RouteMeasurement, bool) {
        if !self.current() {
            return (unavailable(), false);
        }
        let observed = vpnhide_checks::route::observe();
        let clean = !self.unidentified
            && (!self.created
                || (self.aliases.iter().all(super::absent) && self.spec.negative(&observed)));
        (observed, clean)
    }
}
pub(super) fn unavailable() -> RouteMeasurement {
    RouteMeasurement::unavailable(ProbeError::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("sys/kernel/random")).unwrap();
        std::fs::create_dir_all(root.path().join("thread-self/ns")).unwrap();
        std::fs::write(
            root.path().join("sys/kernel/random/boot_id"),
            "12345678-1234-1234-1234-123456789abc\n",
        )
        .unwrap();
        std::fs::write(root.path().join("thread-self/ns/net"), "").unwrap();
        root
    }
    #[test]
    fn environment_requires_known_boot_and_namespace_and_detects_replacement() {
        let root = root();
        let initial = Environment::observe_in(root.path()).unwrap();
        assert_eq!(Environment::observe_in(root.path()).unwrap(), initial);
        std::fs::rename(
            root.path().join("thread-self/ns/net"),
            root.path().join("thread-self/ns/old"),
        )
        .unwrap();
        std::fs::write(root.path().join("thread-self/ns/net"), "").unwrap();
        assert_ne!(Environment::observe_in(root.path()).unwrap(), initial);
        std::fs::write(root.path().join("sys/kernel/random/boot_id"), "unknown").unwrap();
        assert!(Environment::observe_in(root.path()).is_err());
        std::fs::write(
            root.path().join("sys/kernel/random/boot_id"),
            "00000000-0000-0000-0000-000000000000",
        )
        .unwrap();
        assert!(Environment::observe_in(root.path()).is_err());
    }
    #[test]
    fn unavailable_namespace_is_not_an_empty_successful_observation() {
        let root = root();
        std::fs::remove_file(root.path().join("thread-self/ns/net")).unwrap();
        assert!(Environment::observe_in(root.path()).is_err());
        let m = unavailable();
        assert!(m.bounded());
        assert_eq!(m.observations.len(), 8);
        assert!(
            m.observations
                .iter()
                .all(|v| v.error == Some(ProbeError::Unsupported))
        );
    }
}
