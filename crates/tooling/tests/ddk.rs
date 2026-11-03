use std::{fs, path::Path, process::Command};

fn modpost_make(root: &Path, original: &str) -> std::process::Output {
    let module = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../kmod/Makefile");
    fs::write(
        root.join("Makefile"),
        format!(
            "KERNELRELEASE := fixture\nMODPOST = {original}\n\
             include {}\n\
             CONFIG_MODVERSIONS := y\n\
             MODPOST += -e -i Module.symvers\n\
             captured:\n\t@printf '%s\\n' $(MODPOST) > $@\n",
            module.display()
        ),
    )
    .unwrap();
    Command::new("make")
        .current_dir(root)
        .args(["--no-print-directory", "VPNHIDE_MODPOST=/private/modpost"])
        .output()
        .unwrap()
}

#[test]
fn replacement_checker_preserves_kernel_flags_late_configuration_and_target() {
    let root = tempfile::tempdir().unwrap();
    let output = modpost_make(
        root.path(),
        "scripts/mod/modpost $(if $(CONFIG_MODVERSIONS),-m) -o $@",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(root.path().join("captured")).unwrap(),
        "/private/modpost\n-m\n-o\ncaptured\n-e\n-i\nModule.symvers\n"
    );
}

#[test]
fn unknown_kernel_checker_command_refuses_instead_of_losing_arguments() {
    let root = tempfile::tempdir().unwrap();
    let output = modpost_make(root.path(), "another/checker -m -o $@");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Unsupported Kbuild MODPOST command"));
    assert!(!root.path().join("captured").exists());
}

#[test]
fn newer_kbuild_executable_variants_preserve_separate_flags_and_reject_unknown_forms() {
    let module = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../kmod/Makefile");
    for (before, accepted) in [
        ("", true),
        ("MODPOST = scripts/mod/modpost\n", true),
        ("MODPOST = $(objtree)/scripts/mod/modpost\n", true),
        ("MODPOST = another/checker -m\n", false),
        ("MODPOST = scripts/mod/modpost -w\n", false),
    ] {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("Makefile"),
            format!(
                "KERNELRELEASE := fixture\n{before}\
                 modpost-args = $(if $(CONFIG_MODVERSIONS),-m) -o $@\n\
                 include {}\n\
                 MODPOST = scripts/mod/modpost\n\
                 CONFIG_MODVERSIONS := y\n\
                 modpost-args += -e -i Module.symvers\n\
                 captured:\n\t@printf '%s\\n' $(MODPOST) $(modpost-args) > $@\n",
                module.display()
            ),
        )
        .unwrap();
        let output = Command::new("make")
            .current_dir(root.path())
            .args(["--no-print-directory", "VPNHIDE_MODPOST=/private/modpost"])
            .output()
            .unwrap();
        assert_eq!(output.status.success(), accepted, "{before}: {output:?}");
        if accepted {
            assert_eq!(
                fs::read_to_string(root.path().join("captured")).unwrap(),
                "/private/modpost\n-m\n-o\ncaptured\n-e\n-i\nModule.symvers\n"
            );
        } else {
            assert!(!root.path().join("captured").exists());
        }
    }
}
