//! Private bounded Zygisk companion transport. This authenticates observations,
//! never exposes configuration writes or arbitrary privileged device operations.

pub mod image;
pub mod transport;

use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;
pub const MAX_FRAME: usize = 4096;
pub const CORE_SOCKET: &str = "/data/adb/vpnhide/core/zygisk.sock";
pub const WORKER_PROCESS: &str = "dev.okhsunrog.vpnhide:diagnostics";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Challenge {
    pub version: u32,
    pub nonce: [u8; 32],
    pub boot_id: String,
    pub epoch: u64,
}

impl Challenge {
    pub fn valid(&self) -> bool {
        self.version == VERSION
            && self.nonce != [0; 32]
            && !self.boot_id.is_empty()
            && self.boot_id.len() <= 128
            && !self.boot_id.chars().any(char::is_control)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credentials {
    pub pid: u32,
    pub uid: u32,
    pub gid: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Callback {
    pub version: u32,
    pub nonce: [u8; 32],
    pub anchor: u64,
    pub target_uid: u32,
    pub process: String,
    pub generation: Option<u64>,
    pub policy_sha256: Option<[u8; 32]>,
}

impl Callback {
    pub fn valid(&self) -> bool {
        self.version == VERSION
            && self.nonce != [0; 32]
            && self.anchor != 0
            && self.target_uid != u32::MAX
            && !self.process.is_empty()
            && self.process.len() <= 256
            && !self.process.chars().any(char::is_control)
            && self.generation.is_some() == self.policy_sha256.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompanionReport {
    pub child: Credentials,
    pub companion_anchor: u64,
    pub callback: Callback,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Acknowledgement {
    pub accepted: bool,
    pub nonce: [u8; 32],
}
