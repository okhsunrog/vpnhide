//! Policy publication through the actual core process/RPC. No APK, LSPosed,
//! Zygisk injection or companion participates in these host scenarios.
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use vpnhide_core_daemon::{CoreClient, config_file};
use vpnhide_core_rpc::{
    AppConfig, BaseRef, CanonicalConfig, ConfigEdit, NativeBackend, NativeRole, OperationId,
    PhaseOutcome,
};

const OLD: &str = "untouched policy";
const PM: &str = r#"#!/bin/sh
if [ "$VH_CASE" = kernel-race ]; then
  printf 'vpnhide 1 status\nbackend 0x4\nkver 0x60100\nhooks 0x3ff\nerror 0x0\n' > "$VPNHIDE_KMOD_CTL"
fi
[ "$VH_CASE" = kpm-race ] && : > "$VH_ROOT/kpm-now"
if [ "$VH_CASE" = changed-recipient ]; then
  printf 'id=foreign\nvpnhideControl=2\n' > "$VPNHIDE_ZYGISK_MODULE/module.prop"
fi
if [ "$2" = users ]; then
  printf 'Users:\n UserInfo{0:Owner:13} running\n'
else
  printf 'package:dev.okhsunrog.vpnhide uid:10123\npackage:com.example uid:10234\n'
fi
"#;

const KPATCH: &str = r#"#!/bin/sh
[ "$1" = hello ] && { printf 'hello\n'; exit 0; }
if [ "$2" = list ]; then
  [ "$VH_CASE" = unknown-kpm ] && exit 1
  if [ "$VH_CASE" = kpm-race ] && [ ! -f "$VH_ROOT/kpm-now" ]; then exit 0; fi
  printf 'vpnhide\n'
  exit 0
fi
case "$4" in
  "vpnhide 1 status")
    printf 'vpnhide 1 status\nbackend 0x1\nkver 0x60100\nhooks 0x3ff\nerror 0x0\n'
    exit 64 ;;
  "vpnhide 2 config"*)
    printf 'configured\n' > "$VH_ROOT/kpm-dispatch"
    exit 0 ;;
esac
exit 1
"#;

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn executable(path: &Path, script: &str) {
    fs::write(path, script).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn artifact(path: &Path, class: u8, machine: u16) {
    let mut bytes = vec![0; if class == 1 { 52 } else { 64 }];
    bytes[..4].copy_from_slice(b"\x7fELF");
    bytes[4..7].copy_from_slice(&[class, 1, 1]);
    bytes[16..18].copy_from_slice(&3u16.to_le_bytes());
    bytes[18..20].copy_from_slice(&machine.to_le_bytes());
    fs::write(path, bytes).unwrap();
}

async fn exercise(case: &str) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let modules = root.join("modules");
    let module = modules.join("vpnhide_zygisk");
    fs::create_dir_all(module.join("zygisk")).unwrap();
    fs::write(
        module.join("module.prop"),
        include_str!("../../../zygisk/module/module.prop"),
    )
    .unwrap();
    if case == "arm32" {
        artifact(&module.join("zygisk/armeabi-v7a.so"), 1, 40);
    } else {
        artifact(&module.join("zygisk/arm64-v8a.so"), 2, 183);
    }
    let policy = module.join("targets.txt");
    if case != "first-publish" {
        fs::write(&policy, OLD).unwrap();
    }
    match case {
        "disable" | "remove" => fs::write(module.join(case), "").unwrap(),
        "foreign" => {
            fs::write(module.join("module.prop"), "id=foreign\nvpnhideControl=2\n").unwrap()
        }
        "legacy-profile" => fs::write(module.join("module.prop"), "id=vpnhide_zygisk\n").unwrap(),
        _ => (),
    }
    let ctl = root.join("ctl");
    if case == "kernel" {
        fs::write(
            &ctl,
            "vpnhide 1 status\nbackend 0x4\nkver 0x60100\nhooks 0x3ff\nerror 0x0\n",
        )
        .unwrap();
    } else if case == "invalid-kernel" {
        fs::write(&ctl, "unknown node").unwrap();
    }
    if case == "kmod-fails" {
        let kmod = modules.join("vpnhide_kmod");
        fs::create_dir_all(&kmod).unwrap();
        fs::write(kmod.join("module.prop"), "id=vpnhide_kmod\n").unwrap();
        fs::write(kmod.join("vpnhide_kmod.ko"), "artifact").unwrap();
    }
    if case == "kpm-without-runtime" {
        let kpm = modules.join("vpnhide_kpm");
        fs::create_dir_all(&kpm).unwrap();
        fs::write(kpm.join("module.prop"), "id=vpnhide_kpm\n").unwrap();
        fs::write(kpm.join("vpnhide.kpm"), "artifact").unwrap();
    }
    if matches!(case, "kpm" | "unmanaged-kpm" | "unknown-kpm" | "kpm-race") {
        executable(&root.join("kpatch"), KPATCH);
    }
    executable(&root.join("pm"), PM);
    executable(
        &root.join("insmod"),
        "#!/bin/sh\nprintf 'attempted\\n' > \"$VH_ROOT/insmod-dispatch\"\nexit 1\n",
    );
    let socket = root.join("core.sock");
    let canonical = root.join("canonical.json");
    let mut command = Command::new(env!("CARGO_BIN_EXE_vpnhide-core"));
    command
        .arg(&socket)
        .env("PATH", root)
        .env("VH_ROOT", root)
        .env("VH_CASE", case)
        .env("VPNHIDE_CONFIG_PATH", &canonical)
        .env("VPNHIDE_KMOD_CTL", &ctl)
        .env("VPNHIDE_KERNEL_MODULES", &modules)
        .env_remove("VPNHIDE_KPM_LOCK")
        .env_remove("VPNHIDE_ZYGISK_MODULE")
        .stdout(Stdio::null())
        .stderr(fs::File::create(root.join("core.log")).unwrap());
    if case != "opt-out" {
        let selected_path = if case == "symlink-module" {
            let link = root.join("linked-module");
            symlink(&module, &link).unwrap();
            link
        } else {
            module.clone()
        };
        command.env("VPNHIDE_ZYGISK_MODULE", selected_path);
    }
    if case == "kpm" {
        command.env("VPNHIDE_KPM_LOCK", root.join("kpm.lock"));
    }
    let mut process = Process(command.spawn().unwrap());
    tokio::time::timeout(Duration::from_secs(5), async {
        while !socket.exists() {
            assert!(
                process.0.try_wait().unwrap().is_none(),
                "{case}: core exited"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("core bound its socket");
    let client = CoreClient::connect_and_mirror(&socket);
    client.hello().await.unwrap();
    let mut candidate = CanonicalConfig::default();
    candidate.apps.insert(
        "com.example".into(),
        AppConfig {
            native: NativeRole {
                enabled: true,
                kernel: Some(vec![]),
                zygisk: Some(vec!["zygisk_getifaddrs".into()]),
            },
            ..Default::default()
        },
    );
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
    let publishes = matches!(
        case,
        "publish" | "arm32" | "first-publish" | "kpm-without-runtime"
    );
    let success = publishes || matches!(case, "kernel" | "kpm");
    assert_eq!(
        record.settled,
        Some(if success {
            PhaseOutcome::Confirmed
        } else {
            PhaseOutcome::FailedKnown
        }),
        "{case}: {}",
        fs::read_to_string(root.join("core.log")).unwrap()
    );
    assert_eq!(
        config_file::read(&canonical).unwrap(),
        Some(candidate),
        "{case}: persisted intent"
    );
    let published = client.current().await.unwrap();
    let selected = if publishes || matches!(case, "changed-recipient" | "kernel-race" | "kpm-race")
    {
        Some(NativeBackend::Zygisk)
    } else if case == "kernel" {
        Some(NativeBackend::Builtin)
    } else if case == "kpm" {
        Some(NativeBackend::Kpm)
    } else {
        None
    };
    assert_eq!(
        published.snapshot.backend.selected, selected,
        "{case}: selected is distinct from active"
    );
    assert_eq!(published.snapshot.config_revision, 1);
    if publishes {
        assert_eq!(
            published.snapshot.backend.active, None,
            "{case}: file publication does not prove injection"
        );
        let wire = fs::read_to_string(&policy).unwrap();
        let parsed = vpnhide_protocol::parse_config(wire.as_bytes()).unwrap();
        assert_eq!(
            parsed.targets,
            vec![vpnhide_protocol::Target {
                uid: 10234,
                hookmask: 1 << 19
            }],
            "{case}"
        );
        assert_eq!(
            fs::metadata(&policy).unwrap().permissions().mode() & 0o777,
            0o644
        );
        // Removing the last target must replace the old runtime policy with an empty snapshot.
        client
            .submit_edit(
                OperationId(2),
                BaseRef { revision: 1 },
                ConfigEdit::Replace {
                    config: CanonicalConfig::default(),
                },
            )
            .await
            .unwrap()
            .unwrap();
        let wire = fs::read_to_string(&policy).unwrap();
        assert!(
            vpnhide_protocol::parse_config(wire.as_bytes())
                .unwrap()
                .targets
                .is_empty()
        );
        assert_eq!(client.current().await.unwrap().snapshot.config_revision, 2);
    } else {
        assert_eq!(
            fs::read_to_string(&policy).unwrap(),
            OLD,
            "{case}: no lower-priority or refused write"
        );
    }
    assert_eq!(
        root.join("insmod-dispatch").exists(),
        case == "kmod-fails",
        "{case}"
    );
    assert_eq!(root.join("kpm-dispatch").exists(), case == "kpm", "{case}");
}

#[tokio::test]
async fn zygisk_publication_selection_and_refusals_over_rpc() {
    for case in [
        "publish",
        "arm32",
        "first-publish",
        "disable",
        "remove",
        "foreign",
        "legacy-profile",
        "changed-recipient",
        "kernel-race",
        "kpm-race",
        "kernel",
        "invalid-kernel",
        "kmod-fails",
        "kpm",
        "unmanaged-kpm",
        "unknown-kpm",
        "kpm-without-runtime",
        "opt-out",
        "symlink-module",
    ] {
        exercise(case).await;
    }
}
