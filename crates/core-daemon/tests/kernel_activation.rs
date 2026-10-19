//! Host integration: core process start -> once-per-boot selection and load -> RPC edit
//! -> delivery to the selected backend only. The child alone receives PATH overrides;
//! no process-global test env mutation. Fake insmod and a regular control file do not
//! establish device/kernel support.
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use vpnhide_core_daemon::{CoreClient, config_file};
use vpnhide_core_rpc::{
    BaseRef, CanonicalConfig, ConfigEdit, ConfigPhase, NativeBackend, NativeBootOutcome,
    OperationId, PhaseOutcome, SelectionMode,
};

struct CoreProcess(Child);

impl Drop for CoreProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Case {
    initial_owner: Option<u32>,
    loaded_owner: u32,
    kmod_module: bool,
    kpm: Option<&'static str>,
    expected: PhaseOutcome,
    expect_insmod: bool,
}

async fn exercise(case: Case, check: impl FnOnce(&NativeBootOutcome)) {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("core.sock");
    let ctl = dir.path().join("ctl");
    let config_path = dir.path().join("config.json");
    let args_path = dir.path().join("load-args");
    let module = dir.path().join("modules/vpnhide_kmod");
    if case.kmod_module {
        fs::create_dir_all(&module).unwrap();
        fs::write(module.join("module.prop"), "id=vpnhide_kmod\n").unwrap();
        fs::write(module.join("vpnhide_kmod.ko"), "fake artifact").unwrap();
    } else {
        fs::create_dir_all(dir.path().join("modules")).unwrap();
    }
    if let Some(owner) = case.initial_owner {
        fs::write(
            &ctl,
            format!("vpnhide 1 status\nbackend 0x{owner:x}\nkver 0x0\nhooks 0x0\nerror 0x0\n"),
        )
        .unwrap();
    }
    // The load-time feature comes from the durable config core starts with.
    let mut initial = CanonicalConfig::default();
    initial
        .settings
        .optional_features
        .insert("filesystem_iface_paths".into());
    config_file::write_atomic(&config_path, &initial).unwrap();
    let insmod = dir.path().join("insmod");
    fs::write(&insmod, "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$VPNHIDE_TEST_LOAD_ARGS\"\nprintf 'vpnhide 1 status\\nbackend 0x%s\\nkver 0x0\\nhooks 0x0\\nerror 0x0\\n' \"$VPNHIDE_TEST_OWNER\" > \"$VPNHIDE_KMOD_CTL\"\n").unwrap();
    fs::set_permissions(&insmod, fs::Permissions::from_mode(0o700)).unwrap();
    if case.kpm.is_some() {
        let kpatch = dir.path().join("kpatch");
        fs::write(&kpatch, "#!/bin/sh\nif [ \"$1\" = hello ]; then printf 'hello\\n'; exit 0; fi\nif [ \"$VPNHIDE_TEST_KPM\" = error ]; then exit 1; fi\nprintf '%s\\n' \"$VPNHIDE_TEST_KPM\"\n").unwrap();
        fs::set_permissions(&kpatch, fs::Permissions::from_mode(0o700)).unwrap();
    }

    let mut child = CoreProcess(
        Command::new(env!("CARGO_BIN_EXE_vpnhide-core"))
            .arg(&socket)
            .env("PATH", dir.path())
            .env("VPNHIDE_CONFIG_PATH", &config_path)
            .env("VPNHIDE_CORE_OWNER_DIR", dir.path().join("owner"))
            .env("VPNHIDE_KMOD_CTL", &ctl)
            .env("VPNHIDE_KERNEL_MODULES", dir.path().join("modules"))
            .env_remove("VPNHIDE_KPM_LOCK")
            .env_remove("VPNHIDE_ZYGISK_MODULE")
            .env_remove("VPNHIDE_BOOT_APPLY")
            .env("VPNHIDE_TEST_LOAD_ARGS", &args_path)
            .env("VPNHIDE_TEST_OWNER", format!("{:x}", case.loaded_owner))
            .env("VPNHIDE_TEST_KPM", case.kpm.unwrap_or_default())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while !socket.exists() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "core exited before binding"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("core bound its socket");
    // Selection and loading happened at startup, before any edit.
    if case.expect_insmod {
        assert_eq!(
            fs::read_to_string(&args_path).unwrap(),
            format!(
                "{}\nfilesystem_hiding=1\n",
                module.join("vpnhide_kmod.ko").display()
            )
        );
    } else {
        assert!(!args_path.exists(), "core must not insmod");
    }
    let client = CoreClient::connect_and_mirror(&socket);
    client.hello().await.unwrap();
    let mut candidate = initial.clone();
    candidate.debug = true;
    client
        .submit_edit(
            OperationId(1),
            BaseRef { revision: 1 },
            ConfigEdit::Replace {
                config: candidate.clone(),
            },
        )
        .await
        .unwrap()
        .unwrap();
    let record = client.get_operation(OperationId(1)).await.unwrap().unwrap();
    assert_eq!(record.settled, Some(case.expected));
    assert_eq!(record.phases[0].phase, ConfigPhase::Persist);
    assert_eq!(record.phases[0].outcome, PhaseOutcome::Confirmed);
    assert_eq!(record.phases[1].phase, ConfigPhase::Native);
    assert_eq!(record.phases[1].outcome, case.expected);
    assert_eq!(config_file::read(&config_path).unwrap(), Some(candidate));
    // The edit never re-selected: still at most the one boot-time insmod.
    let published = client.current().await.unwrap();
    let boot = published.snapshot.backend.boot.expect("typed boot status");
    assert!(!boot.boot_id.is_empty());
    check(&boot.outcome);
    if !ctl.exists() {
        return;
    }
    let content = fs::read_to_string(&ctl).unwrap();
    if case.expected == PhaseOutcome::Confirmed {
        assert!(
            content.starts_with("vpnhide 2 config\n"),
            "core delivered policy: {content}"
        );
    } else {
        assert!(
            content.starts_with("vpnhide 1 status\n"),
            "refusal must not overwrite the node"
        );
    }
}

#[tokio::test]
async fn kmod_is_loaded_at_boot_and_receives_policy() {
    let case = Case {
        initial_owner: None,
        loaded_owner: 0,
        kmod_module: true,
        kpm: None,
        expected: PhaseOutcome::Confirmed,
        expect_insmod: true,
    };
    exercise(case, |outcome| {
        assert_eq!(
            *outcome,
            NativeBootOutcome::Selected {
                backend: NativeBackend::Kmod,
                mode: SelectionMode::Loaded,
                filesystem_hiding: Some(true),
            }
        );
    })
    .await;
}

#[tokio::test]
async fn builtin_alone_is_adopted_and_receives_policy() {
    let case = Case {
        initial_owner: Some(4),
        loaded_owner: 0,
        kmod_module: false,
        kpm: None,
        expected: PhaseOutcome::Confirmed,
        expect_insmod: false,
    };
    exercise(case, |outcome| {
        assert!(matches!(
            outcome,
            NativeBootOutcome::Selected {
                backend: NativeBackend::Builtin,
                mode: SelectionMode::Adopted,
                ..
            }
        ));
    })
    .await;
}

#[tokio::test]
async fn builtin_plus_kmod_module_is_a_conflict_and_nothing_is_configured() {
    let case = Case {
        initial_owner: Some(4),
        loaded_owner: 0,
        kmod_module: true,
        kpm: None,
        expected: PhaseOutcome::FailedKnown,
        expect_insmod: false,
    };
    exercise(case, |outcome| {
        assert_eq!(
            *outcome,
            NativeBootOutcome::Conflict {
                installed: vec![NativeBackend::Kmod],
                live: vec![NativeBackend::Builtin],
                staged: vec![],
            }
        );
    })
    .await;
}

#[tokio::test]
async fn wrong_post_load_identity_preserves_saved_intent_and_refuses_delivery() {
    let case = Case {
        initial_owner: None,
        loaded_owner: 4,
        kmod_module: true,
        kpm: None,
        expected: PhaseOutcome::FailedKnown,
        expect_insmod: true,
    };
    exercise(case, |outcome| {
        assert!(matches!(
            outcome,
            NativeBootOutcome::LoadFailed {
                backend: NativeBackend::Kmod,
                ..
            }
        ));
    })
    .await;
}

#[tokio::test]
async fn a_live_kpm_next_to_a_kmod_module_is_a_conflict() {
    let case = Case {
        initial_owner: None,
        loaded_owner: 0,
        kmod_module: true,
        kpm: Some("vpnhide"),
        expected: PhaseOutcome::FailedKnown,
        expect_insmod: false,
    };
    exercise(case, |outcome| {
        assert_eq!(
            *outcome,
            NativeBootOutcome::Conflict {
                installed: vec![NativeBackend::Kmod],
                live: vec![NativeBackend::Kpm],
                staged: vec![],
            }
        );
    })
    .await;
}

#[tokio::test]
async fn unqueryable_kernelpatch_without_kpm_candidate_does_not_block_kmod() {
    let case = Case {
        initial_owner: None,
        loaded_owner: 0,
        kmod_module: true,
        kpm: Some("error"),
        expected: PhaseOutcome::Confirmed,
        expect_insmod: true,
    };
    exercise(case, |outcome| {
        assert!(matches!(
            outcome,
            NativeBootOutcome::Selected {
                backend: NativeBackend::Kmod,
                ..
            }
        ));
    })
    .await;
}
