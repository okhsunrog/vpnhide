//! Passive loaded-module/file binding. A directory hit or a manager's name is
//! never current-kernel evidence. No syscalls load, unload or alter modules here.
use super::elf::{self, ModuleMetadata};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{self, Read},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const ROOTS: [&str; 5] = [
    "/vendor_dlkm/lib/modules",
    "/system_dlkm/lib/modules",
    "/vendor/lib/modules",
    "/odm/lib/modules",
    "/odm_dlkm/lib/modules",
];
const MAX_FILES: usize = 512;
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
const MAX_MODULES: usize = 32;
pub(crate) mod probe;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundModule {
    pub source: PathBuf,
    pub sha256: String,
    pub metadata: ModuleMetadata,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Observation {
    pub modules: Vec<BoundModule>,
    pub unbound_files: usize,
    /// Partial scan cannot settle a candidate tie. Valid contrary evidence is
    /// retained even when the remaining inventory cannot be observed.
    pub incomplete: bool,
}

fn directory(path: &Path, owner: u32) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let metadata = file.metadata()?;
    if metadata.uid() != owner || metadata.mode() & 0o022 != 0 {
        return Err(io::Error::other("foreign module directory"));
    }
    Ok(file)
}
fn bounded(file: File, limit: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(io::Error::other("module evidence exceeds bound"));
    }
    Ok(bytes)
}
fn sys_bytes(sys: &File, name: &str, leaf: &str, limit: u64) -> io::Result<Vec<u8>> {
    bounded(super::open_beneath(sys, &format!("{name}/{leaf}"))?, limit)
}
fn live_identity(sys: &File, name: &str) -> io::Result<Vec<u8>> {
    if sys_bytes(sys, name, "initstate", 32)? != b"live\n"
        || sys_bytes(sys, name, "taint", 64)?.contains(&b'F')
    {
        return Err(io::Error::other("module not live or force-loaded"));
    }
    elf::gnu_build_id(&sys_bytes(sys, name, "notes/.note.gnu.build-id", 4096)?)
}
fn bind(
    sys: &File,
    file: File,
    source: PathBuf,
    owner: u32,
    budget: &mut u64,
) -> io::Result<BoundModule> {
    let before = file.metadata()?;
    if before.len() > *budget {
        *budget = 0;
        return Err(io::Error::other("module inventory byte budget exhausted"));
    }
    if before.uid() != owner || before.mode() & 0o022 != 0 || before.len() > MAX_FILE_BYTES {
        return Err(io::Error::other("module file identity or byte budget"));
    }
    *budget -= before.len();
    let bytes = bounded(file.try_clone()?, before.len())?;
    let metadata = elf::module_metadata(&bytes)?;
    // Linux normalizes '-' to '_' for its runtime module identifier.
    let name = metadata.name.replace('-', "_");
    let live = live_identity(sys, &name)?;
    if metadata.build_id.as_ref() != Some(&live) {
        return Err(io::Error::other(
            "loaded/file build ID differs or is absent",
        ));
    }
    if let Some(version) = &metadata.srcversion {
        let source = sys_bytes(sys, &name, "srcversion", 128)?;
        if source.strip_suffix(b"\n").unwrap_or(&source) != version.as_bytes() {
            return Err(io::Error::other("loaded/file source version differs"));
        }
    }
    let after = file.metadata()?;
    if bytes.len() as u64 != before.len()
        || before.len() != after.len()
        || before.mtime() != after.mtime()
        || before.mtime_nsec() != after.mtime_nsec()
        || before.ctime() != after.ctime()
        || before.ctime_nsec() != after.ctime_nsec()
        || live_identity(sys, &name)? != live
    {
        return Err(io::Error::other(
            "module identity changed during observation",
        ));
    }
    Ok(BoundModule {
        source,
        sha256: format!("{:x}", Sha256::digest(&bytes)),
        metadata,
    })
}

/// Inspect only the fixed Android module locations. Missing evidence is ordinary
/// Unknown; compressed modules and symlink roots are not silently decoded/followed.
pub fn observe() -> Observation {
    probe::observe()
}
fn observe_at(
    roots: &[PathBuf],
    sys_path: &Path,
    owner: u32,
    mut emit: impl FnMut(&BoundModule) -> io::Result<()>,
) -> Observation {
    let mut observation = Observation::default();
    let Ok(sys) = directory(sys_path, owner) else {
        observation.incomplete = true;
        return observation;
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut budget = MAX_TOTAL_BYTES;
    let mut seen_directories = BTreeSet::new();
    let mut queue: Vec<_> = roots.iter().map(|root| (root.clone(), 0)).collect();
    let mut count = 0;
    while let Some((root, depth)) = queue.pop() {
        let dir = match directory(&root, owner) {
            Ok(dir) => dir,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => {
                observation.incomplete = true;
                continue;
            }
        };
        let Ok(identity) = dir.metadata() else {
            observation.incomplete = true;
            continue;
        };
        if !seen_directories.insert((identity.dev(), identity.ino())) {
            continue;
        }
        let Ok(entries) = fs::read_dir(format!("/proc/self/fd/{}", dir.as_raw_fd())) else {
            observation.incomplete = true;
            continue;
        };
        for entry in entries {
            count += 1;
            if count > MAX_FILES
                || Instant::now() >= deadline
                || observation.modules.len() >= MAX_MODULES
            {
                observation.incomplete = true;
                return observation;
            }
            let Ok(entry) = entry else {
                observation.incomplete = true;
                continue;
            };
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                observation.incomplete = true;
                continue;
            };
            let Ok(kind) = entry.file_type() else {
                observation.incomplete = true;
                continue;
            };
            if kind.is_dir() && depth < 1 {
                // The queued directory is independently pinned and owner-checked.
                queue.push((root.join(name), depth + 1));
                continue;
            }
            if !name.ends_with(".ko") {
                continue;
            }
            let module = super::open_beneath(&dir, name)
                .and_then(|file| bind(&sys, file, root.join(name), owner, &mut budget));
            match module {
                Ok(module) => {
                    if emit(&module).is_err() {
                        observation.incomplete = true;
                        return observation;
                    }
                    observation.modules.push(module);
                }
                Err(_) => observation.unbound_files += 1,
            }
            if budget == 0 {
                observation.incomplete = true;
                return observation;
            }
        }
    }
    observation
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpnhide_artifact_model::fixtures::{Section, relocatable};
    fn fixture(root: &Path, module: &str, id: u8) {
        fs::create_dir_all(root.join("files")).unwrap();
        let sys = root.join("sys").join(module);
        fs::create_dir_all(sys.join("notes")).unwrap();
        let note = [
            4, 0, 0, 0, 4, 0, 0, 0, 3, 0, 0, 0, b'G', b'N', b'U', 0, id, 2, 3, 4,
        ];
        let bytes = relocatable(
            format!("name={module}\0vermagic=5.15.123-android13 SMP aarch64\0srcversion=ABC\0")
                .as_bytes(),
            &[Section::new(".note.gnu.build-id", note.to_vec(), 7)],
        );
        fs::write(root.join("files").join(format!("{module}.ko")), bytes).unwrap();
        fs::write(sys.join("initstate"), "live\n").unwrap();
        fs::write(sys.join("taint"), "OE\n").unwrap();
        fs::write(sys.join("srcversion"), "ABC\n").unwrap();
        fs::write(sys.join("notes/.note.gnu.build-id"), note).unwrap();
    }
    fn read(root: &Path) -> Observation {
        observe_at(
            &[root.join("files")],
            &root.join("sys"),
            fs::metadata(root).unwrap().uid(),
            |_| Ok(()),
        )
    }
    #[test]
    fn binds_real_elf_to_live_identity_not_a_ksu_filename() {
        let root = tempfile::tempdir().unwrap();
        fixture(root.path(), "ksu_next", 1);
        fixture(root.path(), "wifi", 2);
        let records = read(root.path());
        assert!(!records.incomplete);
        assert_eq!(records.modules.len(), 2);
        assert_eq!(records.unbound_files, 0);
        let note = root.path().join("sys/ksu_next/notes/.note.gnu.build-id");
        let mut different = fs::read(&note).unwrap();
        different[16] = 99;
        fs::write(note, different).unwrap();
        let records = read(root.path());
        assert_eq!(records.modules.len(), 1);
        assert_eq!(records.unbound_files, 1);
    }
    #[test]
    fn stale_forced_unloaded_and_symlinked_files_are_unbound() {
        let root = tempfile::tempdir().unwrap();
        fixture(root.path(), "ksu_next", 1);
        let sys = root.path().join("sys/ksu_next");
        for (leaf, value) in [
            ("srcversion", "DEF\n"),
            ("taint", "F\n"),
            ("initstate", "coming\n"),
        ] {
            let original = fs::read(sys.join(leaf)).unwrap();
            fs::write(sys.join(leaf), value).unwrap();
            assert!(read(root.path()).modules.is_empty());
            fs::write(sys.join(leaf), original).unwrap();
        }
        fs::rename(
            root.path().join("files/ksu_next.ko"),
            root.path().join("real.ko"),
        )
        .unwrap();
        std::os::unix::fs::symlink(
            root.path().join("real.ko"),
            root.path().join("files/ksu_next.ko"),
        )
        .unwrap();
        assert!(read(root.path()).modules.is_empty());
        fs::remove_dir_all(sys).unwrap();
        assert!(read(root.path()).modules.is_empty());
    }

    #[test]
    fn scan_limits_and_untrusted_directory_permissions_never_claim_completeness() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        fixture(root.path(), "wifi", 1);
        for index in 0..MAX_FILES {
            fs::write(root.path().join("files").join(index.to_string()), []).unwrap();
        }
        assert!(read(root.path()).incomplete);
        fs::set_permissions(root.path().join("files"), fs::Permissions::from_mode(0o777)).unwrap();
        let observed = read(root.path());
        assert!(observed.incomplete);
        assert!(observed.modules.is_empty());
    }
}
