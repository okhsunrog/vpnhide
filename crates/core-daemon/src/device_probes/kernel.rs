//! Boot/kernel partition metadata and gzipped image export, for kernel bug reports.
//! Read-only on the partitions; images land in the probe's own export dir.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use super::{device_size, ls_line, run, sha256_line, trimmed};

const IMAGES: &[&str] = &["boot", "init_boot", "vendor_boot", "vendor_kernel_boot"];
/// Kernel/boot images are tens of MB; cap the copy so a mislabelled partition can't
/// fill /data.
pub const MAX_IMAGE_BYTES: u64 = 512 << 20;
const MAX_HASH_BYTES: u64 = 256 << 20;

fn slot_suffix() -> String {
    vpnhide_activator::sys::property("ro.boot.slot_suffix").unwrap_or_default()
}

/// `/dev/block/by-name/<name><slot>` or `/dev/block/by-name/<name>`.
pub fn resolve_partition(by_name: &Path, name: &str, slot: &str) -> Option<PathBuf> {
    [format!("{name}{slot}"), name.to_owned()]
        .into_iter()
        .map(|c| by_name.join(c))
        .find(|p| p.exists())
}

pub fn partitions() -> (i32, String) {
    let by_name = Path::new("/dev/block/by-name");
    let slot = slot_suffix();
    let mut s = format!("slot_suffix={slot}\n");
    let timeout = std::time::Duration::from_secs(5);
    let current = run("bootctl", &["get-current-slot"], timeout);
    let current = if current.exit == 0 {
        trimmed(&current.stdout).to_owned()
    } else {
        String::new()
    };
    s.push_str(&format!("current_slot={current}\n"));
    let suffix = run("bootctl", &["get-suffix", &current], timeout);
    s.push_str(&format!(
        "current_slot_suffix={}\n",
        if suffix.exit == 0 {
            trimmed(&suffix.stdout)
        } else {
            ""
        }
    ));
    s.push_str(&super::debug_snapshot::props(&[
        "ro.boot.slot_suffix",
        "ro.boot.slot",
        "ro.boot.bootdevice",
        "ro.boot.super_partition",
        "ro.boot.init_boot",
        "ro.boot.vendor_boot",
        "ro.boot.vbmeta.device_state",
        "ro.boot.verifiedbootstate",
        "ro.boot.dynamic_partitions",
    ]));
    s.push('\n');
    for name in IMAGES.iter().chain(&["vendor_dlkm"]) {
        s.push_str(&format!("### {name}\n"));
        let Some(path) = resolve_partition(by_name, name, &slot) else {
            s.push_str("present=0\n\n");
            continue;
        };
        let real = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        let size = device_size(&path);
        s.push_str(&format!(
            "present=1\npath={}\nreal_path={}\nsize_bytes={}\n{}\n",
            path.display(),
            real.display(),
            size.map_or_else(|| "unknown".into(), |n| n.to_string()),
            ls_line(&path)
        ));
        if *name == "vendor_dlkm" {
            s.push_str("sha256=skipped (vendor_dlkm can be large; export images separately only when needed)\n");
        } else if size.is_some_and(|n| n <= MAX_HASH_BYTES) {
            s.push_str(&sha256_line(&path));
            s.push('\n');
        } else {
            s.push_str("sha256=skipped (size above 268435456 bytes or unknown)\n");
        }
        s.push('\n');
    }
    s.push_str("### by-name entries\n");
    if let Ok(entries) = std::fs::read_dir(by_name) {
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for p in paths {
            let name = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if IMAGES
                .iter()
                .chain(&["vendor_dlkm"])
                .any(|n| name.starts_with(n))
            {
                s.push_str(&ls_line(&p));
                s.push('\n');
            }
        }
    }
    (0, s)
}

/// Gzip each present image (capped) into `out_dir`, with a manifest. Exit 0 only when at
/// least one image was captured.
pub fn images(out_dir: &Path) -> (i32, String) {
    images_from(Path::new("/dev/block/by-name"), &slot_suffix(), out_dir)
}

pub fn images_from(by_name: &Path, slot: &str, out_dir: &Path) -> (i32, String) {
    let mut manifest = format!(
        "schema=1\nslot_suffix={slot}\ncreated_utc_ms={}\nincluded={}\nskipped=vendor_dlkm (metadata only; usually large and not a kernel image)\n\n",
        super::now_ms(),
        IMAGES.join(" ")
    );
    let mut captured = false;
    for name in IMAGES {
        manifest.push_str(&format!("### {name}\n"));
        let Some(path) = resolve_partition(by_name, name, slot) else {
            manifest.push_str("present=0\n\n");
            continue;
        };
        let size = device_size(&path);
        manifest.push_str(&format!(
            "present=1\npath={}\nsize_bytes={}\n",
            path.display(),
            size.map_or_else(|| "unknown".into(), |n| n.to_string())
        ));
        match size {
            None => manifest.push_str("skipped=size unknown (refusing unbounded copy)\n"),
            Some(n) if n > MAX_IMAGE_BYTES => {
                manifest.push_str(&format!("skipped=size {n} above cap {MAX_IMAGE_BYTES}\n"))
            }
            Some(_) => {
                let base = format!("{name}{slot}.img.gz");
                let gz = out_dir.join(&base);
                match gzip_copy(&path, &gz, MAX_IMAGE_BYTES) {
                    Ok(()) => {
                        manifest.push_str(&format!("file=images/{base}\n{}\n", sha256_line(&gz)));
                        captured = true;
                    }
                    Err(e) => {
                        let _ = std::fs::remove_file(&gz);
                        manifest.push_str(&format!("copy_failed={e}\n"));
                    }
                }
            }
        }
        manifest.push('\n');
    }
    let _ = std::fs::write(out_dir.join("manifest.txt"), &manifest);
    (if captured { 0 } else { 1 }, String::new())
}

fn gzip_copy(src: &Path, dst: &Path, cap: u64) -> std::io::Result<()> {
    let input = File::open(src)?;
    let mut encoder =
        flate2::write::GzEncoder::new(File::create(dst)?, flate2::Compression::default());
    let mut reader = input.take(cap);
    let mut chunk = vec![0u8; 4 << 20];
    loop {
        let n = reader.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        encoder.write_all(&chunk[..n])?;
    }
    encoder.finish()?.sync_all()
}
