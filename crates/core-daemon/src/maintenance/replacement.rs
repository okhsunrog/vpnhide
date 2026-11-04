//! Frozen authenticated inputs; the existing command writer owns effects.
use super::{
    image::ImageFence,
    legacy_modules::LegacyInventory,
    manager::{Manager, ManagerIdentity},
    package::{FloorStore, PackageStaging, PreparedPackage},
};
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::{
    fs::File,
    io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};
pub struct PreparationPermit(Arc<AtomicBool>);
impl PreparationPermit {
    pub fn acquire(flag: Arc<AtomicBool>) -> Option<Self> {
        flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self(flag))
    }
}
impl Drop for PreparationPermit {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
use vpnhide_core_rpc::{
    CoreMaintenanceFailure, CoreMaintenanceStage, CoreMaintenanceStatus, CoreReplacementRequest,
    OperationId,
};

#[derive(Clone)]
pub struct ReplacementPaths {
    pub root: PathBuf,
    pub persistent: PathBuf,
    pub app: PathBuf,
}
impl ReplacementPaths {
    pub fn active(&self) -> PathBuf {
        self.root.join("data/adb/modules/vpnhide_core")
    }
    pub fn staged(&self) -> PathBuf {
        self.root.join("data/adb/modules_update/vpnhide_core")
    }
    pub fn boot(&self) -> io::Result<String> {
        let boot =
            crate::native_boot::read_boot_id(&self.root.join("proc/sys/kernel/random/boot_id"));
        if !valid_boot(&boot) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid maintenance boot identity",
            ));
        }
        Ok(boot)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplacementWork {
    pub request: CoreReplacementRequest,
    pub status: CoreMaintenanceStatus,
    pub boot_id: String,
    pub before: ImageFence,
    pub manager: ManagerIdentity,
    pub legacy: LegacyInventory,
    pub staged: Option<ImageFence>,
    pub retired: bool,
    /// Synced before the manager can be spawned. Recovery never erases this.
    pub dispatch_intent: bool,
}
impl ReplacementWork {
    pub(crate) fn request_hash(&self) -> String {
        request_hash(&self.request)
    }
    pub(crate) fn valid(&self, issued: u64) -> bool {
        use CoreMaintenanceStage as S;
        let hex = |s: &str, n| {
            s.len() == n
                && s.bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        };
        let stage = self.status.stage;
        self.status.operation.0 > 0
            && self.status.operation.0 <= issued
            && self.request.descriptor.len() <= vpnhide_package_trust::MAX_ENVELOPE_BYTES
            && !self.request.descriptor.is_empty()
            && hex(&self.request.expected_commit, 40)
            && hex(&self.request.expected_package_sha256, 64)
            && self.request.target_rpc > 0
            && self.status.target_commit == self.request.expected_commit
            && valid_boot(&self.boot_id)
            && self.before.files.contains_key("vpnhide-core")
            && self.before.files.len() <= 32
            && self.legacy.modules.len() <= 12
            && self.manager.valid()
            && (matches!(stage, S::Unknown | S::FailedKnown) == self.status.failure.is_some())
            && (!matches!(stage, S::PendingRestart | S::Verified | S::RetiringLegacy)
                || self
                    .staged
                    .as_ref()
                    .is_some_and(ImageFence::verified_layout))
            && (!matches!(stage, S::PendingRestart | S::Verified) || self.retired)
            && (!self.retired || self.staged.is_some())
            && (!matches!(
                stage,
                S::Dispatching
                    | S::VerifyingStaged
                    | S::RetiringLegacy
                    | S::PendingRestart
                    | S::Verified
            ) || self.dispatch_intent)
            && (stage != S::Accepted || !self.dispatch_intent)
    }
    pub(crate) fn blocks_runtime(&self) -> bool {
        self.retained() && self.status.stage != CoreMaintenanceStage::PendingRestart
    }
    pub fn retained(&self) -> bool {
        !matches!(
            self.status.stage,
            CoreMaintenanceStage::Verified | CoreMaintenanceStage::FailedKnown
        )
    }
    pub fn fence_restart(&mut self) {
        match self.status.stage {
            CoreMaintenanceStage::Accepted => {
                self.status.stage = CoreMaintenanceStage::Unknown;
                self.status.failure = Some(CoreMaintenanceFailure::InterruptedBeforeDispatch);
            }
            CoreMaintenanceStage::Dispatching
            | CoreMaintenanceStage::VerifyingStaged
            | CoreMaintenanceStage::RetiringLegacy => {
                self.status.stage = CoreMaintenanceStage::Unknown;
                self.status.failure = Some(CoreMaintenanceFailure::DispatchUncertain);
            }
            _ => {}
        }
    }
}
pub struct PreparedReplacement {
    pub(super) _permit: PreparationPermit,
    owner_uid: u32,
    running: File,
    pub paths: ReplacementPaths,
    pub package: PreparedPackage,
    pub floor: FloorStore,
    pub manager: Manager,
    pub work: ReplacementWork,
}
impl PreparedReplacement {
    /// Directories and UID derive from Core's authenticated socket/profile.
    pub fn prepare(
        paths: ReplacementPaths,
        request: CoreReplacementRequest,
        operation: OperationId,
        caller: Option<u32>,
        permit: PreparationPermit,
    ) -> Result<Self, String> {
        let expected_uid = std::fs::metadata(&paths.app)
            .map_err(|e| e.to_string())?
            .uid();
        if caller.is_some_and(|uid| uid != 0 && uid != expected_uid) {
            return Err("maintenance app identity changed".into());
        }
        let floor = FloorStore::open(&paths.persistent).map_err(|e| e.to_string())?;
        let package = {
            let lease = floor.lock().map_err(|e| e.to_string())?;
            PackageStaging::open(&paths.persistent, &paths.app, expected_uid)
                .map_err(|e| e.to_string())?
                .prepare_replacement(
                    &request.descriptor,
                    &vpnhide_package_trust::Target {
                        source_commit: request.expected_commit.clone(),
                        package_sha256: request.expected_package_sha256.clone(),
                        rpc: request.target_rpc,
                        current_journal: vpnhide_build_contract::JOURNAL_VERSION,
                    },
                    &lease,
                    operation.0,
                    &permit,
                )
                .map_err(|e| e.to_string())?
        };
        package.verify_bundle().map_err(|e| e.to_string())?;
        let running = File::open("/proc/self/exe").map_err(|e| e.to_string())?;
        let before = ImageFence::capture_active(&paths.active()).map_err(|e| e.to_string())?;
        before
            .require_running(&running)
            .map_err(|e| e.to_string())?;
        require_absent(&paths.staged()).map_err(|e| e.to_string())?;
        let manager = Manager::discover(&paths.root).map_err(|e| e.to_string())?;
        let work = ReplacementWork {
            status: CoreMaintenanceStatus {
                operation,
                target_commit: request.expected_commit.clone(),
                stage: CoreMaintenanceStage::Accepted,
                failure: None,
            },
            boot_id: paths.boot().map_err(|e| e.to_string())?,
            before,
            manager: manager.identity.clone(),
            legacy: LegacyInventory::capture(&paths.root).map_err(|e| e.to_string())?,
            request,
            staged: None,
            retired: false,
            dispatch_intent: false,
        };
        Ok(Self {
            _permit: permit,
            owner_uid: 0,
            running,
            paths,
            package,
            floor,
            manager,
            work,
        })
    }
    pub fn require_fresh(&self) -> io::Result<()> {
        if self.paths.boot()? != self.work.boot_id {
            return Err(io::Error::other("maintenance boot changed"));
        }
        self.live_unchanged()?;
        self.legacy_unchanged()?;
        self.manager.require_unchanged()?;
        require_absent(&self.paths.staged())
    }
    pub(crate) fn live_unchanged(&self) -> io::Result<()> {
        self.work
            .before
            .require_unchanged_owned(&self.paths.active(), self.owner_uid)?;
        self.work
            .before
            .require_running_owned(&self.running, self.owner_uid)
    }
    pub(crate) fn legacy_unchanged(&self) -> io::Result<()> {
        self.work
            .legacy
            .require_unchanged_owned(&self.paths.root, self.owner_uid)
    }
    pub(crate) fn verify_staged(&self) -> io::Result<ImageFence> {
        ImageFence::inspect_replacement(
            &self.paths.staged(),
            self.package.verified(),
            self.owner_uid,
        )
    }
    pub(crate) fn retire(&self, staged: &ImageFence) -> io::Result<()> {
        staged.require_unchanged_owned(&self.paths.staged(), self.owner_uid)?;
        self.work
            .legacy
            .retire_owned(&self.paths.root, self.owner_uid)
    }
}
pub(super) fn require_absent(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "unclaimed Core staging slot is occupied",
        )),
    }
}

mod reconcile;
pub(crate) use reconcile::GenerationProof;

pub(crate) fn request_hash(request: &CoreReplacementRequest) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(request).expect("maintenance request encoding"))
    )
}
fn valid_boot(boot: &str) -> bool {
    boot.len() == 36
        && boot.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

#[cfg(test)]
pub(crate) mod tests;
