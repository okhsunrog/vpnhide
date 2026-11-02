//! Automatic, bounded framework evidence for the existing user-triggered export.
//! Telemetry is evidence, never authority to read arbitrary root-accessible files.
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::json;
use sha2::{Digest, Sha256};

const TOTAL: u64 = 96 << 20;
const PER_FILE: u64 = 64 << 20;
const MAX_FILES: usize = 12;
const TEXT_LIMIT: u64 = 128 << 10;

fn allowed(path: &Path) -> bool {
    let text = path.to_string_lossy();
    let extension = path.extension().and_then(|v| v.to_str());
    if !matches!(extension, Some("jar" | "apk")) || text.len() > 1024 {
        return false;
    }
    if path.components().any(|c| {
        !matches!(
            c,
            std::path::Component::RootDir | std::path::Component::Normal(_)
        )
    }) {
        return false;
    }
    [
        "/system/framework/",
        "/system_ext/framework/",
        "/product/framework/",
        "/vendor/framework/",
    ]
    .iter()
    .any(|prefix| text.starts_with(prefix))
        || (text.starts_with("/apex/") && text.split('/').nth(3) == Some("javalib"))
}

fn bounded_text(path: &Path) -> io::Result<String> {
    let mut text = String::new();
    crate::export::open_regular(path)?
        .take(TEXT_LIMIT + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > TEXT_LIMIT {
        return Err(io::Error::other("telemetry exceeds 128 KiB"));
    }
    Ok(text)
}

fn candidates(telemetry: &str, root: &Path) -> Vec<PathBuf> {
    let mut paths = BTreeSet::new();
    // Explicit origin records, produced only for known hook target classes.
    // Unknown/stale producers still cannot escape the filesystem allowlist.
    for line in telemetry
        .lines()
        .filter(|l| l.starts_with("meta java_origin_"))
    {
        if let Some(path) = line.splitn(3, ' ').nth(2)
            && paths.len() < 32
        {
            paths.insert(PathBuf::from(path));
        }
    }
    for path in [
        "/system/framework/framework.jar",
        "/system/framework/services.jar",
    ] {
        paths.insert(path.into());
    }
    // Fallback when LSPosed never initialized: known Connectivity containers on
    // the active APEX mounts; no recursive scan or app APK inventory.
    if let Ok(entries) = fs::read_dir(root.join("apex")) {
        for entry in entries.take(128).flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.contains('@')
                || !matches!(
                    name.as_str(),
                    "com.android.tethering" | "com.android.connectivity"
                )
            {
                continue;
            }
            for jar in [
                "framework-connectivity.jar",
                "framework-connectivity-t.jar",
                "service-connectivity.jar",
            ] {
                paths.insert(PathBuf::from(format!("/apex/{name}/javalib/{jar}")));
            }
        }
    }
    paths.into_iter().collect()
}

fn open_source(root: &Path, source: &Path) -> io::Result<(File, PathBuf)> {
    if !allowed(source) {
        return Err(io::Error::other("source outside framework allowlist"));
    }
    let real = fs::canonicalize(root.join(source.strip_prefix("/").map_err(io::Error::other)?))?;
    let relative = real.strip_prefix(root).map_err(io::Error::other)?;
    let canonical_source = Path::new("/").join(relative);
    if !allowed(&canonical_source) {
        return Err(io::Error::other(
            "resolved source outside framework allowlist",
        ));
    }
    // Resolve first for Android's normal partition/APEX symlinks, then walk every
    // resolved component with O_NOFOLLOW. Replacing a parent cannot redirect reads.
    let file = crate::export::open_regular(&real)?;
    let meta = file.metadata()?;
    if meta.nlink() != 1 || meta.mode() & 0o022 != 0 {
        return Err(io::Error::other("mutable or hardlinked source"));
    }
    #[cfg(not(test))]
    if meta.uid() != 0 {
        return Err(io::Error::other("framework source is not root-owned"));
    }
    Ok((file, canonical_source))
}

/// Read ZIP directory metadata only; never inflate or execute contents. ZIP64 and
/// malformed/oversized directories are explicitly unsupported, not called DEX-free.
fn dex_status(file: &mut File) -> io::Result<&'static str> {
    let length = file.metadata()?.len();
    let tail_len = length.min(65557) as usize;
    file.seek(SeekFrom::End(-(tail_len as i64)))?;
    let mut tail = vec![0; tail_len];
    file.read_exact(&mut tail)?;
    let pos = tail
        .windows(4)
        .rposition(|v| v == b"PK\x05\x06")
        .ok_or_else(|| io::Error::other("no ZIP directory"))?;
    let end = tail
        .get(pos..pos + 22)
        .ok_or_else(|| io::Error::other("short ZIP directory"))?;
    let count = u16::from_le_bytes([end[10], end[11]]) as usize;
    let size = u32::from_le_bytes(end[12..16].try_into().unwrap()) as usize;
    let offset = u32::from_le_bytes(end[16..20].try_into().unwrap()) as u64;
    if count > 8192 || size > 2 << 20 || offset + size as u64 > length {
        return Err(io::Error::other("unsupported ZIP directory limits"));
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut directory = vec![0; size];
    file.read_exact(&mut directory)?;
    let mut cursor = 0;
    let mut found = false;
    for _ in 0..count {
        let header = directory
            .get(cursor..cursor + 46)
            .ok_or_else(|| io::Error::other("short ZIP entry"))?;
        if &header[..4] != b"PK\x01\x02" {
            return Err(io::Error::other("invalid ZIP entry"));
        }
        let name_len = u16::from_le_bytes([header[28], header[29]]) as usize;
        let extra = u16::from_le_bytes([header[30], header[31]]) as usize;
        let comment = u16::from_le_bytes([header[32], header[33]]) as usize;
        let name = directory
            .get(cursor + 46..cursor + 46 + name_len)
            .ok_or_else(|| io::Error::other("short ZIP name"))?;
        found |= name.starts_with(b"classes") && name.ends_with(b".dex") && !name.contains(&b'/');
        cursor += 46 + name_len + extra + comment;
        if cursor > directory.len() {
            return Err(io::Error::other("short ZIP metadata"));
        }
    }
    Ok(if found {
        "dex_present"
    } else {
        "dex_absent_no_odex_reconstruction"
    })
}

fn capture(
    root: &Path,
    source: &Path,
    destination: &Path,
    remaining: u64,
    deadline: Instant,
) -> io::Result<serde_json::Value> {
    let (mut input, canonical) = open_source(root, source)?;
    let before = input.metadata()?;
    if before.len() > PER_FILE || before.len() > remaining {
        return Err(io::Error::other("file/total byte budget exceeded"));
    }
    let dex = dex_status(&mut input)
        .map(str::to_owned)
        .unwrap_or_else(|e| format!("unknown: {e}"));
    input.rewind()?;
    let mut output = File::options()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let mut hasher = Sha256::new();
    let mut copied = 0;
    let mut buffer = [0u8; 65536];
    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::other("collection deadline exceeded"));
        }
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        copied += n as u64;
        if copied > before.len() || copied > remaining {
            return Err(io::Error::other("source grew during copy"));
        }
        output.write_all(&buffer[..n])?;
        hasher.update(&buffer[..n]);
    }
    let after = input.metadata()?;
    if copied != before.len()
        || before.mtime() != after.mtime()
        || before.mtime_nsec() != after.mtime_nsec()
        || before.ctime() != after.ctime()
        || before.ctime_nsec() != after.ctime_nsec()
    {
        return Err(io::Error::other("source changed during copy"));
    }
    output.sync_all()?;
    Ok(
        json!({"source": source, "resolved": canonical, "bytes":copied, "sha256":format!("{:x}",hasher.finalize()), "dex":dex, "file":destination.file_name()}),
    )
}

pub fn collect(out: &Path, include_files: bool) -> (i32, String) {
    collect_from(Path::new("/"), out, Duration::from_secs(25), include_files)
}

fn collect_from(root: &Path, out: &Path, budget: Duration, include_files: bool) -> (i32, String) {
    let deadline = Instant::now() + budget;
    let telemetry = bounded_text(&root.join("data/system/vpnhide_lsposed_state"));
    let current_boot =
        bounded_text(&root.join("proc/sys/kernel/random/boot_id")).unwrap_or_default();
    let raw = telemetry.as_deref().unwrap_or("");
    let boot = raw
        .lines()
        .find_map(|l| l.strip_prefix("meta boot_id "))
        .unwrap_or("");
    let freshness = if raw.is_empty() {
        "missing"
    } else if boot.is_empty() || current_boot.trim().is_empty() {
        "unknown_boot"
    } else if boot == current_boot.trim() {
        "current_boot_not_proof_of_live_hooks"
    } else {
        "stale_boot"
    };
    let mut entries = Vec::new();
    let mut bytes = 0;
    let mut files = 0;
    for (index, source) in candidates(raw, root).iter().enumerate() {
        let result = if !include_files {
            Err(io::Error::other("not selected by export plan"))
        } else if files >= MAX_FILES || Instant::now() >= deadline {
            Err(io::Error::other("file count/time budget exhausted"))
        } else {
            let name = format!(
                "framework-{index}.{}",
                source.extension().and_then(|v| v.to_str()).unwrap_or("jar")
            );
            let destination = out.join(&name);
            let result = capture(root, source, &destination, TOTAL - bytes, deadline);
            if result.is_err() {
                let _ = fs::remove_file(destination);
            }
            result
        };
        match result {
            Ok(entry) => {
                bytes += entry["bytes"].as_u64().unwrap_or(0);
                files += 1;
                entries.push(entry);
            }
            Err(error) => entries.push(json!({"source":source.to_string_lossy().chars().take(256).map(|c| if c.is_control() { ' ' } else { c }).collect::<String>(),"skipped":error.to_string()})),
        }
    }
    let report: String = raw
        .chars()
        .take(32768)
        .map(|c| if c.is_control() && c != '\n' { ' ' } else { c })
        .collect();
    let manifest = json!({"schema":1,"files_requested":include_files,"java_report_available":raw.contains("meta java_report_schema 1"),"telemetry_freshness":freshness,"telemetry_error":telemetry.as_ref().err().map(ToString::to_string),"hook_report":report,"hook_report_truncated":raw.chars().count()>32768,"created_utc_ms":super::now_ms(),"rom":super::debug_snapshot::props(&["ro.build.fingerprint","ro.build.version.sdk","ro.build.version.security_patch","ro.product.cpu.abilist"]),"limits":{"total_bytes":TOTAL,"per_file_bytes":PER_FILE,"files":MAX_FILES,"seconds":25},"captured_bytes":bytes,"entries":entries,"limitations":"Origins are best-effort classloader evidence; fallbacks do not prove loaded class identity. No private app data or OAT/VDEX reconstruction. Missing telemetry may mean LSPosed never initialized; stale telemetry is not live evidence."});
    match fs::write(
        out.join("java-manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    ) {
        Ok(()) => (0, String::new()),
        Err(e) => (1, format!("Java manifest: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn fixture() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("system/framework")).unwrap();
        fs::create_dir_all(root.path().join("data/system")).unwrap();
        fs::create_dir_all(root.path().join("proc/sys/kernel/random")).unwrap();
        fs::write(
            root.path().join("proc/sys/kernel/random/boot_id"),
            "current",
        )
        .unwrap();
        root
    }
    fn jar(root: &Path, name: &str, dex: bool) -> PathBuf {
        let file = root.join("system/framework").join(name);
        // A bounded central directory suffices: the collector never extracts entries.
        let name = if dex {
            b"classes.dex".as_slice()
        } else {
            b"resource.txt".as_slice()
        };
        let mut central = vec![0; 46];
        central[..4].copy_from_slice(b"PK\x01\x02");
        central[28..30].copy_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend(name);
        let mut end = vec![0; 22];
        end[..4].copy_from_slice(b"PK\x05\x06");
        end[10..12].copy_from_slice(&1u16.to_le_bytes());
        end[12..16].copy_from_slice(&(central.len() as u32).to_le_bytes());
        central.extend(end);
        fs::write(&file, central).unwrap();
        file
    }
    #[test]
    fn captures_framework_with_hash_dex_and_stale_telemetry_without_private_files() {
        let root = fixture();
        let output = tempfile::tempdir().unwrap();
        jar(root.path(), "framework.jar", true);
        jar(root.path(), "services.jar", false);
        fs::write(
            root.path().join("data/system/vpnhide_lsposed_state"),
            "meta boot_id old\nmeta java_origin_0 /data/private/secret.apk\n",
        )
        .unwrap();
        assert_eq!(
            collect_from(root.path(), output.path(), Duration::from_secs(2), true).0,
            0
        );
        let m: serde_json::Value =
            serde_json::from_slice(&fs::read(output.path().join("java-manifest.json")).unwrap())
                .unwrap();
        assert_eq!(m["telemetry_freshness"], "stale_boot");
        let entries = m["entries"].as_array().unwrap();
        assert!(entries.iter().any(|e| e["dex"] == "dex_present"));
        assert!(
            entries
                .iter()
                .any(|e| e["dex"] == "dex_absent_no_odex_reconstruction")
        );
        assert!(
            entries
                .iter()
                .any(|e| e["skipped"] == "source outside framework allowlist")
        );
        assert!(
            entries
                .iter()
                .filter_map(|e| e["sha256"].as_str())
                .all(|hash| hash.len() == 64)
        );
    }
    #[test]
    fn baseline_never_copies_containers_and_heavy_collection_caps_file_count() {
        let root = fixture();
        let output = tempfile::tempdir().unwrap();
        let mut telemetry = String::from("meta boot_id current\nmeta java_report_schema 1\n");
        for i in 0..20 {
            jar(root.path(), &format!("oem{i}.jar"), true);
            telemetry.push_str(&format!(
                "meta java_origin_{i} /system/framework/oem{i}.jar\n"
            ));
        }
        fs::write(
            root.path().join("data/system/vpnhide_lsposed_state"),
            telemetry,
        )
        .unwrap();
        collect_from(root.path(), output.path(), Duration::from_secs(2), false);
        assert_eq!(fs::read_dir(output.path()).unwrap().count(), 1);
        let m: serde_json::Value =
            serde_json::from_slice(&fs::read(output.path().join("java-manifest.json")).unwrap())
                .unwrap();
        assert_eq!(m["captured_bytes"], 0);
        assert_eq!(
            m["telemetry_freshness"],
            "current_boot_not_proof_of_live_hooks"
        );
        collect_from(root.path(), output.path(), Duration::from_secs(2), true);
        assert_eq!(fs::read_dir(output.path()).unwrap().count(), MAX_FILES + 1);
    }

    #[test]
    fn rejects_traversal_symlink_escape_hardlink_and_special_files() {
        let root = fixture();
        let source = jar(root.path(), "framework.jar", true);
        assert!(!allowed(Path::new(
            "/system/framework/../../data/secret.jar"
        )));
        assert!(!allowed(Path::new("/data/app/example/base.apk")));
        fs::write(root.path().join("data/system/secret.jar"), "private").unwrap();
        symlink(
            root.path().join("data/system/secret.jar"),
            root.path().join("system/framework/evil.jar"),
        )
        .unwrap();
        assert!(open_source(root.path(), Path::new("/system/framework/evil.jar")).is_err());
        fs::hard_link(&source, root.path().join("system/framework/hard.jar")).unwrap();
        assert!(open_source(root.path(), Path::new("/system/framework/hard.jar")).is_err());
        fs::create_dir(root.path().join("system/framework/directory.jar")).unwrap();
        assert!(open_source(root.path(), Path::new("/system/framework/directory.jar")).is_err());
    }
    #[test]
    fn budget_timeout_and_missing_telemetry_preserve_manifest_without_partial_file() {
        let root = fixture();
        let output = tempfile::tempdir().unwrap();
        jar(root.path(), "framework.jar", true);
        assert!(
            capture(
                root.path(),
                Path::new("/system/framework/framework.jar"),
                &output.path().join("limited.jar"),
                1,
                Instant::now() + Duration::from_secs(1)
            )
            .is_err()
        );
        assert!(!output.path().join("limited.jar").exists());
        assert_eq!(
            collect_from(root.path(), output.path(), Duration::ZERO, true).0,
            0
        );
        let m: serde_json::Value =
            serde_json::from_slice(&fs::read(output.path().join("java-manifest.json")).unwrap())
                .unwrap();
        assert_eq!(m["telemetry_freshness"], "missing");
        assert_eq!(m["captured_bytes"], 0);
        assert!(
            m["entries"]
                .as_array()
                .unwrap()
                .iter()
                .all(|e| e["skipped"] == "file count/time budget exhausted")
        );
        assert_eq!(fs::read_dir(output.path()).unwrap().count(), 1);
    }
    #[test]
    fn oversized_and_malformed_containers_are_reported_and_origin_list_is_bounded() {
        let root = fixture();
        let output = tempfile::tempdir().unwrap();
        let source = root.path().join("system/framework/framework.jar");
        File::create(&source)
            .unwrap()
            .set_len(PER_FILE + 1)
            .unwrap();
        let mut broken = File::open(&source).unwrap();
        assert!(dex_status(&mut broken).is_err());
        assert!(
            capture(
                root.path(),
                Path::new("/system/framework/framework.jar"),
                &output.path().join("large.jar"),
                TOTAL,
                Instant::now() + Duration::from_secs(1)
            )
            .is_err()
        );
        let text = (0..1000)
            .map(|i| format!("meta java_origin_{i} /vendor/framework/{i}.jar\n"))
            .collect::<String>();
        assert!(candidates(&text, root.path()).len() <= 34);
    }
    #[test]
    fn opened_descriptor_cannot_be_redirected_by_parent_replacement() {
        let root = fixture();
        jar(root.path(), "framework.jar", true);
        let (mut file, _) =
            open_source(root.path(), Path::new("/system/framework/framework.jar")).unwrap();
        fs::rename(
            root.path().join("system/framework"),
            root.path().join("old"),
        )
        .unwrap();
        symlink(
            root.path().join("data/system"),
            root.path().join("system/framework"),
        )
        .unwrap();
        fs::write(root.path().join("data/system/framework.jar"), "private").unwrap();
        assert_eq!(dex_status(&mut file).unwrap(), "dex_present");
        assert!(open_source(root.path(), Path::new("/system/framework/framework.jar")).is_err());
    }
}
