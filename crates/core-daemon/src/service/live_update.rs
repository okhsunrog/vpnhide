//! Same-boot activation of a root-manager-authorized passive Core install.
//! The manager still owns both module directories and next-boot promotion.
use super::{APP_DATA_DIR, PERSIST_DIR};
use crate::maintenance::{image::ImageFence, replacement::ReplacementPaths};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
    time::Duration,
};
use vpnhide_build_contract::{JOURNAL_VERSION, RPC_VERSION};
use vpnhide_core_rpc::NativeBackend;

const RECEIPT: &str = "manager-live-install.json";

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct UpdateMarker {
    device: u64,
    inode: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Install {
    boot: String,
    rpc: u32,
    journal: u32,
    previous_marker: Option<UpdateMarker>,
    image: ImageFence,
}

pub(super) fn paths() -> ReplacementPaths {
    ReplacementPaths {
        root: "/".into(),
        persistent: PERSIST_DIR.into(),
        app: APP_DATA_DIR.into(),
    }
}

fn absent(path: &Path) -> bool {
    fs::symlink_metadata(path).is_err_and(|e| e.kind() == io::ErrorKind::NotFound)
}

fn owned_file(path: &Path, limit: u64) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file()
        || meta.uid() != 0
        || meta.nlink() != 1
        || meta.mode() & 0o022 != 0
        || meta.len() > limit
    {
        return Err(io::Error::other("unsafe manager live-install input"));
    }
    Ok(file)
}

fn marker(path: &Path) -> io::Result<Option<UpdateMarker>> {
    let file = match owned_file(path, 0) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let meta = file.metadata()?;
    Ok(Some(UpdateMarker {
        device: meta.dev(),
        inode: meta.ino(),
        modified: (meta.mtime(), meta.mtime_nsec()),
        changed: (meta.ctime(), meta.ctime_nsec()),
    }))
}

fn manager_finished(before: Option<UpdateMarker>, after: Option<UpdateMarker>) -> bool {
    after.is_some() && after != before
}

fn read_owned(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    owned_file(path, limit)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(io::Error::other("manager live-install input changed size"));
    }
    Ok(bytes)
}

fn journal_allows(paths: &ReplacementPaths, boot: &str) -> io::Result<bool> {
    let journal = crate::journal::read(&paths.persistent.join("operations.json"))?;
    let builtin = journal
        .native_startup
        .as_ref()
        .and_then(|start| start.completed.as_ref())
        .is_some_and(|result| {
            result.status.boot_id == boot
                && result
                    .selection
                    .as_ref()
                    .is_some_and(|selection| selection.backend == NativeBackend::Builtin)
        });
    Ok(builtin
        && journal.version == JOURNAL_VERSION
        && !journal.has_unknown()
        && journal.pending_persist.is_none()
        && journal
            .replacement
            .as_ref()
            .is_none_or(|work| !work.retained())
        && journal.zygisk_authority.is_none()
        && journal.zygisk_bootstrap.is_none()
        && absent(&paths.active().join("zygisk"))
        && absent(&paths.root.join("data/adb/modules/vpnhide_zygisk_runtime"))
        && absent(
            &paths
                .root
                .join("data/adb/modules_update/vpnhide_zygisk_runtime"),
        ))
}

/// Called at the end of the existing root-only installer handoff. The callback
/// precedes manager completion, so an Install alone never authorizes a launch.
pub(super) fn record(incoming: &Path) -> io::Result<()> {
    let paths = paths();
    if incoming != paths.staged() || std::env::var("KSU").as_deref() != Ok("true") {
        return Ok(());
    }
    let boot = paths.boot()?;
    if !journal_allows(&paths, &boot)? {
        return Ok(());
    }
    let previous_marker = marker(&paths.active().join("update"))?;
    let image = ImageFence::capture_manager_stage(incoming)?;
    image.require_running(&File::open("/proc/self/exe")?)?;
    let mut file = tempfile::NamedTempFile::new_in(&paths.persistent)?;
    serde_json::to_writer(
        &mut file,
        &Install {
            boot,
            rpc: RPC_VERSION,
            journal: JOURNAL_VERSION,
            previous_marker,
            image,
        },
    )?;
    file.as_file().sync_all()?;
    file.persist(paths.persistent.join(RECEIPT))?;
    File::open(&paths.persistent)?.sync_all()
}

pub(super) struct Candidate {
    pub(super) file: File,
    pub(super) image: ImageFence,
}

impl Candidate {
    pub(super) fn open(paths: &ReplacementPaths, receipt: &[u8]) -> io::Result<Option<Self>> {
        let install: Install = serde_json::from_slice(receipt)?;
        let active = paths.active();
        let staged = paths.staged();
        if install.boot != paths.boot()?
            || install.rpc != RPC_VERSION
            || install.journal != JOURNAL_VERSION
            || !absent(&staged.join("customize.sh"))
            || ["disable", "remove"]
                .iter()
                .any(|name| !absent(&active.join(name)))
        {
            return Ok(None);
        }
        let update = marker(&active.join("update"))?;
        if !manager_finished(install.previous_marker, update)
            || read_owned(&active.join("module.prop"), 64 * 1024)?
                != read_owned(&staged.join("module.prop"), 64 * 1024)?
            || !journal_allows(paths, &install.boot)?
        {
            return Ok(None);
        }
        let active = ImageFence::capture_active(&active)?;
        for name in ["service.sh", "sepolicy.rule"] {
            if active.files.get(name).map(|f| &f.identity)
                != install.image.files.get(name).map(|f| &f.identity)
            {
                return Ok(None);
            }
        }
        install.image.require_unchanged(&staged)?;
        let file = owned_file(&staged.join("vpnhide-core"), 128 * 1024 * 1024)?;
        install.image.require_running(&file)?;
        Ok(Some(Self {
            file,
            image: install.image,
        }))
    }
}

pub(super) fn receipt(paths: &ReplacementPaths) -> io::Result<Vec<u8>> {
    read_owned(&paths.persistent.join(RECEIPT), 32 * 1024)
}

pub(super) fn ready(runtime: &tokio::runtime::Runtime, paths: &ReplacementPaths) -> bool {
    runtime.block_on(async {
        let client = vpnhide_core_client::CoreClient::connect_expected_server_uid(
            crate::app_socket::socket_path(&paths.app),
            0,
        );
        tokio::time::timeout(Duration::from_millis(300), client.current())
            .await
            .is_ok_and(|result| {
                result.is_ok_and(|published| {
                    let state = published.snapshot;
                    state.startup_error.is_none()
                        && state.storage_fault.is_none()
                        && !state.mutation_held
                        && state.config.is_some()
                        && state.backend.applied == Some(NativeBackend::Builtin)
                })
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installer_callback_cannot_reuse_an_older_manager_update_marker() {
        let stamp = |nanos| UpdateMarker {
            device: 1,
            inode: 2,
            modified: (10, nanos),
            changed: (10, nanos),
        };
        assert!(!manager_finished(None, None));
        assert!(!manager_finished(Some(stamp(1)), None));
        assert!(!manager_finished(Some(stamp(1)), Some(stamp(1))));
        assert!(manager_finished(None, Some(stamp(2))));
        assert!(manager_finished(Some(stamp(1)), Some(stamp(2))));
    }
}
