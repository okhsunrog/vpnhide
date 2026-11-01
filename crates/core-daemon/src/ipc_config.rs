//! Validated endpoint settings, resolved before the daemon opens durable state.
//! AppOwner derives placement/DAC together; peer ownership remains dynamic.
use crate::auth::{self, Allowlist};
use std::{
    collections::BTreeSet,
    ffi::OsString,
    io,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct EndpointConfig {
    path: PathBuf,
    policy: Allowlist,
    mode: u32,
    owner: Option<u32>,
}
impl EndpointConfig {
    pub fn app_owner(dir: PathBuf) -> io::Result<Self> {
        if dir.as_os_str().is_empty() {
            return Err(invalid("empty app data directory"));
        }
        Ok(Self {
            path: crate::app_socket::socket_path(&dir),
            policy: Allowlist::AppOwner(dir),
            mode: 0o600,
            owner: None,
        })
    }
    pub fn from_env(socket: &Path) -> io::Result<Self> {
        Self::resolve(socket, |name| std::env::var_os(name), unsafe {
            libc::geteuid()
        })
    }
    fn resolve(
        socket: &Path,
        get: impl Fn(&str) -> Option<OsString>,
        uid: u32,
    ) -> io::Result<Self> {
        if let Some(dir) = get(crate::app_socket::APP_DATA_DIR_ENV) {
            if dir.is_empty() {
                return Err(invalid("empty app data directory"));
            }
            // Production AppOwner takes precedence; plain-only knobs cannot weaken it.
            return Self::app_owner(dir.into());
        }
        let policy = match text(get(auth::ALLOW_UIDS_ENV))? {
            Some(raw) => Allowlist::Uids(auth::parse_uid_list(&raw)?),
            None => Allowlist::Uids(BTreeSet::from([uid])),
        };
        let owner = text(get(auth::SOCKET_OWNER_UID_ENV))?
            .map(|raw| auth::parse_uid(&raw))
            .transpose()?;
        let mode = match text(get(auth::SOCKET_MODE_ENV))? {
            Some(raw) => {
                let mode =
                    u32::from_str_radix(raw.trim().strip_prefix("0o").unwrap_or(raw.trim()), 8)
                        .map_err(|_| invalid("invalid socket mode"))?;
                if mode > 0o777 {
                    return Err(invalid("socket mode must contain only permission bits"));
                }
                mode
            }
            None => 0o600,
        };
        if socket.as_os_str().is_empty() {
            return Err(invalid("empty socket path"));
        }
        Ok(Self {
            path: socket.into(),
            policy,
            mode,
            owner,
        })
    }
    /// Explicit library callers do not inherit shell IPC settings. Open is useful
    /// only to exercise authentication boundaries in tests, never a CLI fallback.
    pub fn explicit(socket: &Path, policy: Allowlist) -> io::Result<Self> {
        if let Allowlist::AppOwner(dir) = &policy {
            let config = Self::app_owner(dir.clone())?;
            if config.path != socket {
                return Err(invalid("AppOwner socket must be inside its app directory"));
            }
            return Ok(config);
        }
        if socket.as_os_str().is_empty() {
            return Err(invalid("empty socket path"));
        }
        Ok(Self {
            path: socket.into(),
            policy,
            mode: 0o600,
            owner: None,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn app_dir(&self) -> Option<&Path> {
        match &self.policy {
            Allowlist::AppOwner(dir) => Some(dir),
            _ => None,
        }
    }
    pub(crate) fn policy(&self) -> Allowlist {
        self.policy.clone()
    }
    pub(crate) fn prepare(&self, socket: &Path) -> io::Result<()> {
        match self.app_dir() {
            Some(dir) => auth::apply_app_socket_permissions(socket, dir),
            None => auth::apply_private_socket_permissions(socket, self.mode, self.owner),
        }
    }
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
fn text(value: Option<OsString>) -> io::Result<Option<String>> {
    value
        .map(|s| {
            s.into_string()
                .map_err(|_| invalid("IPC setting is not UTF-8"))
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStringExt;

    fn resolve(values: &[(&str, OsString)]) -> io::Result<EndpointConfig> {
        EndpointConfig::resolve(
            Path::new("/tmp/private.sock"),
            |key| {
                values
                    .iter()
                    .find(|(name, _)| *name == key)
                    .map(|(_, value)| value.clone())
            },
            12345,
        )
    }
    #[test]
    fn absent_policy_is_current_uid_not_open_or_implicit_root() {
        let config = resolve(&[]).unwrap();
        assert!(config.policy.permits(12345));
        assert!(!config.policy.permits(0));
        assert!(!config.policy.permits(12346));
        assert_eq!(config.mode, 0o600);
    }
    #[test]
    fn malformed_uid_settings_never_fall_back_or_partially_parse() {
        for value in [
            "",
            " ",
            ",,",
            "garbage",
            "12345,garbage",
            "-1",
            "4294967296",
            "4294967295",
            "+123",
            "open",
        ] {
            assert_eq!(
                resolve(&[(auth::ALLOW_UIDS_ENV, value.into())])
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidInput,
                "{value:?}"
            );
        }
        assert!(resolve(&[(auth::ALLOW_UIDS_ENV, OsString::from_vec(vec![0xff]))]).is_err());
        let config = resolve(&[(auth::ALLOW_UIDS_ENV, "42, 43\n42".into())]).unwrap();
        assert!(config.policy.permits(42));
        assert!(!config.policy.permits(12345));
        assert!(!config.policy.permits(0));
    }
    #[test]
    fn app_owner_precedence_cannot_be_weakened_by_private_settings() {
        let dir = tempfile::tempdir().unwrap();
        let config = resolve(&[
            (crate::app_socket::APP_DATA_DIR_ENV, dir.path().into()),
            (auth::ALLOW_UIDS_ENV, OsString::from_vec(vec![0xff])),
            (auth::SOCKET_MODE_ENV, "0666".into()),
            (auth::SOCKET_OWNER_UID_ENV, "99999".into()),
        ])
        .unwrap();
        assert_eq!(config.path(), crate::app_socket::socket_path(dir.path()));
        assert_eq!(config.policy, Allowlist::AppOwner(dir.path().into()));
        assert_eq!(config.mode, 0o600);
        assert_eq!(config.owner, None);
    }
    #[test]
    fn owner_mode_and_app_directory_fail_validation_when_invalid() {
        for value in ["", "no", "-1", "+123", "4294967295", "4294967296"] {
            assert!(resolve(&[(auth::SOCKET_OWNER_UID_ENV, value.into())]).is_err());
        }
        for value in ["", "888", "-1", "10000", "0o0o600"] {
            assert!(resolve(&[(auth::SOCKET_MODE_ENV, value.into())]).is_err());
        }
        assert!(resolve(&[(crate::app_socket::APP_DATA_DIR_ENV, "".into())]).is_err());
        assert!(
            EndpointConfig::explicit(
                Path::new("/tmp/wrong.sock"),
                Allowlist::AppOwner("/tmp/app".into())
            )
            .is_err()
        );
    }
}
