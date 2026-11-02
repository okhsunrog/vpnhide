//! Source-bound maintenance preparation and execution. Prepared input is read
//! outside the command writer; the existing journal owns intent/phase ordering.
//! Retired sources are archived, never irreversibly unlinked after import.
use std::collections::BTreeMap;
use std::ffi::CString;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use vpnhide_core_rpc::{CanonicalConfig, OperationId};
use vpnhide_migration_policy::{LegacyCandidate, SourceRole};

const MAX_SOURCE_BYTES: u64 = 64 * 1024;
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacySource {
    Kmod,
    Kpm,
    Zygisk,
    Java,
    Ports,
    Hidden,
    Observers,
    DerivedJavaUids,
    HookStatus,
}
impl LegacySource {
    pub const ALL: [Self; 9] = [
        Self::Kmod,
        Self::Kpm,
        Self::Zygisk,
        Self::Java,
        Self::Ports,
        Self::Hidden,
        Self::Observers,
        Self::DerivedJavaUids,
        Self::HookStatus,
    ];
    fn path(self) -> &'static str {
        if let Some(role) = self.role() {
            return role.path();
        }
        match self {
            Self::DerivedJavaUids => "data/system/vpnhide_uids.txt",
            Self::HookStatus => "data/system/vpnhide_hook_active",
            _ => unreachable!("mapped legacy role"),
        }
    }
    fn role(self) -> Option<SourceRole> {
        match self {
            Self::Kmod => Some(SourceRole::Kmod),
            Self::Kpm => Some(SourceRole::Kpm),
            Self::Zygisk => Some(SourceRole::Zygisk),
            Self::Java => Some(SourceRole::Java),
            Self::Ports => Some(SourceRole::Ports),
            Self::Hidden => Some(SourceRole::Hidden),
            Self::Observers => Some(SourceRole::ObserverUids),
            Self::DerivedJavaUids | Self::HookStatus => None,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Kmod => "kmod",
            Self::Kpm => "kpm",
            Self::Zygisk => "zygisk",
            Self::Java => "java",
            Self::Ports => "ports",
            Self::Hidden => "hidden",
            Self::Observers => "observers",
            Self::DerivedJavaUids => "derived_java_uids",
            Self::HookStatus => "hook_status",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceIdentity {
    pub device: u64,
    pub inode: u64,
    pub bytes: u64,
    pub uid: u32,
    pub mode: u32,
    pub modified: (i64, i64),
    pub changed: (i64, i64),
    pub sha256: String,
}
impl SourceIdentity {
    fn from(file: &File, bytes: &[u8]) -> io::Result<Self> {
        let m = file.metadata()?;
        Ok(Self {
            device: m.dev(),
            inode: m.ino(),
            bytes: m.len(),
            uid: m.uid(),
            mode: m.mode(),
            modified: (m.mtime(), m.mtime_nsec()),
            changed: (m.ctime(), m.ctime_nsec()),
            sha256: format!("{:x}", Sha256::digest(bytes)),
        })
    }
    fn same_body(&self, other: &Self) -> bool {
        self.device == other.device
            && self.inode == other.inode
            && self.bytes == other.bytes
            && self.uid == other.uid
            && self.mode == other.mode
            && self.modified == other.modified
            && self.sha256 == other.sha256
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SourceObservation {
    Absent,
    Present { identity: SourceIdentity },
    Unreadable { reason: String },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundSource {
    pub source: LegacySource,
    pub observation: SourceObservation,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaintenanceKind {
    LegacyImport,
    LegacyDiscard,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaintenancePlan {
    pub version: u32,
    pub operation: OperationId,
    pub kind: MaintenanceKind,
    pub root_identity: (u64, u64),
    pub sources: Vec<BoundSource>,
    pub cleanup_allowed: bool,
    pub preserved_reason: Option<String>,
}
#[derive(Debug, Clone)]
pub struct PreparedMaintenance {
    raw: BTreeMap<SourceRole, String>,
    pub plan: MaintenancePlan,
    pub candidate: LegacyCandidate,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaintenanceSourceOutcome {
    Archived,
    Absent,
    Changed,
    Unreadable,
    Preserved,
    RestoreUncertain,
    NotAttempted,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaintenanceReport {
    pub sources: Vec<(LegacySource, MaintenanceSourceOutcome)>,
    pub durability_unknown: bool,
}
impl MaintenanceReport {
    pub fn confirmed(&self) -> bool {
        !self.durability_unknown
            && self.sources.iter().all(|(_, r)| {
                matches!(
                    r,
                    MaintenanceSourceOutcome::Archived
                        | MaintenanceSourceOutcome::Absent
                        | MaintenanceSourceOutcome::Preserved
                )
            })
    }
}
fn invalid(message: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}
fn root_file(path: &Path) -> io::Result<File> {
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}
fn read_source(root: &File, source: LegacySource) -> io::Result<(SourceIdentity, Vec<u8>)> {
    let mut file = crate::payloads::open_beneath(root, source.path())?;
    let before = SourceIdentity::from(&file, &[])?;
    if before.bytes > MAX_SOURCE_BYTES {
        return Err(invalid("legacy source exceeds limit"));
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_SOURCE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    let after = SourceIdentity::from(&file, &bytes)?;
    if bytes.len() as u64 != before.bytes
        || before.bytes != after.bytes
        || before.device != after.device
        || before.inode != after.inode
        || before.modified != after.modified
        || before.changed != after.changed
    {
        return Err(invalid("legacy source changed while reading"));
    }
    Ok((after, bytes))
}

impl PreparedMaintenance {
    /// Only fixed source IDs are accepted. Missing paths are known-empty;
    /// unreadable/symlinked/malformed sources are preserved, never empty policy.
    pub fn capture(
        root: &Path,
        operation: OperationId,
        discard: bool,
        packages: &BTreeMap<String, Vec<u32>>,
        inventory_complete: bool,
    ) -> io::Result<Self> {
        let root = root_file(root)?;
        let metadata = root.metadata()?;
        let mut sources = Vec::new();
        let mut text = BTreeMap::new();
        for source in LegacySource::ALL {
            let observation = match read_source(&root, source) {
                Ok((identity, bytes)) => {
                    if let Some(role) = source.role()
                        && let Ok(raw) = String::from_utf8(bytes)
                    {
                        text.insert(role, raw);
                    }
                    SourceObservation::Present { identity }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    if let Some(role) = source.role() {
                        text.insert(role, String::new());
                    }
                    SourceObservation::Absent
                }
                Err(error) => SourceObservation::Unreadable {
                    reason: format!("{:?}", error.kind()),
                },
            };
            sources.push(BoundSource {
                source,
                observation,
            });
        }
        let candidate = vpnhide_migration_policy::parse(&text, packages, inventory_complete);
        let interpreted = candidate.fully_interpreted()
            && sources
                .iter()
                .all(|s| !matches!(s.observation, SourceObservation::Unreadable { .. }));
        Ok(Self {
            raw: text,
            plan: MaintenancePlan {
                version: 1,
                operation,
                kind: if discard {
                    MaintenanceKind::LegacyDiscard
                } else {
                    MaintenanceKind::LegacyImport
                },
                root_identity: (metadata.dev(), metadata.ino()),
                sources,
                cleanup_allowed: discard || interpreted,
                preserved_reason: (!discard && !interpreted).then(|| {
                    "legacy sources are incomplete, malformed or contain unresolved UIDs".into()
                }),
            },
            candidate,
        })
    }
    pub fn resolve(&mut self, packages: &BTreeMap<String, Vec<u32>>, complete: bool) {
        self.candidate = vpnhide_migration_policy::parse(&self.raw, packages, complete);
        if self.plan.kind == MaintenanceKind::LegacyImport {
            self.plan.cleanup_allowed = self.candidate.fully_interpreted()
                && self
                    .plan
                    .sources
                    .iter()
                    .all(|s| !matches!(s.observation, SourceObservation::Unreadable { .. }));
            self.plan.preserved_reason = (!self.plan.cleanup_allowed)
                .then(|| "legacy sources or UID inventory are incomplete".into());
        }
    }
    pub fn bind_replacement(&mut self, config: &CanonicalConfig) {
        if self.plan.kind == MaintenanceKind::LegacyImport && !self.candidate.covered_by(config) {
            self.plan.cleanup_allowed = false;
            self.plan.preserved_reason =
                Some("replacement does not cover all interpreted legacy roles".into());
        }
    }
}
impl MaintenancePlan {
    pub fn valid(&self) -> bool {
        self.version == 1
            && self.operation.0 != 0
            && self.sources.len() == LegacySource::ALL.len()
            && self.sources.iter().map(|s| s.source).eq(LegacySource::ALL)
            && self.sources.iter().all(|s| match &s.observation {
                SourceObservation::Present { identity } => {
                    identity.bytes <= MAX_SOURCE_BYTES
                        && identity.sha256.len() == 64
                        && identity.sha256.bytes().all(|b| b.is_ascii_hexdigit())
                }
                SourceObservation::Absent => true,
                SourceObservation::Unreadable { reason } => reason.len() <= 128,
            })
            && self
                .preserved_reason
                .as_ref()
                .is_none_or(|r| r.len() <= 512)
    }
    pub fn execute(&self, root_path: &Path) -> io::Result<MaintenanceReport> {
        if !self.valid() {
            return Err(invalid("invalid maintenance plan"));
        }
        let root = root_file(root_path)?;
        let m = root.metadata()?;
        if self.root_identity != (m.dev(), m.ino()) {
            return Err(invalid("maintenance root changed"));
        }
        let mut report = MaintenanceReport {
            sources: Vec::new(),
            durability_unknown: false,
        };
        for source in &self.sources {
            if report.durability_unknown {
                report
                    .sources
                    .push((source.source, MaintenanceSourceOutcome::NotAttempted));
                continue;
            }
            let outcome = if !self.cleanup_allowed {
                MaintenanceSourceOutcome::Preserved
            } else {
                match &source.observation {
                    SourceObservation::Absent => MaintenanceSourceOutcome::Absent,
                    SourceObservation::Unreadable { .. } => MaintenanceSourceOutcome::Unreadable,
                    SourceObservation::Present { identity } => archive(
                        &root,
                        self.operation,
                        source.source,
                        identity,
                        &mut report.durability_unknown,
                    ),
                }
            };
            report.sources.push((source.source, outcome));
        }
        Ok(report)
    }
}
fn directory(root: &File, path: &str, create: bool) -> io::Result<File> {
    let mut dir = root.try_clone()?;
    for part in path.split('/') {
        let part = CString::new(part).map_err(invalid)?;
        if create {
            let result = unsafe { libc::mkdirat(dir.as_raw_fd(), part.as_ptr(), 0o700) };
            if result != 0 && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists {
                return Err(io::Error::last_os_error());
            }
        }
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                part.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        if create {
            dir.sync_all()?;
        }
        dir = unsafe { File::from_raw_fd(fd) };
    }
    Ok(dir)
}
fn rename_no_replace(from: &File, name: &str, to: &File, target: &str) -> io::Result<()> {
    let name = CString::new(name).map_err(invalid)?;
    let target = CString::new(target).map_err(invalid)?;
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            from.as_raw_fd(),
            name.as_ptr(),
            to.as_raw_fd(),
            target.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
fn archive(
    root: &File,
    op: OperationId,
    source: LegacySource,
    expected: &SourceIdentity,
    uncertain: &mut bool,
) -> MaintenanceSourceOutcome {
    let (current, bytes) = match read_source(root, source) {
        Ok(pair) => pair,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return MaintenanceSourceOutcome::Absent,
        Err(_) => return MaintenanceSourceOutcome::Unreadable,
    };
    if current != *expected {
        return MaintenanceSourceOutcome::Changed;
    }
    let (parent, name) = source.path().rsplit_once('/').expect("fixed source path");
    let prepared = (|| -> io::Result<_> {
        let parent = directory(root, parent, false)?;
        let backup = directory(
            root,
            &format!("data/adb/vpnhide/core/maintenance/{}", op.0),
            true,
        )?;
        // Immutable snapshot bytes survive even a legacy writer retaining the
        // original inode. Its retired inode is preserved too, never unlinked.
        let snapshot = CString::new(format!("{}.snapshot", source.name())).map_err(invalid)?;
        let fd = unsafe {
            libc::openat(
                backup.as_raw_fd(),
                snapshot.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut file = unsafe { File::from_raw_fd(fd) };
        file.write_all(&bytes)?;
        file.sync_all()?;
        backup.sync_all()?;
        Ok((parent, backup))
    })();
    let Ok((parent, backup)) = prepared else {
        return MaintenanceSourceOutcome::Unreadable;
    };
    let retired = format!("{}.retired", source.name());
    if rename_no_replace(&parent, name, &backup, &retired).is_err() {
        return MaintenanceSourceOutcome::Changed;
    }
    let verify = (|| -> io::Result<bool> {
        let mut captured = crate::payloads::open_beneath(&backup, &retired)?;
        let mut bytes = Vec::new();
        (&mut captured)
            .take(MAX_SOURCE_BYTES + 1)
            .read_to_end(&mut bytes)?;
        let actual = SourceIdentity::from(&captured, &bytes)?;
        captured.sync_all()?;
        parent.sync_all()?;
        backup.sync_all()?;
        Ok(expected.same_body(&actual))
    })();
    match verify {
        Ok(true) => MaintenanceSourceOutcome::Archived,
        Ok(false) => {
            // A replacement raced the move. Restore without overwriting any new
            // source; keep every byte in quarantine if restoration is uncertain.
            if rename_no_replace(&backup, &retired, &parent, name).is_ok()
                && parent.sync_all().is_ok()
                && backup.sync_all().is_ok()
            {
                MaintenanceSourceOutcome::Changed
            } else {
                *uncertain = true;
                MaintenanceSourceOutcome::RestoreUncertain
            }
        }
        Err(_) => {
            *uncertain = true;
            MaintenanceSourceOutcome::RestoreUncertain
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;
    fn source(root: &Path, id: LegacySource, bytes: &str) -> std::path::PathBuf {
        let path = root.join(id.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }
    fn capture(root: &Path, discard: bool) -> PreparedMaintenance {
        PreparedMaintenance::capture(root, OperationId(7), discard, &Default::default(), true)
            .unwrap()
    }
    #[test]
    fn import_archives_only_bound_sources_and_preserves_later_writes_to_retired_inode() {
        let root = tempfile::tempdir().unwrap();
        let path = source(root.path(), LegacySource::Kmod, "com.bank.app\n");
        let mut legacy_writer = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        let mut prepared = capture(root.path(), false);
        let replacement = vpnhide_migration_policy::apply(
            &CanonicalConfig::default(),
            &prepared.candidate,
            vpnhide_migration_policy::ImportMode::Merge,
            vpnhide_migration_policy::APP_PACKAGE,
        );
        prepared.bind_replacement(&replacement);
        let report = prepared.plan.execute(root.path()).unwrap();
        assert!(report.confirmed());
        assert!(!path.exists());
        let backup = root.path().join("data/adb/vpnhide/core/maintenance/7");
        assert_eq!(
            fs::read_to_string(backup.join("kmod.snapshot")).unwrap(),
            "com.bank.app\n"
        );
        legacy_writer.write_all(b"com.later.app\n").unwrap();
        assert_eq!(
            fs::read_to_string(backup.join("kmod.retired")).unwrap(),
            "com.bank.app\ncom.later.app\n"
        );
        assert_eq!(
            fs::read_to_string(backup.join("kmod.snapshot")).unwrap(),
            "com.bank.app\n"
        );
    }
    #[test]
    fn replaced_inode_or_changed_bytes_remain_at_their_original_path() {
        for replace_inode in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let path = source(root.path(), LegacySource::Kmod, "com.bank.app\n");
            let plan = capture(root.path(), true).plan;
            if replace_inode {
                let tmp = path.with_extension("new");
                fs::write(&tmp, "com.bank.app\n").unwrap();
                fs::rename(tmp, &path).unwrap();
            } else {
                fs::write(&path, "com.newp.app\n").unwrap();
            }
            let bytes = fs::read(&path).unwrap();
            let report = plan.execute(root.path()).unwrap();
            assert!(
                report
                    .sources
                    .contains(&(LegacySource::Kmod, MaintenanceSourceOutcome::Changed))
            );
            assert!(!report.confirmed());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
    }
    #[test]
    fn incomplete_inputs_or_missing_replacement_roles_cannot_authorize_cleanup() {
        for contents in ["com.bank.app\nbroken\n", "com.bank.app\n"] {
            let root = tempfile::tempdir().unwrap();
            let path = source(root.path(), LegacySource::Kmod, contents);
            let mut prepared = capture(root.path(), false);
            prepared.bind_replacement(&CanonicalConfig::default());
            assert!(!prepared.plan.cleanup_allowed);
            assert!(prepared.plan.execute(root.path()).unwrap().confirmed());
            assert_eq!(fs::read_to_string(path).unwrap(), contents);
        }
    }
    #[test]
    fn symlinked_sources_and_archive_directories_never_escape_the_bound_root() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let path = source(root.path(), LegacySource::Kmod, "com.bank.app\n");
        let outside_file = outside.path().join("outside");
        fs::write(&outside_file, "original").unwrap();
        fs::remove_file(&path).unwrap();
        symlink(&outside_file, &path).unwrap();
        let prepared = capture(root.path(), true);
        assert!(matches!(
            prepared.plan.sources[0].observation,
            SourceObservation::Unreadable { .. }
        ));
        assert!(!prepared.plan.execute(root.path()).unwrap().confirmed());
        assert_eq!(fs::read_to_string(&outside_file).unwrap(), "original");
        fs::remove_file(&path).unwrap();
        fs::write(&path, "com.bank.app\n").unwrap();
        let prepared = capture(root.path(), true);
        let core = root.path().join("data/adb/vpnhide/core");
        fs::create_dir_all(&core).unwrap();
        symlink(outside.path(), core.join("maintenance")).unwrap();
        assert!(!prepared.plan.execute(root.path()).unwrap().confirmed());
        assert!(path.exists());
        assert!(!outside.path().join("7").exists());
    }
    #[test]
    fn preexisting_backup_is_not_overwritten_or_treated_as_a_completed_move() {
        let root = tempfile::tempdir().unwrap();
        let path = source(root.path(), LegacySource::Kmod, "com.bank.app\n");
        let prepared = capture(root.path(), true);
        let backup = root.path().join("data/adb/vpnhide/core/maintenance/7");
        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join("kmod.snapshot"), "prior evidence").unwrap();
        assert!(!prepared.plan.execute(root.path()).unwrap().confirmed());
        assert!(path.exists());
        assert_eq!(
            fs::read_to_string(backup.join("kmod.snapshot")).unwrap(),
            "prior evidence"
        );
    }
}
