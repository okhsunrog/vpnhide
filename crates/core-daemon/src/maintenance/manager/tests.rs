use super::*;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    sync::OnceLock,
};
fn binary() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
        let root = tempfile::tempdir().unwrap().keep();
        let output = root.join("fixture-manager");
        let status = Command::new("rustc")
            .arg("--edition=2024")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/maintenance-manager.rs"
            ))
            .arg("-o")
            .arg(&output)
            .status()
            .unwrap();
        assert!(status.success());
        output
    })
}
pub(crate) fn install(root: &Path) -> Manager {
    fs::create_dir_all(root.join("data/adb/modules/vpnhide_core")).unwrap();
    fs::write(
        root.join("data/adb/modules/vpnhide_core/module.prop"),
        "id=vpnhide_core\n",
    )
    .unwrap();
    let path = root.join("data/adb/ksud");
    fs::copy(binary(), &path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    Manager::discover_owned(root, unsafe { libc::geteuid() }).unwrap()
}
#[test]
fn real_manager_reads_retained_package_fd_and_exit_zero_is_only_dispatch_evidence() {
    let root = tempfile::tempdir().unwrap();
    let manager = install(root.path());
    fs::write(root.path().join("data/adb/mode"), "none").unwrap();
    let package = root.path().join("private.zip");
    fs::write(&package, b"exact authenticated private bytes").unwrap();
    let file = File::open(&package).unwrap();
    fs::remove_file(&package).unwrap();
    let result = manager.stage(&file).unwrap();
    assert!(result.output.status.success());
    assert!(!result.timed_out);
    assert_eq!(
        fs::read(root.path().join("data/adb/observed-package")).unwrap(),
        b"exact authenticated private bytes"
    );
    assert!(
        !root
            .path()
            .join("data/adb/modules_update/vpnhide_core")
            .exists()
    );
    assert_eq!(
        fs::read_to_string(root.path().join("data/adb/dispatch-count")).unwrap(),
        "1"
    );
}
#[test]
fn swapped_executable_and_new_custom_installer_invalidate_preparation() {
    let root = tempfile::tempdir().unwrap();
    let manager = install(root.path());
    manager.require_unchanged().unwrap();
    let meta = root.path().join("data/adb/modules/custom");
    fs::create_dir(&meta).unwrap();
    fs::write(meta.join("module.prop"), "id=custom\nmetamodule=true\n").unwrap();
    fs::write(meta.join("metainstall.sh"), "untrusted custom installer").unwrap();
    assert!(manager.require_unchanged().is_err());
    fs::remove_dir_all(meta).unwrap();
    manager.require_unchanged().unwrap();
    let path = root.path().join("data/adb/ksud");
    fs::remove_file(&path).unwrap();
    fs::copy(binary(), path).unwrap();
    assert!(manager.require_unchanged().is_err());
}
#[test]
fn ambiguous_manager_and_symlink_escape_are_refused() {
    let root = tempfile::tempdir().unwrap();
    let _ = install(root.path());
    fs::copy(binary(), root.path().join("data/adb/apd")).unwrap();
    assert!(Manager::discover_owned(root.path(), unsafe { libc::geteuid() }).is_err());
    fs::remove_file(root.path().join("data/adb/apd")).unwrap();
    symlink(binary(), root.path().join("data/adb/apd")).unwrap();
    assert!(Manager::discover_owned(root.path(), unsafe { libc::geteuid() }).is_err());
}
#[test]
fn properties_do_not_guess_escaped_or_duplicate_installer_semantics() {
    for raw in [
        "metamodule:true",
        "metamodule true",
        "metamodule=tr\\ue",
        "metamodule=true\nmetamodule=false",
    ] {
        assert!(simple_properties(raw).is_err(), "{raw}");
    }
    assert_eq!(
        simple_properties("# comment\nmetamodule = true\n").unwrap()["metamodule"],
        "true"
    );
}

#[test]
fn platform_data_parent_is_distinct_from_privileged_adb_boundary() {
    let root = tempfile::tempdir().unwrap();
    let manager = install(root.path());
    fs::set_permissions(root.path().join("data"), fs::Permissions::from_mode(0o771)).unwrap();
    manager.require_unchanged().unwrap();
    fs::set_permissions(
        root.path().join("data/adb"),
        fs::Permissions::from_mode(0o771),
    )
    .unwrap();
    assert!(manager.require_unchanged().is_err());
}

#[test]
fn replacing_discovery_symlink_cannot_reuse_the_old_pinned_target() {
    let root = tempfile::tempdir().unwrap();
    let _ = install(root.path());
    let original = root.path().join("data/adb/ksud");
    let target = root.path().join("data/adb/manager-real");
    fs::rename(&original, &target).unwrap();
    symlink("manager-real", &original).unwrap();
    let manager = Manager::discover_owned(root.path(), unsafe { libc::geteuid() }).unwrap();
    fs::copy(binary(), root.path().join("data/adb/manager-new")).unwrap();
    fs::remove_file(&original).unwrap();
    symlink("manager-new", &original).unwrap();
    assert!(manager.require_unchanged().is_err());
}
