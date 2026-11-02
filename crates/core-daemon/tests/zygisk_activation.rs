//! Policy publication through the actual core process/RPC. No APK, LSPosed,
//! Zygisk injection or companion participates in these host scenarios.

mod common;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;
use std::process::{Child, Stdio};
use std::time::Duration;

use vpnhide_core_client::CoreClient;
use vpnhide_core_daemon::config_file;
use vpnhide_core_rpc::{
    AppConfig, BaseRef, CanonicalConfig, ConfigEdit, NativeBackend, NativeBootOutcome, NativeRole,
    OperationId, PhaseOutcome, SelectionMode, ZygiskTargets,
};

const OLD: &str = "untouched policy";
const PM: &str = r#"#!/bin/sh
[ "$1" = package ] || exit 64
shift
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
[ "$1" = hello ] && { printf 'hello2026\n'; exit 0; }
if [ "$2" = num ]; then
  if [ "$VH_CASE" = kpm-race ] && [ ! -f "$VH_ROOT/kpm-now" ]; then printf '0\n'; else printf '1\n'; fi
  exit 0
fi
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
    executable(&root.join("cmd"), PM);
    executable(
        &root.join("insmod"),
        "#!/bin/sh\nprintf 'attempted\\n' > \"$VH_ROOT/insmod-dispatch\"\nexit 1\n",
    );
    let socket = root.join("core.sock");
    let canonical = root.join("canonical.json");
    let mut command = common::core_command();
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
        .env_remove("VPNHIDE_BOOT_APPLY")
        .env("VPNHIDE_CORE_OWNER_DIR", root.join("owner"))
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
    if case == "crash-native" {
        command.env("VPNHIDE_TEST_CRASH_AFTER", "native");
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
    let client = CoreClient::connect(&socket);
    common::reserve_operation_ids(&client, 2).await;
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
    let submitted = client
        .submit_edit(
            OperationId(1),
            BaseRef { revision: 0 },
            ConfigEdit::Replace {
                config: candidate.clone(),
            },
        )
        .await;
    if case == "crash-native" {
        // Admission may have reached the client before the injected crash.
        let _ = submitted;
        process.0.wait().unwrap();
        let published_wire = fs::read_to_string(&policy).unwrap();
        let parsed = vpnhide_protocol::parse_config(published_wire.as_bytes()).unwrap();
        assert_eq!(parsed.targets.len(), 1);
        assert_eq!(parsed.targets[0].uid, 10234);
        let journal =
            vpnhide_core_daemon::journal::read(&canonical.with_file_name("core_operations.json"))
                .unwrap();
        let intent = journal
            .get(OperationId(1))
            .unwrap()
            .native_intent
            .as_ref()
            .unwrap();
        assert_eq!(intent.backend, NativeBackend::Zygisk);
        assert_eq!(intent.wire, published_wire);
        let published_at = fs::metadata(&policy).unwrap().modified().unwrap();

        fs::remove_file(&socket).unwrap();
        command.env_remove("VPNHIDE_TEST_CRASH_AFTER");
        process = Process(command.spawn().unwrap());
        tokio::time::timeout(Duration::from_secs(5), async {
            while !socket.exists() {
                assert!(process.0.try_wait().unwrap().is_none());
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let restarted = CoreClient::connect(&socket);
        let record = restarted
            .get_operation(OperationId(1))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.settled, Some(PhaseOutcome::Confirmed));
        assert_eq!(
            fs::metadata(&policy).unwrap().modified().unwrap(),
            published_at,
            "recovery must read the exact intent without re-publishing policy"
        );
        return;
    }
    submitted.unwrap().unwrap();
    let record = common::settled(&client, OperationId(1)).await;
    let expected = expectation(case);
    assert_eq!(
        record.settled,
        Some(expected.settled),
        "{case}: {}",
        fs::read_to_string(root.join("core.log")).unwrap()
    );
    assert_eq!(
        config_file::read(&canonical).unwrap(),
        Some(candidate),
        "{case}: persisted intent"
    );
    let published = client.current().await.unwrap();
    assert_eq!(
        published.snapshot.backend.selected, expected.selected,
        "{case}: selected is distinct from active"
    );
    let boot = published
        .snapshot
        .backend
        .boot
        .clone()
        .expect("typed boot status");
    assert_eq!(
        std::mem::discriminant(&boot.outcome),
        std::mem::discriminant(&expected.outcome),
        "{case}: {:?}",
        boot.outcome
    );
    assert_eq!(published.snapshot.config_revision, 1);
    match expected.policy {
        Policy::Published => {
            assert_eq!(boot.zygisk_targets, ZygiskTargets::Published, "{case}");
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
            common::settled(&client, OperationId(2)).await;
            let wire = fs::read_to_string(&policy).unwrap();
            assert!(
                vpnhide_protocol::parse_config(wire.as_bytes())
                    .unwrap()
                    .targets
                    .is_empty()
            );
            assert_eq!(client.current().await.unwrap().snapshot.config_revision, 2);
        }
        Policy::Retired => {
            assert_eq!(boot.zygisk_targets, ZygiskTargets::Retired, "{case}");
            let wire = fs::read_to_string(&policy).unwrap();
            assert!(
                vpnhide_protocol::parse_config(wire.as_bytes())
                    .unwrap()
                    .targets
                    .is_empty(),
                "{case}: a non-selected Zygisk keeps no targets"
            );
        }
        Policy::Untouched => assert_eq!(
            fs::read_to_string(&policy).unwrap(),
            OLD,
            "{case}: refused or not-installed recipient is never written"
        ),
    }
    assert!(
        !root.join("insmod-dispatch").exists(),
        "{case}: a conflict loads nothing"
    );
    assert!(
        !root.join("kpm-dispatch").exists(),
        "{case}: a conflict configures nothing"
    );
}

enum Policy {
    Published,
    Retired,
    Untouched,
}

struct Expected {
    settled: PhaseOutcome,
    selected: Option<NativeBackend>,
    outcome: NativeBootOutcome,
    policy: Policy,
}

/// Strict once-per-boot semantics: Zygisk counts as a native backend, so any second
/// candidate is a conflict that loads nothing and retires Zygisk's targets.
fn expectation(case: &str) -> Expected {
    let selected_zygisk = NativeBootOutcome::Selected {
        backend: NativeBackend::Zygisk,
        mode: SelectionMode::Adopted,
        filesystem_hiding: None,
    };
    let conflict = NativeBootOutcome::Conflict {
        installed: vec![],
        live: vec![],
        staged: vec![],
    };
    let ok = |policy| Expected {
        settled: PhaseOutcome::Confirmed,
        selected: Some(NativeBackend::Zygisk),
        outcome: selected_zygisk.clone(),
        policy,
    };
    let refused = Expected {
        settled: PhaseOutcome::FailedKnown,
        selected: Some(NativeBackend::Zygisk),
        outcome: NativeBootOutcome::DeliveryFailed {
            backend: NativeBackend::Zygisk,
            detail: String::new(),
        },
        policy: Policy::Untouched,
    };
    let stopped = |outcome| Expected {
        settled: PhaseOutcome::FailedKnown,
        selected: None,
        outcome,
        policy: Policy::Retired,
    };
    match case {
        "publish" | "arm32" | "first-publish" | "opt-out" => ok(Policy::Published),
        "unknown-kpm" => stopped(NativeBootOutcome::KpmUnverifiable {
            detail: String::new(),
        }),
        "disable" | "remove" => Expected {
            settled: PhaseOutcome::Confirmed,
            selected: None,
            outcome: NativeBootOutcome::NoBackend,
            policy: Policy::Untouched,
        },
        "foreign" | "legacy-profile" | "changed-recipient" | "kernel-race" | "kpm-race"
        | "symlink-module" => refused,
        "kernel" | "kmod-fails" | "kpm" | "unmanaged-kpm" | "kpm-without-runtime" => {
            stopped(conflict)
        }
        "invalid-kernel" => stopped(NativeBootOutcome::Unknown {
            detail: String::new(),
        }),
        other => panic!("no expectation for {other}"),
    }
}

#[tokio::test]
async fn zygisk_crash_recovers_exact_uid_wire_without_republishing() {
    exercise("crash-native").await;
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

#[tokio::test]
async fn bundled_authorization_crash_recovery_never_confirms_an_old_generation() {
    use vpnhide_core_daemon::{config_file, journal};
    use vpnhide_protocol::zygisk_activation::{Authorization, FILE_NAME};
    for point in ["native-intent", "native"] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let module = root.join("recipient");
        fs::create_dir_all(module.join("zygisk")).unwrap();
        fs::write(
            module.join("module.prop"),
            "id=vpnhide_core\nvpnhideControl=2\nvpnhideActivation=1\n",
        )
        .unwrap();
        artifact(&module.join("zygisk/arm64-v8a.so"), 2, 183);
        executable(&root.join("cmd"), PM);
        let config_path = root.join("config.json");
        config_file::write_atomic(&config_path, &CanonicalConfig::default()).unwrap();
        let socket = root.join("core.sock");
        let spawn = |crash: Option<&str>| {
            let mut command = common::core_command();
            command
                .arg(&socket)
                .env("PATH", root)
                .env("VPNHIDE_CONFIG_PATH", &config_path)
                .env("VPNHIDE_CORE_OWNER_DIR", root.join("owner"))
                .env("VPNHIDE_KMOD_CTL", root.join("ctl"))
                .env("VPNHIDE_ZYGISK_MODULE", &module)
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            if let Some(crash) = crash {
                command.env("VPNHIDE_TEST_CRASH_AFTER", crash);
            }
            Process(command.spawn().unwrap())
        };
        let wait = |mut process: Process| async {
            tokio::time::timeout(Duration::from_secs(5), async {
                while !socket.exists() {
                    assert!(process.0.try_wait().unwrap().is_none());
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            process
        };
        let first = wait(spawn(None)).await;
        let client = CoreClient::connect(&socket);
        common::reserve_operation_ids(&client, 1).await;
        client
            .submit_edit(
                OperationId(1),
                BaseRef { revision: 1 },
                ConfigEdit::RepairActivation,
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            common::settled(&client, OperationId(1)).await.settled,
            Some(PhaseOutcome::Confirmed)
        );
        let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap();
        let before = fs::read(module.join(FILE_NAME)).unwrap();
        assert_eq!(
            Authorization::decode(&before, boot.trim())
                .unwrap()
                .generation,
            1
        );
        drop(client);
        drop(first);
        let _ = fs::remove_file(&socket);
        let mut failing = wait(spawn(Some(point))).await;
        let client = CoreClient::connect(&socket);
        common::reserve_operation_ids(&client, 2).await;
        let _ = client
            .submit_edit(
                OperationId(2),
                BaseRef { revision: 1 },
                ConfigEdit::RepairActivation,
            )
            .await;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(status) = failing.0.try_wait().unwrap() {
                    assert_eq!(status.code(), Some(97));
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let stored = journal::read(&root.join("core_operations.json")).unwrap();
        let intent = stored
            .get(OperationId(2))
            .unwrap()
            .native_intent
            .as_ref()
            .unwrap();
        assert_eq!(intent.zygisk_authorization.as_ref().unwrap().generation, 2);
        let interrupted = fs::read(module.join(FILE_NAME)).unwrap();
        drop(client);
        drop(failing);
        let _ = fs::remove_file(&socket);
        let _restarted = wait(spawn(None)).await;
        let client = CoreClient::connect(&socket);
        let expected = if point == "native" {
            PhaseOutcome::Confirmed
        } else {
            PhaseOutcome::Unknown
        };
        assert_eq!(
            common::settled(&client, OperationId(2)).await.settled,
            Some(expected)
        );
        assert_eq!(
            fs::read(module.join(FILE_NAME)).unwrap(),
            interrupted,
            "restart must not republish"
        );
        if point == "native-intent" {
            assert_eq!(
                interrupted, before,
                "the old authorization can still authorize old processes"
            );
            common::reserve_operation_ids(&client, 3).await;
            client
                .submit_edit(
                    OperationId(3),
                    BaseRef { revision: 1 },
                    ConfigEdit::RepairActivation,
                )
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                common::settled(&client, OperationId(3)).await.settled,
                Some(PhaseOutcome::Confirmed)
            );
            assert_eq!(
                common::settled(&client, OperationId(2)).await.settled,
                Some(PhaseOutcome::Unknown)
            );
            assert_eq!(
                Authorization::decode(&fs::read(module.join(FILE_NAME)).unwrap(), boot.trim())
                    .unwrap()
                    .generation,
                3
            );
        }
    }
}
