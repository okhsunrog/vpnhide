//! Fixed, descriptor-relative Core→system_server fixture channel. Canonical
//! configuration and the APK never write this authorization. Source removal is
//! independent of removing already-loaded native hooks.
use serde::de::DeserializeOwned;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use vpnhide_diagnostic_model::framework::{FrameworkAcknowledgement, FrameworkEnvelope};

pub(crate) const ENVELOPE_FILE: &str = "vpnhide-diagnostic-framework.json";
pub(crate) const ACK_FILE: &str = "vpnhide-diagnostic-framework-ack.json";
const MAX_BYTES: u64 = 4096;
pub(super) struct FrameworkStore {
    directory: File,
    path: PathBuf,
    owner: u32,
    ack_owner: u32,
}
fn invalid(message: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}
fn replaced() -> io::Error {
    io::Error::new(
        io::ErrorKind::WouldBlock,
        "framework channel file was replaced",
    )
}
fn content_identity(m: &fs::Metadata) -> (u64, u64, u64, u32, u32, i64, i64) {
    (
        m.dev(),
        m.ino(),
        m.len(),
        m.uid(),
        m.mode(),
        m.mtime(),
        m.mtime_nsec(),
    )
}

fn require_stable_read(
    before: &fs::Metadata,
    after: &fs::Metadata,
    named: &fs::Metadata,
) -> io::Result<()> {
    if (before.dev(), before.ino()) != (named.dev(), named.ino())
        || after.nlink() == 0
        || named.nlink() == 0
    {
        return Err(replaced());
    }
    if [after, named]
        .iter()
        .any(|m| m.nlink() != 1 || content_identity(before) != content_identity(m))
    {
        return Err(invalid("framework channel changed during read"));
    }
    // Metadata for the new name can become visible before rename finishes
    // updating ctime. Discard this sample and let the ACK poll reopen the name.
    let ctime = (before.ctime(), before.ctime_nsec());
    if [after, named]
        .iter()
        .any(|m| (m.ctime(), m.ctime_nsec()) != ctime)
    {
        return Err(replaced());
    }
    Ok(())
}
impl FrameworkStore {
    pub(super) fn production() -> io::Result<Self> {
        Self::open(Path::new("/data/system"), 0, 1000)
    }
    pub(super) fn open(path: &Path, owner: u32, ack_owner: u32) -> io::Result<Self> {
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC)
            .open(path)?;
        Ok(Self {
            directory,
            path: path.into(),
            owner,
            ack_owner,
        })
    }
    fn pinned(&self) -> PathBuf {
        PathBuf::from(format!("/proc/self/fd/{}", self.directory.as_raw_fd()))
    }
    fn revalidate(&self) -> io::Result<()> {
        let active = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC)
            .open(&self.path)?;
        let a = active.metadata()?;
        let b = self.directory.metadata()?;
        if (a.dev(), a.ino()) != (b.dev(), b.ino()) {
            return Err(invalid("framework channel directory replaced"));
        }
        Ok(())
    }
    fn read<T: DeserializeOwned>(&self, name: &str, owner: u32) -> io::Result<Option<T>> {
        self.revalidate()?;
        let file = match crate::payloads::open_beneath(&self.directory, name) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        self.read_opened(file, name, owner)
    }
    fn read_opened<T: DeserializeOwned>(
        &self,
        mut file: File,
        name: &str,
        owner: u32,
    ) -> io::Result<Option<T>> {
        let before = file.metadata()?;
        if before.uid() != owner
            || before.nlink() > 1
            || before.mode() & 0o022 != 0
            || before.len() == 0
            || before.len() > MAX_BYTES
        {
            return Err(invalid("framework channel ownership/type/size is invalid"));
        }
        if before.nlink() == 0 {
            return Err(replaced());
        }
        let mut bytes = Vec::new();
        (&mut file).take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
        let after = file.metadata()?;
        let named = fs::symlink_metadata(self.pinned().join(name)).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                replaced()
            } else {
                error
            }
        })?;
        require_stable_read(&before, &after, &named)?;
        if bytes.len() as u64 != before.len() {
            return Err(invalid("framework channel changed during read"));
        }
        self.revalidate()?;
        serde_json::from_slice(&bytes).map(Some).map_err(invalid)
    }
    pub(super) fn current(&self) -> io::Result<Option<FrameworkEnvelope>> {
        let value = self.read::<FrameworkEnvelope>(ENVELOPE_FILE, self.owner)?;
        if value.as_ref().is_some_and(|v| !v.valid()) {
            return Err(invalid("invalid framework authority"));
        }
        Ok(value)
    }
    pub(super) fn poll_acknowledgement(
        &self,
        expected: &FrameworkEnvelope,
    ) -> io::Result<Option<FrameworkAcknowledgement>> {
        match self.acknowledgement(expected) {
            // system_server publishes each phase by renaming a complete ACK.
            // Retry the new name under the phase deadline, never use the old inode.
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(None),
            result => result,
        }
    }
    pub(super) fn acknowledgement(
        &self,
        expected: &FrameworkEnvelope,
    ) -> io::Result<Option<FrameworkAcknowledgement>> {
        let value = self.read::<FrameworkAcknowledgement>(ACK_FILE, self.ack_owner)?;
        if value.as_ref().is_some_and(|v| !v.matches(expected)) {
            return Ok(None);
        }
        Ok(value)
    }
    /// Compare the exact old lease, including phase. Never silently replace an
    /// unrelated active run or extend its expiry on a lost-reply retry.
    pub(super) fn publish(
        &self,
        expected: Option<&FrameworkEnvelope>,
        next: &FrameworkEnvelope,
    ) -> io::Result<()> {
        if !next.valid() || self.current()?.as_ref() != expected {
            return Err(invalid("framework authority changed"));
        }
        if expected == Some(next) {
            return Ok(());
        }
        let bytes = serde_json::to_vec(next).map_err(invalid)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(invalid("framework envelope exceeds bound"));
        }
        let mut staged = tempfile::NamedTempFile::new_in(self.pinned())?;
        staged.as_file_mut().write_all(&bytes)?;
        staged
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o640))?;
        if staged.as_file().metadata()?.uid() != self.owner {
            return Err(invalid("framework writer is not its declared owner"));
        }
        if self.owner == 0
            && self.ack_owner == 1000
            && unsafe { libc::fchown(staged.as_file().as_raw_fd(), u32::MAX, 1000) } != 0
        {
            return Err(io::Error::last_os_error());
        }
        staged.as_file().sync_all()?;
        self.revalidate()?;
        if self.current()?.as_ref() != expected {
            return Err(invalid("framework authority changed before publication"));
        }
        staged
            .persist(self.pinned().join(ENVELOPE_FILE))
            .map_err(|e| e.error)?;
        self.directory.sync_all()?;
        if self.current()?.as_ref() != Some(next) {
            return Err(invalid("framework publication readback failed"));
        }
        Ok(())
    }
    pub(super) fn revoke(&self, expected: &FrameworkEnvelope) -> io::Result<()> {
        if self.current()?.as_ref() != Some(expected) {
            return Err(invalid("framework cleanup source changed"));
        }
        fs::remove_file(self.pinned().join(ENVELOPE_FILE))?;
        self.directory.sync_all()?;
        if self.current()?.is_some() {
            return Err(invalid("framework authorization still present"));
        }
        // ACK is not authorization; remove only this exact session's response.
        if self.acknowledgement(expected)?.is_some() {
            fs::remove_file(self.pinned().join(ACK_FILE))?;
            self.directory.sync_all()?;
        }
        self.revalidate()
    }
    pub(super) fn reconcile_orphan(&self, epoch: u64, boot: &str, now: u64) -> io::Result<()> {
        if let Some(old) = self.current()? {
            if old.core_epoch == epoch && old.boot_id == boot && now < old.expires_boot_ms {
                return Err(invalid("a live framework lease still owns the channel"));
            }
            self.revoke(&old)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use vpnhide_diagnostic_model::framework::FrameworkPhase;
    fn envelope() -> FrameworkEnvelope {
        serde_json::from_str(include_str!(
            "../../../../fixtures/diagnostics/framework-envelope.json"
        ))
        .unwrap()
    }
    fn store(root: &Path) -> FrameworkStore {
        let owner = unsafe { libc::geteuid() };
        FrameworkStore::open(root, owner, owner).unwrap()
    }
    #[test]
    fn atomic_ack_replacement_invalidates_the_open_inode_but_not_the_channel() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let path = root.path().join(ACK_FILE);
        fs::write(&path, b"1").unwrap();
        let opened = crate::payloads::open_beneath(&store.directory, ACK_FILE).unwrap();

        let mut staged = tempfile::NamedTempFile::new_in(root.path()).unwrap();
        staged.write_all(b"2").unwrap();
        staged.persist(&path).unwrap();

        assert_eq!(
            store
                .read_opened::<u8>(opened, ACK_FILE, store.ack_owner)
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock,
        );
        assert_eq!(
            store.read::<u8>(ACK_FILE, store.ack_owner).unwrap(),
            Some(2)
        );
        assert_eq!(
            store.poll_acknowledgement(&envelope()).unwrap_err().kind(),
            io::ErrorKind::InvalidData,
        );
    }

    #[test]
    fn rename_metadata_race_is_retriable_but_in_place_content_change_is_invalid() {
        let root = tempfile::tempdir().unwrap();
        let staged = root.path().join("staged");
        let path = root.path().join(ACK_FILE);
        fs::write(&staged, b"1").unwrap();
        let before = fs::metadata(&staged).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::rename(&staged, &path).unwrap();
        let after_rename = fs::metadata(&path).unwrap();
        assert_eq!(
            require_stable_read(&before, &after_rename, &after_rename)
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock,
        );
        fs::write(&path, b"22").unwrap();
        let after_write = fs::metadata(&path).unwrap();
        assert_eq!(
            require_stable_read(&after_rename, &after_write, &after_write)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData,
        );
    }
    #[test]
    fn publication_is_exact_idempotent_and_recovery_revokes_only_a_known_orphan() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let prepared = envelope();
        store.publish(None, &prepared).unwrap();
        store.publish(Some(&prepared), &prepared).unwrap();
        assert!(
            store
                .reconcile_orphan(prepared.core_epoch, &prepared.boot_id, 1)
                .is_err()
        );
        let raw = prepared.for_phase(FrameworkPhase::Raw);
        store.publish(Some(&prepared), &raw).unwrap();
        assert!(store.publish(Some(&prepared), &prepared).is_err());
        assert_eq!(store.current().unwrap(), Some(raw));
        store
            .reconcile_orphan(prepared.core_epoch + 1, &prepared.boot_id, 1)
            .unwrap();
        assert!(store.current().unwrap().is_none());
    }
    #[test]
    fn symlinks_malformed_or_changed_sources_are_preserved_without_overwrite_or_delete() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let env = envelope();
        let target = root.path().join("keep");
        fs::write(&target, b"unrelated").unwrap();
        let path = root.path().join(ENVELOPE_FILE);
        symlink(&target, &path).unwrap();
        assert!(store.publish(None, &env).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"unrelated");
        fs::remove_file(&path).unwrap();
        fs::write(&path, b"invalid").unwrap();
        assert!(store.reconcile_orphan(8, &env.boot_id, 30_000).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"invalid");
        fs::remove_file(&path).unwrap();
        store.publish(None, &env).unwrap();
        let mut other = env.clone();
        other.nonce = "b".repeat(64);
        assert!(store.revoke(&other).is_err());
        assert_eq!(store.current().unwrap(), Some(env));
    }
    #[test]
    fn replaced_directory_and_group_writable_authority_do_not_become_a_grant() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("system");
        fs::create_dir(&path).unwrap();
        let store = store(&path);
        let env = envelope();
        store.publish(None, &env).unwrap();
        fs::set_permissions(path.join(ENVELOPE_FILE), fs::Permissions::from_mode(0o664)).unwrap();
        assert!(store.current().is_err());
        fs::set_permissions(path.join(ENVELOPE_FILE), fs::Permissions::from_mode(0o644)).unwrap();
        fs::rename(&path, root.path().join("old")).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(store.current().is_err());
        assert!(store.revoke(&env).is_err());
        assert!(root.path().join("old").join(ENVELOPE_FILE).is_file());
    }
}
