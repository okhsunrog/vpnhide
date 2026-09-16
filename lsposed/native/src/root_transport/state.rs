use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use super::output::NativeCapacity;
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;
const MAX_STATE: u64 = 4096;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Idle,
    Running,
    Finished,
    NotStarted,
}

/// Lifetime metadata and typed numeric warnings. Never persist scripts, configs or secret material.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub version: u32,
    pub revision: u64,
    pub boot: String,
    pub session: Option<String>,
    pub sequence: u64,
    pub status: Status,
    pub exit_code: Option<i32>,
    pub descendant_failed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_capacity: Option<NativeCapacity>,
}

impl State {
    pub fn idle(boot: &str) -> Self {
        Self {
            version: VERSION,
            revision: 0,
            boot: boot.into(),
            session: None,
            sequence: 0,
            status: Status::Idle,
            exit_code: None,
            descendant_failed: false,
            native_capacity: None,
        }
    }

    pub fn quiescent(&self, boot: &str) -> bool {
        self.boot != boot || self.status != Status::Running
    }

    pub fn matches(&self, boot: &str, session: &str) -> bool {
        self.boot == boot && self.session.as_deref() == Some(session)
    }

    fn valid(&self) -> bool {
        self.version == VERSION
            && self.revision < i64::MAX as u64
            && self.sequence < i64::MAX as u64
            && valid_id(&self.boot)
            && self.session.as_deref().is_none_or(valid_id)
            && (self.status == Status::Idle) == (self.sequence == 0)
            && (self.session.is_some() || self.status == Status::Idle)
            && (self.status == Status::Finished) == self.exit_code.is_some()
            && self.exit_code.is_none_or(|code| (0..=255).contains(&code))
            && (!self.descendant_failed || self.status == Status::Finished)
            && self
                .native_capacity
                .is_none_or(|warning| self.status == Status::Finished && warning.valid())
    }
}

pub fn valid_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit() && !b.is_ascii_uppercase()
            }
        })
}

pub struct Store {
    directory: PathBuf,
    // This inode is permanent. Never unlink/replace it, including during recovery.
    _lock: File,
    pub state: State,
}

impl Store {
    pub fn lock(directory: &Path, boot: &str) -> io::Result<Self> {
        let created = match fs::DirBuilder::new().mode(0o700).create(directory) {
            Ok(()) => true,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => false,
            Err(e) => return Err(e),
        };
        let metadata = fs::symlink_metadata(directory)?;
        if !metadata.is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
        {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(created)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(directory.join("lock"))?;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let state = if created {
            State::idle(boot)
        } else {
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(directory.join("state.json"))?;
            let mut bytes = Vec::new();
            file.take(MAX_STATE + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > MAX_STATE {
                return Err(io::ErrorKind::InvalidData.into());
            }
            let state: State = serde_json::from_slice(&bytes)?;
            if !state.valid() {
                return Err(io::ErrorKind::InvalidData.into());
            }
            state
        };
        let store = Self {
            directory: directory.into(),
            _lock: lock,
            state,
        };
        if created {
            store.persist()?;
            File::open(directory.parent().ok_or(io::ErrorKind::InvalidInput)?)?.sync_all()?;
        }
        Ok(store)
    }

    pub fn replace(&mut self, mut state: State) -> io::Result<()> {
        state.revision = self
            .state
            .revision
            .checked_add(1)
            .ok_or(io::ErrorKind::InvalidData)?;
        if !state.valid() {
            return Err(io::ErrorKind::InvalidData.into());
        }
        self.state = state;
        self.persist()
    }

    fn persist(&self) -> io::Result<()> {
        let tmp = self.directory.join("state.next");
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&tmp)?;
        file.write_all(&serde_json::to_vec(&self.state)?)?;
        file.sync_all()?;
        fs::rename(tmp, self.directory.join("state.json"))?;
        File::open(&self.directory)?.sync_all()
    }
}
