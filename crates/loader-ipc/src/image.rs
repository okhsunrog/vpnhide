//! Identify the actual executable backing inode. Paths reported by dladdr or a
//! module directory do not identify the memfd that a manager already mapped.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{FileExt, MetadataExt};
use std::path::Path;

const MAX_MAPS: u64 = 1024 * 1024;
const MAX_IMAGE: u64 = 128 * 1024 * 1024;

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub fn bounded_text(path: &Path, limit: u64) -> io::Result<String> {
    let mut text = String::new();
    File::open(path)?
        .take(limit + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > limit {
        return Err(invalid("process evidence exceeds budget"));
    }
    Ok(text)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub start_ticks: u64,
}

pub fn process_identity(proc_root: &Path, pid: u32) -> io::Result<ProcessIdentity> {
    if pid == 0 {
        return Err(invalid("invalid process id"));
    }
    let stat = bounded_text(&proc_root.join(pid.to_string()).join("stat"), 8192)?;
    let prefix = format!("{pid} (");
    if !stat.starts_with(&prefix) {
        return Err(invalid("process stat identity differs"));
    }
    let (_, fields) = stat
        .rsplit_once(") ")
        .ok_or_else(|| invalid("invalid process stat"))?;
    let start_ticks = fields
        .split_whitespace()
        .nth(19)
        .ok_or_else(|| invalid("missing process starttime"))?
        .parse()
        .map_err(|_| invalid("invalid process starttime"))?;
    Ok(ProcessIdentity { pid, start_ticks })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mapping {
    pub start: u64,
    pub end: u64,
    pub device_major: u32,
    pub device_minor: u32,
    pub inode: u64,
}

pub fn mapping_in(text: &str, anchor: u64) -> io::Result<Mapping> {
    if anchor == 0 || text.len() as u64 > MAX_MAPS {
        return Err(invalid("invalid executable mapping request"));
    }
    let mut found = None;
    for line in text.lines() {
        if line.len() > 8192 {
            return Err(invalid("mapping record exceeds budget"));
        }
        let mut fields = line.split_whitespace();
        let range = fields
            .next()
            .ok_or_else(|| invalid("missing mapping interval"))?;
        let (start, end) = range
            .split_once('-')
            .ok_or_else(|| invalid("invalid mapping interval"))?;
        let start =
            u64::from_str_radix(start, 16).map_err(|_| invalid("invalid mapping address"))?;
        let end = u64::from_str_radix(end, 16).map_err(|_| invalid("invalid mapping address"))?;
        let permissions = fields
            .next()
            .ok_or_else(|| invalid("missing mapping permissions"))?;
        if !(start <= anchor && anchor < end) {
            continue;
        }
        if found.is_some()
            || permissions.as_bytes().get(2) != Some(&b'x')
            || permissions.as_bytes().get(1) == Some(&b'w')
        {
            return Err(invalid("ambiguous or mutable executable mapping"));
        }
        fields
            .next()
            .ok_or_else(|| invalid("missing mapping offset"))?;
        let (major, minor) = fields
            .next()
            .and_then(|d| d.split_once(':'))
            .ok_or_else(|| invalid("missing mapping device"))?;
        let inode = fields
            .next()
            .ok_or_else(|| invalid("missing mapping inode"))?
            .parse()
            .map_err(|_| invalid("invalid mapping inode"))?;
        if inode == 0 {
            return Err(invalid(
                "anonymous executable mapping has no backing identity",
            ));
        }
        found = Some(Mapping {
            start,
            end,
            device_major: u32::from_str_radix(major, 16)
                .map_err(|_| invalid("invalid mapping device"))?,
            device_minor: u32::from_str_radix(minor, 16)
                .map_err(|_| invalid("invalid mapping device"))?,
            inode,
        });
    }
    found.ok_or_else(|| invalid("executable anchor not mapped"))
}

pub fn mapping(proc_root: &Path, pid: u32, anchor: u64) -> io::Result<Mapping> {
    mapping_in(
        &bounded_text(&proc_root.join(pid.to_string()).join("maps"), MAX_MAPS)?,
        anchor,
    )
}

pub fn matches(file: &File, mapping: Mapping) -> io::Result<bool> {
    let metadata = file.metadata()?;
    #[cfg(all(target_os = "android", target_pointer_width = "32"))]
    let device = u32::try_from(metadata.dev()).map_err(io::Error::other)?;
    #[cfg(not(all(target_os = "android", target_pointer_width = "32")))]
    let device = metadata.dev();
    #[cfg(target_os = "android")]
    let (major, minor) = (
        u32::try_from(libc::major(device)).map_err(io::Error::other)?,
        u32::try_from(libc::minor(device)).map_err(io::Error::other)?,
    );
    #[cfg(not(target_os = "android"))]
    let (major, minor) = (libc::major(device), libc::minor(device));
    Ok(metadata.is_file()
        && metadata.ino() == mapping.inode
        && major == mapping.device_major
        && minor == mapping.device_minor)
}

/// Neo keeps the library memfd open through pre-specialization. Duplicate its
/// matching inode before returning; never reopen dladdr's synthetic pathname or
/// assume a reported `/proc/self/fd/N` still denotes that image.
pub fn own_backing_image(anchor: u64) -> io::Result<File> {
    let expected = mapping(Path::new("/proc"), std::process::id(), anchor)?;
    for entry in fs::read_dir("/proc/self/fd")?.take(4096) {
        let entry = entry?;
        let Some(raw) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<libc::c_int>().ok())
            .filter(|n| *n >= 0)
        else {
            continue;
        };
        let duplicate = unsafe { libc::fcntl(raw, libc::F_DUPFD_CLOEXEC, 0) };
        if duplicate < 0 {
            continue;
        }
        // SAFETY: fcntl created a fresh owned fd; every nonmatch drops it.
        let file = unsafe { File::from_raw_fd(duplicate) };
        if matches(&file, expected)? {
            return Ok(file);
        }
    }
    Err(invalid("mapped backing descriptor unavailable"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageAbi {
    Arm64,
    Arm32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageIdentity {
    pub abi: ImageAbi,
    pub bytes: u64,
    pub sha256: [u8; 32],
}

pub fn inspect(file: &File) -> io::Result<ImageIdentity> {
    inspect_before(file, None)
}

pub fn inspect_before(
    file: &File,
    deadline: Option<std::time::Instant>,
) -> io::Result<ImageIdentity> {
    let before = file.metadata()?;
    if !before.is_file() || before.len() < 64 || before.len() > MAX_IMAGE {
        return Err(invalid("invalid mapped image size/type"));
    }
    let seals = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GET_SEALS) };
    let required = libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK;
    if seals >= 0 {
        if seals & required != required {
            return Err(invalid("mapped memfd lacks immutable seals"));
        }
    } else if before.uid() != 0 || before.mode() & 0o022 != 0 || before.nlink() == 0 {
        return Err(invalid(
            "unsealed mapped image is not a protected root-owned file",
        ));
    }
    let mut header = [0; 20];
    file.read_exact_at(&mut header, 0)?;
    if header[..4] != *b"\x7fELF"
        || header[5..7] != [1, 1]
        || u16::from_le_bytes([header[16], header[17]]) != 3
    {
        return Err(invalid(
            "mapped image is not a little-endian ELF shared object",
        ));
    }
    let abi = match (header[4], u16::from_le_bytes([header[18], header[19]])) {
        (2, 183) => ImageAbi::Arm64,
        (1, 40) => ImageAbi::Arm32,
        _ => return Err(invalid("mapped image has unsupported architecture")),
    };
    let mut digest = Sha256::new();
    let mut offset = 0;
    let mut buffer = [0; 64 * 1024];
    while offset < before.len() {
        if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "loaded image inspection deadline",
            ));
        }
        let count = (before.len() - offset).min(buffer.len() as u64) as usize;
        file.read_exact_at(&mut buffer[..count], offset)?;
        digest.update(&buffer[..count]);
        offset += count as u64;
    }
    let after = file.metadata()?;
    if before.len() != after.len()
        || before.dev() != after.dev()
        || before.ino() != after.ino()
        || before.mtime() != after.mtime()
        || before.mtime_nsec() != after.mtime_nsec()
        || before.ctime() != after.ctime()
        || before.ctime_nsec() != after.ctime_nsec()
    {
        return Err(invalid("mapped image changed during observation"));
    }
    Ok(ImageIdentity {
        abi,
        bytes: before.len(),
        sha256: digest.finalize().into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, Write};

    fn image(sealed: bool) -> File {
        let raw = unsafe {
            libc::memfd_create(
                c"vpnhide-loader-test".as_ptr(),
                libc::MFD_ALLOW_SEALING | libc::MFD_CLOEXEC,
            )
        };
        assert!(raw >= 0, "{}", io::Error::last_os_error());
        let mut file = unsafe { File::from_raw_fd(raw) };
        let mut elf = vec![0; 4096];
        elf[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        elf[16..18].copy_from_slice(&3u16.to_le_bytes());
        elf[18..20].copy_from_slice(&183u16.to_le_bytes());
        file.write_all(&elf).unwrap();
        if sealed {
            assert_eq!(
                unsafe {
                    libc::fcntl(
                        raw,
                        libc::F_ADD_SEALS,
                        libc::F_SEAL_WRITE
                            | libc::F_SEAL_GROW
                            | libc::F_SEAL_SHRINK
                            | libc::F_SEAL_SEAL,
                    )
                },
                0
            );
        }
        file
    }

    #[test]
    fn sealed_actual_mapping_matches_inode_not_identical_path_or_content() {
        let mut file = image(true);
        let other = image(true);
        let mapped = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                4096,
                libc::PROT_READ | libc::PROT_EXEC,
                libc::MAP_PRIVATE,
                file.as_raw_fd(),
                0,
            )
        };
        assert_ne!(mapped, libc::MAP_FAILED, "{}", io::Error::last_os_error());
        let location = mapping(
            Path::new("/proc"),
            std::process::id(),
            mapped as usize as u64,
        )
        .unwrap();
        assert!(matches(&file, location).unwrap());
        assert!(!matches(&other, location).unwrap());
        let duplicated = own_backing_image(mapped as usize as u64).unwrap();
        assert_eq!(
            duplicated.metadata().unwrap().ino(),
            file.metadata().unwrap().ino()
        );
        file.rewind().unwrap();
        let before = file.stream_position().unwrap();
        let identity = inspect(&file).unwrap();
        assert_eq!(identity.abi, ImageAbi::Arm64);
        assert_eq!(identity.bytes, 4096);
        assert_eq!(identity.sha256, inspect(&other).unwrap().sha256);
        assert_eq!(file.stream_position().unwrap(), before);
        assert!(file.write_all(b"changed").is_err());
        assert_eq!(unsafe { libc::munmap(mapped, 4096) }, 0);
    }

    #[test]
    fn unsealed_memfd_and_anonymous_or_writable_code_never_prove_loaded_image() {
        assert!(inspect(&image(false)).is_err());
        assert!(mapping_in("1000-2000 r-xp 0000 00:00 0 /anonymous\n", 0x1234).is_err());
        assert!(mapping_in("1000-2000 rwxp 0000 00:01 7 /mutable\n", 0x1234).is_err());
        assert!(
            mapping_in(
                "1000-2000 r-xp 0000 00:01 7 /image\n1000-2000 r-xp 0000 00:01 8 /other\n",
                0x1234
            )
            .is_err()
        );
        assert!(mapping_in("1000-2000 r-xp 0000 00:01 7 /image\n", 0x2000).is_err());
    }

    #[test]
    fn kernel_starttime_is_bounded_and_bound_to_pid() {
        let identity = process_identity(Path::new("/proc"), std::process::id()).unwrap();
        assert_eq!(identity.pid, std::process::id());
        assert!(identity.start_ticks > 0);
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("123")).unwrap();
        let suffix = (0..20).map(|i| i.to_string()).collect::<Vec<_>>().join(" ");
        fs::write(
            root.path().join("123/stat"),
            format!("123 (name) tricky ) {suffix}"),
        )
        .unwrap();
        assert_eq!(process_identity(root.path(), 123).unwrap().start_ticks, 19);
        fs::write(root.path().join("123/stat"), format!("456 (name) {suffix}")).unwrap();
        assert!(process_identity(root.path(), 123).is_err());
        fs::write(root.path().join("123/stat"), "x".repeat(8193)).unwrap();
        assert!(process_identity(root.path(), 123).is_err());
    }
}
