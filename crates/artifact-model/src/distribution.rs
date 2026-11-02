//! Public distribution policy shared by the host CLI and update-feed generator.
//! Internal producer ZIPs are inputs, never separate user installations.
use crate::{archive::read_regular, invalid};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};
pub const PUBLIC_ASSETS: [&str; 3] = [
    "vpnhide.apk",
    "vpnhide-core.zip",
    "vpnhide-core.descriptor.json",
];
pub fn select(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for name in PUBLIC_ASSETS {
        let path = root.join(name);
        let meta = fs::symlink_metadata(&path)?;
        let text = path
            .to_str()
            .ok_or_else(|| invalid("non-UTF8 public asset path"))?;
        if !meta.is_file() || meta.len() == 0 || text.contains(['\n', '\r', '\0']) {
            return Err(invalid("missing or invalid public asset"));
        }
        paths.push(path);
    }
    Ok(paths)
}
pub fn github_output(root: &Path, output: &Path) -> io::Result<()> {
    // Validate the complete set before appending a single Actions output.
    let paths = select(root)?;
    let mut body = String::from("files<<VPNHIDE_PUBLIC_ASSETS\n");
    for path in paths {
        body.push_str(
            path.to_str()
                .ok_or_else(|| invalid("non-UTF8 public asset path"))?,
        );
        body.push('\n');
    }
    body.push_str("VPNHIDE_PUBLIC_ASSETS\n");
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(output)?
        .write_all(body.as_bytes())
}
pub fn update_feed(repo: &Path) -> io::Result<()> {
    let bytes = read_regular(&repo.join("VERSION"), 256)?;
    let version = std::str::from_utf8(&bytes).map_err(invalid)?.trim();
    let parts: Vec<_> = version.split('.').collect();
    if parts.len() != 3
        || parts
            .iter()
            .any(|s| s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(invalid("VERSION must be MAJOR.MINOR.PATCH"));
    }
    let values: Result<Vec<u64>, _> = parts.iter().map(|s| s.parse::<u64>()).collect();
    let n = values.map_err(invalid)?;
    let code = n[0]
        .checked_mul(10000)
        .and_then(|v| n[1].checked_mul(100).and_then(|m| v.checked_add(m)))
        .and_then(|v| v.checked_add(n[2]))
        .ok_or_else(|| invalid("version code overflow"))?;
    let feed = serde_json::json!({"version":format!("v{version}"),"versionCode":code,
        "zipUrl":format!("https://github.com/okhsunrog/vpnhide/releases/download/v{version}/vpnhide-core.zip"),
        "changelog":"https://raw.githubusercontent.com/okhsunrog/vpnhide/main/update-json/changelog.md"});
    let parent = repo.join("update-json");
    fs::create_dir_all(&parent)?;
    let mut staged = tempfile::NamedTempFile::new_in(&parent)?;
    staged.write_all(&serde_json::to_vec_pretty(&feed).map_err(invalid)?)?;
    staged.write_all(b"\n")?;
    staged.as_file().sync_all()?;
    staged
        .persist(parent.join("update-core.json"))
        .map_err(|e| e.error)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn assets() -> tempfile::TempDir {
        let r = tempfile::tempdir().unwrap();
        for name in PUBLIC_ASSETS {
            fs::write(r.path().join(name), b"producer-verified fixture").unwrap();
        }
        r
    }
    #[test]
    fn public_set_excludes_all_backend_private_and_base_artifacts() {
        let r = assets();
        for name in [
            "vpnhide-core-base.zip",
            "vpnhide-kmod-android14-6.1.zip",
            "vpnhide-kpm.zip",
            "vpnhide-zygisk.zip",
            "private-fixture.apk",
        ] {
            fs::write(r.path().join(name), b"internal").unwrap();
        }
        assert_eq!(
            select(r.path())
                .unwrap()
                .iter()
                .map(|p| p.file_name().unwrap().to_str().unwrap())
                .collect::<Vec<_>>(),
            PUBLIC_ASSETS
        );
    }
    #[test]
    fn missing_empty_symlink_and_newline_paths_refuse_before_output() {
        let r = assets();
        for name in PUBLIC_ASSETS {
            let p = r.path().join(name);
            fs::remove_file(&p).unwrap();
            assert!(select(r.path()).is_err());
            fs::write(&p, []).unwrap();
            assert!(select(r.path()).is_err());
            fs::write(&p, b"restored").unwrap();
        }
        #[cfg(unix)]
        {
            let p = r.path().join("vpnhide-core.zip");
            fs::remove_file(&p).unwrap();
            std::os::unix::fs::symlink("vpnhide.apk", p).unwrap();
            assert!(github_output(r.path(), &r.path().join("out")).is_err());
            assert!(!r.path().join("out").exists());
        }
        let newline = r.path().join("bad\npath");
        fs::create_dir(&newline).unwrap();
        for name in PUBLIC_ASSETS {
            fs::write(newline.join(name), b"data").unwrap();
        }
        assert!(select(&newline).is_err());
    }
    #[test]
    fn exact_actions_allowlist_appends_without_overwriting_existing_outputs() {
        let r = assets();
        let o = r.path().join("output");
        fs::write(&o, b"previous=yes\n").unwrap();
        github_output(r.path(), &o).unwrap();
        let expected = format!(
            "previous=yes\nfiles<<VPNHIDE_PUBLIC_ASSETS\n{}\nVPNHIDE_PUBLIC_ASSETS\n",
            PUBLIC_ASSETS
                .iter()
                .map(|n| r.path().join(n).display().to_string())
                .collect::<Vec<_>>()
                .join("\n")
        );
        assert_eq!(fs::read_to_string(o).unwrap(), expected);
    }
    #[test]
    fn only_core_feed_changes_and_zero_padded_version_uses_decimal() {
        let r = tempfile::tempdir().unwrap();
        let d = r.path().join("update-json");
        fs::create_dir(&d).unwrap();
        for n in [
            "update-kmod-android14-6.1.json",
            "update-kpm.json",
            "update-zygisk.json",
        ] {
            fs::write(d.join(n), b"frozen legacy feed").unwrap();
        }
        fs::write(r.path().join("VERSION"), b"1.08.0\n").unwrap();
        update_feed(r.path()).unwrap();
        let feed: serde_json::Value =
            serde_json::from_slice(&fs::read(d.join("update-core.json")).unwrap()).unwrap();
        assert_eq!(feed["version"], "v1.08.0");
        assert_eq!(feed["versionCode"], 10800);
        assert!(
            feed["zipUrl"]
                .as_str()
                .unwrap()
                .ends_with("/v1.08.0/vpnhide-core.zip")
        );
        assert!(PUBLIC_ASSETS.contains(&"vpnhide-core.zip"));
        for n in [
            "update-kmod-android14-6.1.json",
            "update-kpm.json",
            "update-zygisk.json",
        ] {
            assert_eq!(fs::read(d.join(n)).unwrap(), b"frozen legacy feed");
        }
        assert_eq!(fs::read_dir(d).unwrap().count(), 4);
    }
    #[test]
    fn invalid_or_overflowing_version_creates_no_feeds() {
        let r = tempfile::tempdir().unwrap();
        for value in [
            "not-a-version",
            "1.2",
            "1.2.3.4",
            "-1.2.3",
            "1. 2.3",
            "18446744073709551615.0.0",
        ] {
            fs::write(r.path().join("VERSION"), value).unwrap();
            assert!(update_feed(r.path()).is_err());
            assert!(!r.path().join("update-json").exists());
        }
    }
}
