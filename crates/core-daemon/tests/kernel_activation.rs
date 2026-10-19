//! Host integration: real RPC -> Native phase -> shared loader -> delivery.
//! The child alone receives PATH overrides; no process-global test env mutation.
//! Fake insmod and a regular control file do not establish device/kernel support.
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use vpnhide_core_daemon::{CoreClient, config_file};
use vpnhide_core_rpc::{
    BaseRef, CanonicalConfig, ConfigEdit, ConfigPhase, NativeBackend, OperationId, PhaseOutcome,
};

struct CoreProcess(Child);

impl Drop for CoreProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn exercise(
    initial_owner: Option<u32>,
    loaded_owner: u32,
    expected: PhaseOutcome,
    kpm: Option<&str>,
) {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("core.sock");
    let ctl = dir.path().join("ctl");
    let config_path = dir.path().join("config.json");
    let args_path = dir.path().join("load-args");
    let module = dir.path().join("modules/vpnhide_kmod");
    fs::create_dir_all(&module).unwrap();
    fs::write(module.join("module.prop"), "id=vpnhide_kmod\n").unwrap();
    fs::write(module.join("vpnhide_kmod.ko"), "fake artifact").unwrap();
    if let Some(owner) = initial_owner {
        fs::write(
            &ctl,
            format!("vpnhide 1 status\nbackend 0x{owner:x}\nkver 0x0\nhooks 0x0\nerror 0x0\n"),
        )
        .unwrap();
    }
    let insmod = dir.path().join("insmod");
    fs::write(&insmod, "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$VPNHIDE_TEST_LOAD_ARGS\"\nprintf 'vpnhide 1 status\\nbackend 0x%s\\nkver 0x0\\nhooks 0x0\\nerror 0x0\\n' \"$VPNHIDE_TEST_OWNER\" > \"$VPNHIDE_KMOD_CTL\"\n").unwrap();
    fs::set_permissions(&insmod, fs::Permissions::from_mode(0o700)).unwrap();
    if let Some(reply) = kpm {
        let kpatch = dir.path().join("kpatch");
        fs::write(&kpatch, "#!/bin/sh\nif [ \"$1\" = hello ]; then printf 'hello\\n'; exit 0; fi\nif [ \"$VPNHIDE_TEST_KPM\" = error ]; then exit 1; fi\nprintf '%s\\n' \"$VPNHIDE_TEST_KPM\"\n").unwrap();
        fs::set_permissions(&kpatch, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(!reply.is_empty());
    }

    let mut child = CoreProcess(
        Command::new(env!("CARGO_BIN_EXE_vpnhide-core"))
            .arg(&socket)
            .env("PATH", dir.path())
            .env("VPNHIDE_CONFIG_PATH", &config_path)
            .env("VPNHIDE_KMOD_CTL", &ctl)
            .env("VPNHIDE_KERNEL_MODULES", dir.path().join("modules"))
            .env_remove("VPNHIDE_KPM_LOCK")
            .env("VPNHIDE_TEST_LOAD_ARGS", &args_path)
            .env("VPNHIDE_TEST_OWNER", format!("{loaded_owner:x}"))
            .env("VPNHIDE_TEST_KPM", kpm.unwrap_or_default())
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
    let client = CoreClient::connect_and_mirror(&socket);
    client.hello().await.unwrap();
    let mut candidate = CanonicalConfig::default();
    candidate
        .settings
        .optional_features
        .insert("filesystem_iface_paths".into());
    client
        .submit_edit(
            OperationId(1),
            BaseRef { revision: 0 },
            ConfigEdit::Replace {
                config: candidate.clone(),
            },
        )
        .await
        .unwrap()
        .unwrap();
    let record = client.get_operation(OperationId(1)).await.unwrap().unwrap();
    assert_eq!(record.settled, Some(expected));
    assert_eq!(record.phases[0].phase, ConfigPhase::Persist);
    assert_eq!(record.phases[0].outcome, PhaseOutcome::Confirmed);
    assert_eq!(record.phases[1].phase, ConfigPhase::Native);
    assert_eq!(record.phases[1].outcome, expected);
    assert_eq!(config_file::read(&config_path).unwrap(), Some(candidate));
    let published = client.current().await.unwrap();
    assert_eq!(published.snapshot.config_revision, 1);
    if initial_owner.is_none() && loaded_owner == 4 && kpm.is_none() {
        assert_eq!(
            published.snapshot.backend.active,
            Some(NativeBackend::Builtin)
        );
    }
    if kpm.is_some() {
        assert!(
            !args_path.exists(),
            "KPM presence or uncertainty must prevent insmod"
        );
        assert!(!ctl.exists());
        return;
    }
    if initial_owner.is_none() {
        let args = fs::read_to_string(&args_path).unwrap();
        assert_eq!(
            args,
            format!(
                "{}\nfilesystem_hiding=1\n",
                module.join("vpnhide_kmod.ko").display()
            )
        );
    } else {
        assert!(!args_path.exists(), "live owner must prevent insmod");
    }
    let content = fs::read_to_string(&ctl).unwrap();
    if expected == PhaseOutcome::Confirmed {
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
async fn kmod_is_loaded_directly_and_receives_policy() {
    exercise(None, 0, PhaseOutcome::Confirmed, None).await;
}

#[tokio::test]
async fn builtin_without_bridge_prevents_kmod_loading() {
    exercise(Some(4), 0, PhaseOutcome::Confirmed, None).await;
}

#[tokio::test]
async fn wrong_post_load_identity_preserves_saved_intent_and_refuses_delivery() {
    exercise(None, 4, PhaseOutcome::FailedKnown, None).await;
}

#[tokio::test]
async fn active_kpm_prevents_kmod_loading() {
    exercise(None, 0, PhaseOutcome::FailedKnown, Some("vpnhide")).await;
}

#[tokio::test]
async fn failed_kpm_enumeration_prevents_kmod_loading() {
    exercise(None, 0, PhaseOutcome::FailedKnown, Some("error")).await;
}
