//! Inert discovery image preparation and exact readback. The durable command
//! writer must own manager dispatch, receipts and next-boot admission separately.
use super::{
    image::{ImageFence, ImageFile, read_file, require_layout},
    manager::Manager,
};
use crate::payloads::{Catalog, frozen::FrozenPayload, open_beneath};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{self, Read},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use vpnhide_artifact_model::zygisk_projection::{Descriptor, FILE_NAME, MAX_DESCRIPTOR_BYTES};
use vpnhide_protocol::zygisk_activation::{MANAGED_MODULE_ID, Recipient, recipient_from_manifest};

pub mod work;

const BUDGET: Duration = Duration::from_secs(30);
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "projection source, ownership or generation unavailable",
    )
}
fn directory(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}
fn identity(file: &File) -> io::Result<(u64, u64)> {
    let m = file.metadata()?;
    if !m.is_dir() {
        return Err(invalid());
    }
    Ok((m.dev(), m.ino()))
}
fn absent(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        _ => Err(invalid()),
    }
}
fn bytes(
    root: &File,
    name: &str,
    limit: u64,
    uid: u32,
    deadline: Instant,
) -> io::Result<(Vec<u8>, ImageFile)> {
    let mut file = open_beneath(root, name)?;
    let image = read_file(&file, uid, limit, deadline)?;
    let mut bytes = Vec::new();
    file.by_ref().take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 != image.identity.bytes {
        return Err(invalid());
    }
    use sha2::{Digest, Sha256};
    if format!("{:x}", Sha256::digest(&bytes)) != image.identity.sha256 {
        return Err(invalid());
    }
    Ok((bytes, image))
}
#[derive(Clone)]
pub struct ProjectionPaths {
    pub root: PathBuf,
    pub persistent: PathBuf,
}
impl ProjectionPaths {
    pub fn source(&self) -> PathBuf {
        self.root.join("data/adb/modules/vpnhide_core")
    }
    pub fn active(&self) -> PathBuf {
        self.root.join("data/adb/modules").join(MANAGED_MODULE_ID)
    }
    pub fn staged(&self) -> PathBuf {
        self.root
            .join("data/adb/modules_update")
            .join(MANAGED_MODULE_ID)
    }
    fn boot(&self) -> io::Result<String> {
        let boot =
            crate::native_boot::read_boot_id(&self.root.join("proc/sys/kernel/random/boot_id"));
        if !work::valid_boot(&boot) {
            return Err(invalid());
        }
        Ok(boot)
    }
    fn require_source_enabled(&self, uid: u32) -> io::Result<()> {
        for name in ["disable", "remove", "zygisk", ".ksu_hot_install"] {
            absent(&self.source().join(name))?;
        }
        let (prop, _) = bytes(
            &directory(&self.source())?,
            "module.prop",
            vpnhide_protocol::zygisk_activation::MAX_MODULE_PROP_BYTES as u64,
            uid,
            Instant::now() + BUDGET,
        )?;
        let properties =
            super::manager::simple_properties(std::str::from_utf8(&prop).map_err(|_| invalid())?)?;
        if properties.get("id").map(String::as_str) != Some("vpnhide_core") {
            return Err(invalid());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionImage {
    pub directory: (u64, u64),
    pub descriptor: Descriptor,
    pub files: BTreeMap<String, ImageFile>,
}
impl ProjectionImage {
    pub fn inspect(path: &Path, expected: &Descriptor) -> io::Result<Self> {
        Self::inspect_owned(path, expected, 0)
    }
    fn inspect_owned(path: &Path, expected: &Descriptor, uid: u32) -> io::Result<Self> {
        expected.validate()?;
        let root = directory(path)?;
        let stat = root.metadata()?;
        if stat.uid() != uid || stat.mode() & 0o022 != 0 {
            return Err(invalid());
        }
        let deadline = Instant::now() + BUDGET;
        let allowed = BTreeSet::from([
            "module.prop".into(),
            FILE_NAME.into(),
            "update".into(),
            Descriptor::member_name(crate::payloads::Abi::Arm64),
            Descriptor::member_name(crate::payloads::Abi::Arm32),
        ]);
        require_layout(
            &root,
            &allowed,
            &BTreeSet::from([
                Descriptor::member_name(crate::payloads::Abi::Arm64),
                Descriptor::member_name(crate::payloads::Abi::Arm32),
            ]),
            uid,
            deadline,
        )?;
        let (prop, _) = bytes(
            &root,
            "module.prop",
            vpnhide_protocol::zygisk_activation::MAX_MODULE_PROP_BYTES as u64,
            uid,
            deadline,
        )?;
        if recipient_from_manifest(&prop) != Some(Recipient::Managed) {
            return Err(invalid());
        }
        let prop = std::str::from_utf8(&prop).map_err(|_| invalid())?;
        // Friendly manager labels are mutable. An independent update channel,
        // installer or other property is not part of this managed projection.
        let properties = super::manager::simple_properties(prop)?;
        if properties.keys().any(|k| {
            !matches!(
                k.as_str(),
                "id" | "name"
                    | "version"
                    | "versionCode"
                    | "author"
                    | "description"
                    | "vpnhideControl"
                    | "vpnhideActivation"
                    | "vpnhideProjection"
            )
        }) {
            return Err(invalid());
        }
        let (metadata, metadata_image) =
            bytes(&root, FILE_NAME, MAX_DESCRIPTOR_BYTES as u64, uid, deadline)?;
        let descriptor = Descriptor::parse(&metadata)?;
        if &descriptor != expected {
            return Err(invalid());
        }
        let mut files = BTreeMap::from([(FILE_NAME.into(), metadata_image)]);
        for abi in [crate::payloads::Abi::Arm64, crate::payloads::Abi::Arm32] {
            let name = Descriptor::member_name(abi);
            let file = open_beneath(&root, &name)?;
            let actual = read_file(&file, uid, crate::payloads::MAX_PAYLOAD_BYTES, deadline)?;
            let mut header = [0; 20];
            use std::os::unix::fs::FileExt;
            file.read_exact_at(&mut header, 0)?;
            vpnhide_artifact_model::elf::shared_object_header(&header, actual.identity.bytes, abi)?;
            let wanted = descriptor
                .payloads
                .iter()
                .find(|p| p.implementation == crate::payloads::PayloadKind::Zygisk { abi })
                .ok_or_else(invalid)?;
            if actual.identity.bytes != wanted.bytes || actual.identity.sha256 != wanted.sha256 {
                return Err(invalid());
            }
            files.insert(name, actual);
        }
        if identity(&directory(path)?)? != identity(&root)? {
            return Err(invalid());
        }
        Ok(Self {
            directory: identity(&root)?,
            descriptor,
            files,
        })
    }
    fn valid_for(&self, expected: &Descriptor) -> bool {
        self.directory.1 > 0
            && self.descriptor == *expected
            && self.files.len() == 3
            && self
                .files
                .get(FILE_NAME)
                .is_some_and(|f| f.valid(MAX_DESCRIPTOR_BYTES as u64))
            && expected.payloads.iter().all(|payload| {
                let crate::payloads::PayloadKind::Zygisk { abi } = payload.implementation else {
                    return false;
                };
                self.files
                    .get(&Descriptor::member_name(abi))
                    .is_some_and(|f| {
                        f.valid(crate::payloads::MAX_PAYLOAD_BYTES)
                            && f.identity.bytes == payload.bytes
                            && f.identity.sha256 == payload.sha256
                    })
            })
    }
    pub fn require_unchanged(&self, path: &Path) -> io::Result<()> {
        self.require_unchanged_owned(path, 0)
    }
    fn require_unchanged_owned(&self, path: &Path, uid: u32) -> io::Result<()> {
        if self != &Self::inspect_owned(path, &self.descriptor, uid)? {
            return Err(invalid());
        }
        Ok(())
    }
}

pub struct PreparedProjection {
    pub descriptor: Descriptor,
    pub source: ImageFence,
    pub manager: Manager,
    paths: ProjectionPaths,
    uid: u32,
    boot_id: String,
    running: File,
    package: FrozenPayload,
}
impl PreparedProjection {
    /// This prepares inert final bytes only. It never invokes the manager.
    pub fn prepare(paths: ProjectionPaths, operation: u64) -> io::Result<Self> {
        let manager = Manager::discover(&paths.root)?;
        let running = File::open("/proc/self/exe")?;
        Self::prepare_owned(paths, operation, manager, running, 0)
    }
    fn prepare_owned(
        paths: ProjectionPaths,
        operation: u64,
        manager: Manager,
        running: File,
        uid: u32,
    ) -> io::Result<Self> {
        let boot_id = paths.boot()?;
        absent(&paths.active())?;
        absent(&paths.staged())?;
        paths.require_source_enabled(uid)?;
        let persistent = directory(&paths.persistent)?;
        let stat = persistent.metadata()?;
        if stat.uid() != uid || stat.mode() & 0o077 != 0 {
            return Err(invalid());
        }
        let source = ImageFence::inspect(&paths.source(), None, crate::payloads::Abi::Arm64, uid)?;
        source.require_running_owned(&running, uid)?;
        let root = directory(&paths.source())?;
        let catalog = Catalog::open_pinned_before(&root, Some(Instant::now() + BUDGET))?;
        catalog.manifest.require_complete()?;
        let (manifest, _) = bytes(
            &root,
            "payloads/manifest.json",
            crate::payloads::MAX_MANIFEST_BYTES,
            uid,
            Instant::now() + BUDGET,
        )?;
        let descriptor = Descriptor::bind(&manifest, operation)?;
        let mut libraries = Vec::new();
        for payload in &descriptor.payloads {
            let source_file = open_beneath(&root, &payload.implementation.relative_path()?)?;
            let snapshot = FrozenPayload::copy(&source_file)?;
            if snapshot.sha256 != payload.sha256 || snapshot.bytes != payload.bytes {
                return Err(invalid());
            }
            let mut data = vec![0; usize::try_from(snapshot.bytes).map_err(|_| invalid())?];
            use std::os::unix::fs::FileExt;
            snapshot.file().read_exact_at(&mut data, 0)?;
            libraries.push(data);
        }
        let mut libraries = libraries.into_iter();
        let files = descriptor.files_owned(
            libraries.next().ok_or_else(invalid)?,
            libraries.next().ok_or_else(invalid)?,
        )?;
        let parent = PathBuf::from(format!("/proc/self/fd/{}", persistent.as_raw_fd()));
        let temporary = tempfile::Builder::new()
            .prefix("projection-prepare-")
            .tempdir_in(parent)?;
        let package_path = temporary.path().join("projection.zip");
        vpnhide_artifact_model::archive::write_archive(&package_path, &files)?;
        drop(files);
        let package = FrozenPayload::copy_with_limit(
            &File::open(package_path)?,
            crate::payloads::MAX_BUNDLE_BYTES,
        )?;
        temporary.close()?;
        let prepared = Self {
            descriptor,
            source,
            manager,
            paths,
            uid,
            boot_id,
            running,
            package,
        };
        prepared.require_fresh()?;
        Ok(prepared)
    }
    pub fn require_fresh(&self) -> io::Result<()> {
        if self.paths.boot()? != self.boot_id {
            return Err(invalid());
        }
        self.paths.require_source_enabled(self.uid)?;
        self.source
            .require_unchanged_owned(&self.paths.source(), self.uid)?;
        self.source.require_running_owned(&self.running, self.uid)?;
        self.manager.require_unchanged()?;
        absent(&self.paths.active())?;
        absent(&self.paths.staged())
    }
    /// The caller must journal dispatch intent before passing this immutable FD
    /// to Manager::stage. A successful exit alone never verifies publication.
    pub fn package(&self) -> &File {
        self.package.file()
    }
    pub fn verify_staged(&self) -> io::Result<ProjectionStaging> {
        if self.paths.boot()? != self.boot_id {
            return Err(invalid());
        }
        self.paths.require_source_enabled(self.uid)?;
        self.source
            .require_unchanged_owned(&self.paths.source(), self.uid)?;
        self.source.require_running_owned(&self.running, self.uid)?;
        ProjectionStaging::inspect_owned(&self.paths, &self.descriptor, self.uid)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ActiveProjection {
    Placeholder { directory: (u64, u64) },
    Complete { image: Box<ProjectionImage> },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionStaging {
    pub staged: Option<ProjectionImage>,
    pub active: Option<ActiveProjection>,
}
impl ProjectionStaging {
    fn valid_for(&self, expected: &Descriptor) -> bool {
        (self.staged.is_some() || matches!(self.active, Some(ActiveProjection::Complete { .. })))
            && self
                .staged
                .as_ref()
                .is_none_or(|image| image.valid_for(expected))
            && self.active.as_ref().is_none_or(|active| match active {
                ActiveProjection::Placeholder { directory } => {
                    directory.1 > 0 && self.staged.is_some()
                }
                ActiveProjection::Complete { image } => image.valid_for(expected),
            })
    }
    fn inspect_owned(paths: &ProjectionPaths, expected: &Descriptor, uid: u32) -> io::Result<Self> {
        let staged = match fs::symlink_metadata(paths.staged()) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
            Ok(_) => Some(ProjectionImage::inspect_owned(
                &paths.staged(),
                expected,
                uid,
            )?),
        };
        let active = match fs::symlink_metadata(paths.active()) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
            Ok(_) => {
                let root = directory(&paths.active())?;
                let meta = root.metadata()?;
                if meta.uid() != uid || meta.mode() & 0o022 != 0 {
                    return Err(invalid());
                }
                match fs::symlink_metadata(paths.active().join(FILE_NAME)) {
                    Ok(_) => Some(ActiveProjection::Complete {
                        image: Box::new(ProjectionImage::inspect_owned(
                            &paths.active(),
                            expected,
                            uid,
                        )?),
                    }),
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {
                        require_layout(
                            &root,
                            &BTreeSet::from(["module.prop".into(), "update".into()]),
                            &BTreeSet::new(),
                            uid,
                            Instant::now() + BUDGET,
                        )?;
                        let (prop, _) = bytes(
                            &root,
                            "module.prop",
                            vpnhide_protocol::zygisk_activation::MAX_MODULE_PROP_BYTES as u64,
                            uid,
                            Instant::now() + BUDGET,
                        )?;
                        // A placeholder is never a ready image or authority. Its
                        // exact generated metadata only accompanies a full stage.
                        if prop != Descriptor::module_prop() {
                            return Err(invalid());
                        }
                        Some(ActiveProjection::Placeholder {
                            directory: identity(&root)?,
                        })
                    }
                    Err(e) => return Err(e),
                }
            }
        };
        if staged.is_none() && !matches!(active, Some(ActiveProjection::Complete { .. })) {
            return Err(invalid());
        }
        Ok(Self { staged, active })
    }
    pub fn require_unchanged(
        &self,
        paths: &ProjectionPaths,
        expected: &Descriptor,
    ) -> io::Result<()> {
        self.require_unchanged_owned(paths, expected, 0)
    }
    fn require_unchanged_owned(
        &self,
        paths: &ProjectionPaths,
        expected: &Descriptor,
        uid: u32,
    ) -> io::Result<()> {
        if self != &Self::inspect_owned(paths, expected, uid)? {
            return Err(invalid());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
