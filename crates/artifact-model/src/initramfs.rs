//! Bounded Linux test rootfs extraction and newc+gzip construction.
//! No ownership restoration, device creation, shell pipelines or symlink walks.
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use std::{
    fs::{self, File},
    io::{self, Read, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Component, Path, PathBuf},
};
const MAX_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ENTRIES: usize = 16384;
fn invalid() -> io::Error {
    io::Error::other("unsafe or excessive test rootfs")
}
fn relative(path: &Path) -> io::Result<PathBuf> {
    let mut p = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Normal(v) => p.push(v),
            Component::CurDir => {}
            _ => return Err(invalid()),
        }
    }
    if p.as_os_str().len() > 4096 || p.components().count() > 64 {
        return Err(invalid());
    }
    Ok(p)
}
pub fn extract(input: &Path, root: &Path) -> io::Result<()> {
    let bytes = crate::archive::read_regular(input, 128 * 1024 * 1024)?;
    fs::create_dir_all(root)?;
    fs::set_permissions(root, fs::Permissions::from_mode(0o755))?;
    let mut archive = tar::Archive::new(GzDecoder::new(bytes.as_slice()).take(MAX_BYTES + 1));
    let mut retained = 0u64;
    let mut names = std::collections::BTreeSet::new();
    for (i, entry) in archive.entries()?.enumerate() {
        if i >= MAX_ENTRIES {
            return Err(invalid());
        }
        let mut entry = entry?;
        let path = relative(&entry.path()?)?;
        if path.as_os_str().is_empty() {
            continue;
        }
        if !names.insert(path.clone()) {
            return Err(invalid());
        }
        retained = retained.checked_add(entry.size()).ok_or_else(invalid)?;
        if retained > MAX_BYTES {
            return Err(invalid());
        }
        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_dir() || kind.is_symlink()) {
            return Err(invalid());
        }
        // tar's unpack_in canonicalizes parents and refuses link escapes.
        if !entry.unpack_in(root)? {
            return Err(invalid());
        }
    }
    let mut remaining = archive.into_inner();
    io::copy(&mut remaining, &mut io::sink())?;
    if remaining.limit() == 0 {
        return Err(invalid());
    }
    Ok(())
}
/// Place trusted harness inputs only in real directories, never through a link
/// supplied by the input rootfs. The root is a private per-run staging tree.
pub fn place(root: &Path, name: &str, bytes: &[u8], executable: bool) -> io::Result<()> {
    let relative = relative(Path::new(name))?;
    let mut parent = root.to_path_buf();
    for c in relative.parent().ok_or_else(invalid)?.components() {
        parent.push(c);
        match fs::symlink_metadata(&parent) {
            Ok(m) if m.is_dir() => {}
            Ok(_) => return Err(invalid()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => fs::create_dir(&parent)?,
            Err(e) => return Err(e),
        }
    }
    let path = root.join(relative);
    if let Ok(m) = fs::symlink_metadata(&path) {
        if !m.is_file() || m.nlink() != 1 {
            return Err(invalid());
        }
        fs::remove_file(&path)?;
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    let mut file = options.open(&path)?;
    file.write_all(bytes)?;
    file.set_permissions(fs::Permissions::from_mode(if executable {
        0o755
    } else {
        0o644
    }))?;
    Ok(())
}
fn pad(writer: &mut impl Write, size: usize) -> io::Result<()> {
    writer.write_all(&[0; 3][..(4 - size % 4) % 4])
}
fn member(
    writer: &mut impl Write,
    name: &[u8],
    mode: u32,
    data: &[u8],
    inode: u32,
) -> io::Result<()> {
    let size = u32::try_from(data.len()).map_err(|_| invalid())?;
    let namesize = u32::try_from(name.len() + 1).map_err(|_| invalid())?;
    let header = format!(
        "070701{inode:08x}{mode:08x}{:08x}{:08x}{:08x}{:08x}{size:08x}{:08x}{:08x}{:08x}{:08x}{namesize:08x}{:08x}",
        0, 0, 1, 0, 0, 0, 0, 0, 0
    );
    writer.write_all(header.as_bytes())?;
    writer.write_all(name)?;
    writer.write_all(&[0])?;
    pad(writer, 110 + name.len() + 1)?;
    writer.write_all(data)?;
    pad(writer, data.len())
}
pub fn pack(root: &Path, output: &Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let mut entries = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(dir) = pending.pop() {
        for e in fs::read_dir(root.join(dir))? {
            let e = e?;
            let p = e
                .path()
                .strip_prefix(root)
                .map_err(|_| invalid())?
                .to_path_buf();
            let m = fs::symlink_metadata(e.path())?;
            if entries.len() >= MAX_ENTRIES {
                return Err(invalid());
            }
            if m.is_dir() {
                pending.push(p.clone());
            }
            entries.push((p, m));
        }
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut writer = GzEncoder::new(File::create(output)?, Compression::default());
    let mut total = 0u64;
    for (i, (p, m)) in entries.into_iter().enumerate() {
        let bytes = if m.is_dir() {
            Vec::new()
        } else if m.is_symlink() {
            fs::read_link(root.join(&p))?
                .as_os_str()
                .as_bytes()
                .to_vec()
        } else if m.is_file() {
            crate::archive::read_regular(&root.join(&p), MAX_BYTES)?
        } else {
            return Err(invalid());
        };
        total = total.checked_add(bytes.len() as u64).ok_or_else(invalid)?;
        if total > MAX_BYTES {
            return Err(invalid());
        }
        member(
            &mut writer,
            p.as_os_str().as_bytes(),
            m.mode() & !0o6000,
            &bytes,
            (i + 1) as u32,
        )?;
    }
    member(&mut writer, b"TRAILER!!!", 0, &[], 0)?;
    writer.finish()?.sync_all()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rootfs_links_cannot_redirect_harness_files_and_newc_is_independent() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("root");
        fs::create_dir(&root).unwrap();
        std::os::unix::fs::symlink("/tmp", root.join("escape")).unwrap();
        assert!(place(&root, "escape/harness", b"bad", true).is_err());
        assert!(place(&root, "../outside", b"bad", true).is_err());
        place(&root, "init", b"guest", true).unwrap();
        let output = t.path().join("r.gz");
        pack(&root, &output).unwrap();
        let mut raw = Vec::new();
        GzDecoder::new(File::open(output).unwrap())
            .read_to_end(&mut raw)
            .unwrap();
        assert_eq!(&raw[..6], b"070701");
        assert!(raw.windows(11).any(|v| v == b"TRAILER!!!\0"));
        assert!(raw.windows(5).any(|v| v == b"guest"));
    }
}
#[cfg(test)]
mod hostile_archive_tests {
    use super::*;
    fn encoded(name: &str, kind: u8) -> Vec<u8> {
        // Independent tar header: bypass the producer's safe path setter.
        let mut header = tar::Header::new_gnu();
        header.set_size(0);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::new(kind));
        header.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
        header.set_cksum();
        let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
        gzip.write_all(header.as_bytes()).unwrap();
        gzip.write_all(&[0u8; 1024]).unwrap();
        gzip.finish().unwrap()
    }
    #[test]
    fn extraction_rejects_absolute_parent_device_and_fifo_members() {
        for (name, kind) in [
            ("../outside", b'0'),
            ("/absolute", b'0'),
            ("device", b'3'),
            ("fifo", b'6'),
        ] {
            let t = tempfile::tempdir().unwrap();
            let input = t.path().join("r.gz");
            fs::write(&input, encoded(name, kind)).unwrap();
            assert!(extract(&input, &t.path().join("root")).is_err(), "{name}");
            assert!(!t.path().join("outside").exists());
        }
    }
}
