//! Authorization publication and observed process consumption are separate facts.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZygiskAuthorizationRef {
    pub operation: u64,
    pub boot_id: String,
    pub generation: u64,
    pub policy_sha256: String,
    pub directory_device: u64,
    pub directory_inode: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZygiskPublication {
    NotObserved,
    Matching,
    Absent,
    Different,
    Unknown,
    PriorBoot,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZygiskConsumption {
    Current,
    Stale,
    Unattributed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZygiskProcessConsumption {
    pub pid: u32,
    pub start_ticks: u64,
    pub uid: u32,
    pub process: String,
    pub abi: String,
    pub generation: Option<u64>,
    pub policy_sha256: Option<String>,
    pub consumption: ZygiskConsumption,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZygiskLifecycle {
    pub expected: ZygiskAuthorizationRef,
    pub core_epoch: u64,
    /// Point-in-time observation, not a continuous liveness guarantee.
    pub observed_at_boot_ms: Option<u64>,
    pub publication: ZygiskPublication,
    /// Authenticated, live, identity-revalidated samples, bounded by the registry.
    pub processes: Vec<ZygiskProcessConsumption>,
    /// A fresh worker and a bounded registry never prove every old game updated.
    pub process_inventory_complete: bool,
    /// The callback precedes install_hooks; consumption is not hook effectiveness.
    pub hook_health_measured: bool,
}
impl ZygiskLifecycle {
    pub fn pending(expected: ZygiskAuthorizationRef, core_epoch: u64) -> Self {
        Self {
            expected,
            core_epoch,
            observed_at_boot_ms: None,
            publication: ZygiskPublication::NotObserved,
            processes: vec![],
            process_inventory_complete: false,
            hook_health_measured: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_publication_and_consumption_roundtrips_without_promoting_health() {
        for publication in [
            ZygiskPublication::NotObserved,
            ZygiskPublication::Matching,
            ZygiskPublication::Absent,
            ZygiskPublication::Different,
            ZygiskPublication::Unknown,
            ZygiskPublication::PriorBoot,
        ] {
            for consumption in [
                ZygiskConsumption::Current,
                ZygiskConsumption::Stale,
                ZygiskConsumption::Unattributed,
            ] {
                let mut state = crate::StateSnapshot {
                    zygisk_lifecycle: Some(ZygiskLifecycle::pending(
                        ZygiskAuthorizationRef {
                            operation: 5,
                            boot_id: "boot".into(),
                            generation: 5,
                            policy_sha256: "a".repeat(64),
                            directory_device: 1,
                            directory_inode: 2,
                        },
                        7,
                    )),
                    ..Default::default()
                };
                let lifecycle = state.zygisk_lifecycle.as_mut().unwrap();
                lifecycle.publication = publication;
                lifecycle.processes.push(ZygiskProcessConsumption {
                    pid: 4,
                    start_ticks: 40,
                    uid: 10123,
                    process: "game".into(),
                    abi: "arm64".into(),
                    generation: Some(5),
                    policy_sha256: Some("a".repeat(64)),
                    consumption,
                });
                let decoded: crate::StateSnapshot =
                    serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
                assert_eq!(decoded, state);
                let lifecycle = decoded.zygisk_lifecycle.unwrap();
                assert!(!lifecycle.process_inventory_complete);
                assert!(!lifecycle.hook_health_measured);
            }
        }
    }
}

/// Logical authorization revocation does not unload code from existing processes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZygiskRevocationState {
    Running,
    Confirmed,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZygiskRevocationStatus {
    pub operation: Option<u64>,
    pub current_boot: String,
    pub authorization: ZygiskAuthorizationRef,
    pub state: ZygiskRevocationState,
}
impl ZygiskRevocationStatus {
    pub fn pending(&self) -> bool {
        self.state != ZygiskRevocationState::Confirmed
    }
}
