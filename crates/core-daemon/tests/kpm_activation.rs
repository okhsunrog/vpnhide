//! Real core process and RPC with isolated fake KernelPatch/PackageManager.
//! These tests validate orchestration, not an Android kernel or APatch supercall.
use std::fs;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use vpnhide_core_daemon::{CoreClient, config_file};
use vpnhide_core_rpc::{
    AppConfig, BaseRef, CanonicalConfig, ConfigEdit, ConfigToggle, NativeBackend,
    NativeBootOutcome, NativeRole, OperationId, PhaseOutcome, SelectionMode, SubmitError,
};

const KPATCH: &str = r#"#!/bin/sh
case "$1 $2" in
  "hello ") printf 'hello\n'; exit 0 ;;
  "kpm list")
    [ "$VH_CASE" = list-fails ] && exit 1
    [ -f "$VH_ROOT/loaded" ] && printf 'vpnhide\n'
    exit 0 ;;
  "kpm load")
    printf '%s\n' "$@" >> "$VH_ROOT/load-args"
    [ "$VH_CASE" = load-fails ] && exit 1
    [ "$VH_CASE" = load-no-identity ] && exit 0
    : > "$VH_ROOT/loaded"
    exit 0 ;;
  "kpm info")
    [ -f "$VH_ROOT/loaded" ] || exit 1
    case "$VH_CASE" in
      manual-old) v=1.1.0 ;;
      manual-garbled) v=nightly ;;
      manual-info|installed-live) v=1.3.1-2-gabcdef0 ;;
      *) exit 1 ;;
    esac
    printf 'name=vpnhide\nversion=%s\nlicense=GPL v2\nauthor=t\ndescription=d\nargs=filesystem_hiding=1\n' "$v"
    exit 0 ;;
  "kpm ctl0")
    case "$4" in
      "vpnhide 1 status")
        owner=1
        [ "$VH_CASE" = foreign ] && owner=99
        [ "$VH_CASE" = wrong-load ] && owner=99
        [ -f "$VH_ROOT/changed-owner" ] && owner=99
        version=1
        [ "$VH_CASE" = protocol ] && version=2
        error=0
        [ "$VH_CASE" = unhealthy ] && error=1
        [ "$VH_CASE" = partial ] && error=4
        printf 'vpnhide %s status\nbackend 0x%s\nkver 0x60100\nhooks 0x3ff\nerror 0x%s' "$version" "$owner" "$error"
        [ "$VH_CASE" = truncated ] || printf '\n'
        exit 64 ;;
      "vpnhide 2 config"*)
        printf 'config\n' >> "$VH_ROOT/dispatches"
        printf '%s' "$4" > "$VH_ROOT/wire"
        [ "$VH_CASE" = lost-reply ] && exit 42
        exit 0 ;;
    esac ;;
esac
exit 1
"#;

const PM: &str = r#"#!/bin/sh
[ "$1" = package ] || exit 64
shift
[ "$VH_CASE" = changed-owner ] && : > "$VH_ROOT/changed-owner"
if [ "$VH_CASE" = kernel-race ]; then
  printf 'vpnhide 1 status\nbackend 0x4\nkver 0x60100\nhooks 0x3ff\nerror 0x0\n' > "$VPNHIDE_KMOD_CTL"
fi
if [ "$2" = users ]; then
  printf 'Users:\n UserInfo{0:Owner:13} running\n'
else
  printf 'package:dev.okhsunrog.vpnhide uid:10123\npackage:com.example uid:10234\n'
fi
"#;

struct CoreProcess(Child);

impl Drop for CoreProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn executable(path: &Path, script: &str) {
    fs::write(path, script).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

async fn exercise(case: &str) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let modules = root.join("modules");
    let needs_load = matches!(
        case,
        "load"
            | "load-default"
            | "disabled-kmod"
            | "load-no-identity"
            | "unsupported"
            | "disabled"
            | "load-fails"
            | "wrong-load"
            | "kmod-fails"
    );
    if !needs_load {
        fs::write(root.join("loaded"), "").unwrap();
    }
    if needs_load || case == "installed-live" {
        let module = modules.join("vpnhide_kpm");
        fs::create_dir_all(&module).unwrap();
        fs::write(module.join("module.prop"), "id=vpnhide_kpm\n").unwrap();
        fs::write(module.join("vpnhide.kpm"), "fake artifact").unwrap();
        if case == "disabled" {
            fs::write(module.join("disable"), "").unwrap();
        }
    }
    if matches!(case, "active-with-kmod" | "kmod-fails" | "disabled-kmod") {
        let module = modules.join("vpnhide_kmod");
        fs::create_dir_all(&module).unwrap();
        fs::write(module.join("module.prop"), "id=vpnhide_kmod\n").unwrap();
        fs::write(module.join("vpnhide_kmod.ko"), "fake artifact").unwrap();
        if case == "disabled-kmod" {
            fs::write(module.join("disable"), "").unwrap();
        }
    }
    let ctl = root.join("ctl");
    if case == "conflict" {
        fs::write(
            &ctl,
            "vpnhide 1 status\nbackend 0x4\nkver 0x60100\nhooks 0x3ff\nerror 0x0\n",
        )
        .unwrap();
    }
    executable(&root.join("kpatch"), KPATCH);
    executable(&root.join("cmd"), PM);
    executable(
        &root.join("insmod"),
        "#!/bin/sh\nprintf 'insmod\\n' > \"$VH_ROOT/insmod-calls\"\nexit 1\n",
    );
    // Core reads the release from uname(2); host builds accept a pinned family so the
    // runner's own kernel never decides KPM support.
    let kernel_release = if case == "unsupported" {
        "6.10.0"
    } else {
        "6.1.44-android14"
    };
    let socket = root.join("core.sock");
    let canonical = root.join("canonical.json");
    // Core selects and loads at startup from the durable config it boots with.
    let mut initial = CanonicalConfig::default();
    if case != "load-default" {
        initial
            .settings
            .optional_features
            .insert("filesystem_iface_paths".into());
    }
    config_file::write_atomic(&canonical, &initial).unwrap();
    let lock_path = root.join("ctl.lock");
    let _held_lock = if case == "busy" {
        let lock = fs::File::create(&lock_path).unwrap();
        assert_eq!(
            unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
        Some(lock)
    } else {
        None
    };
    let mut process = CoreProcess(
        Command::new(env!("CARGO_BIN_EXE_vpnhide-core"))
            .arg(&socket)
            .env("PATH", root)
            .env("VH_ROOT", root)
            .env("VH_CASE", case)
            .env("VPNHIDE_TEST_KERNEL_RELEASE", kernel_release)
            .env("VPNHIDE_CONFIG_PATH", &canonical)
            .env("VPNHIDE_KMOD_CTL", &ctl)
            .env("VPNHIDE_KERNEL_MODULES", &modules)
            .env("VPNHIDE_KPM_LOCK", &lock_path)
            .env("VPNHIDE_CORE_OWNER_DIR", root.join("owner"))
            .env_remove("VPNHIDE_ZYGISK_MODULE")
            .env_remove("VPNHIDE_BOOT_APPLY")
            .stdout(Stdio::null())
            .stderr(fs::File::create(root.join("core.log")).unwrap())
            .spawn()
            .unwrap(),
    );
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
    if case != "load-default" {
        candidate
            .settings
            .optional_features
            .insert("filesystem_iface_paths".into());
    }
    candidate.apps.insert(
        "com.example".into(),
        AppConfig {
            native: NativeRole {
                enabled: true,
                kernel: None,
                zygisk: Some(vec![]),
            },
            ..Default::default()
        },
    );
    let edit = ConfigEdit::Replace {
        config: candidate.clone(),
    };
    client
        .submit_edit(OperationId(1), BaseRef { revision: 1 }, edit.clone())
        .await
        .unwrap()
        .unwrap();
    let record = client.get_operation(OperationId(1)).await.unwrap().unwrap();
    let succeeds = matches!(
        case,
        "active"
            | "manual-info"
            | "installed-live"
            | "load"
            | "load-default"
            | "disabled-kmod"
            | "partial"
            | "disabled"
    );
    // A disabled module is not a candidate: NoBackend has nothing to deliver.
    let delivers = succeeds && case != "disabled";
    let expected = if succeeds {
        PhaseOutcome::Confirmed
    } else if case == "lost-reply" {
        PhaseOutcome::Unknown
    } else {
        PhaseOutcome::FailedKnown
    };
    assert_eq!(
        record.settled,
        Some(expected),
        "{case}: {}",
        fs::read_to_string(root.join("core.log")).unwrap()
    );
    assert_eq!(
        config_file::read(&canonical).unwrap(),
        Some(candidate),
        "{case}: saved intent"
    );
    let published = client.current().await.unwrap();
    assert_eq!(published.snapshot.config_revision, 2, "{case}");
    let boot = published
        .snapshot
        .backend
        .boot
        .clone()
        .expect("typed boot status");
    match case {
        "active-with-kmod" | "kmod-fails" => assert!(
            matches!(boot.outcome, NativeBootOutcome::Conflict { .. }),
            "{case}: {:?}",
            boot.outcome
        ),
        "load" | "load-default" | "disabled-kmod" => assert!(
            matches!(
                boot.outcome,
                NativeBootOutcome::Selected {
                    backend: NativeBackend::Kpm,
                    mode: SelectionMode::Loaded,
                    ..
                }
            ),
            "{case}: {:?}",
            boot.outcome
        ),
        // A live KPM nobody loaded through core (APatch autoload, a hand-loaded
        // `.kpm`), or the module's own KPM already live: one candidate, adopted.
        "active" | "manual-info" | "installed-live" => assert_eq!(
            boot.outcome,
            NativeBootOutcome::Selected {
                backend: NativeBackend::Kpm,
                mode: SelectionMode::Adopted,
                filesystem_hiding: (case != "active").then_some(true),
            },
            "{case}"
        ),
        "manual-old" | "manual-garbled" | "foreign" | "protocol" | "truncated" => assert!(
            matches!(
                &boot.outcome,
                NativeBootOutcome::KpmIncompatible { found, minimum, .. }
                    if minimum == vpnhide_activator::MIN_KPM_VERSION
                        && found.as_deref()
                            == match case {
                                "manual-old" => Some("1.1.0"),
                                "manual-garbled" => Some("nightly"),
                                _ => None,
                            }
            ),
            "{case}: {:?}",
            boot.outcome
        ),
        "busy" | "list-fails" => assert!(
            matches!(boot.outcome, NativeBootOutcome::KpmUnverifiable { .. }),
            "{case}: {:?}",
            boot.outcome
        ),
        "unsupported" => assert!(
            matches!(boot.outcome, NativeBootOutcome::UnsupportedKernel { .. }),
            "{case}: {:?}",
            boot.outcome
        ),
        "disabled" => assert_eq!(boot.outcome, NativeBootOutcome::NoBackend, "{case}"),
        _ => {}
    }
    if delivers || case == "lost-reply" {
        assert_eq!(
            published.snapshot.backend.active,
            Some(NativeBackend::Kpm),
            "{case}"
        );
        let wire = fs::read_to_string(root.join("wire")).unwrap();
        let parsed = vpnhide_protocol::parse_config(wire.as_bytes()).unwrap();
        assert!(
            !parsed.targets.is_empty(),
            "{case}: KPM uses kernel overrides"
        );
        assert!(wire.contains("27fa"), "{case}: projected UID");
        assert_eq!(
            fs::read_to_string(root.join("dispatches")).unwrap(),
            "config\n",
            "{case}: exactly one dispatch"
        );
    } else {
        assert!(!root.join("wire").exists(), "{case}: must refuse delivery");
    }
    let loaded = root.join("load-args");
    assert_eq!(
        loaded.exists(),
        matches!(
            case,
            "load"
                | "load-default"
                | "disabled-kmod"
                | "load-no-identity"
                | "load-fails"
                | "wrong-load"
        ),
        "{case}: loading policy"
    );
    if loaded.exists() {
        assert_eq!(
            fs::read_to_string(loaded).unwrap(),
            format!(
                "kpm\nload\n{}\n{}",
                modules.join("vpnhide_kpm/vpnhide.kpm").display(),
                if case == "load-default" {
                    ""
                } else {
                    "filesystem_hiding=1\n"
                }
            ),
            "{case}"
        );
    }
    assert!(
        !root.join("insmod-calls").exists(),
        "{case}: no conflicting/fallback load"
    );
    if case == "lost-reply" {
        client
            .submit_edit(OperationId(1), BaseRef { revision: 1 }, edit.clone())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            client
                .submit_edit(OperationId(1), BaseRef { revision: 2 }, edit.clone())
                .await
                .unwrap(),
            Err(SubmitError::IdReused)
        ));
        assert!(matches!(
            client
                .submit_edit(
                    OperationId(1),
                    BaseRef { revision: 1 },
                    ConfigEdit::Toggle {
                        field: ConfigToggle::Debug,
                        enabled: true
                    }
                )
                .await
                .unwrap(),
            Err(SubmitError::IdReused)
        ));
        assert!(matches!(
            client
                .submit_edit(
                    OperationId(2),
                    BaseRef { revision: 2 },
                    ConfigEdit::Toggle {
                        field: ConfigToggle::Debug,
                        enabled: true
                    }
                )
                .await
                .unwrap(),
            Err(SubmitError::Unavailable)
        ));
        assert_eq!(
            fs::read_to_string(root.join("dispatches")).unwrap(),
            "config\n"
        );
    }
}

#[tokio::test]
async fn kpm_lifecycle_and_refusals_over_rpc() {
    for case in [
        "active",
        "manual-info",
        "manual-old",
        "manual-garbled",
        "installed-live",
        "active-with-kmod",
        "load",
        "load-default",
        "disabled-kmod",
        "load-no-identity",
        "partial",
        "unhealthy",
        "foreign",
        "protocol",
        "truncated",
        "changed-owner",
        "kernel-race",
        "conflict",
        "unsupported",
        "disabled",
        "load-fails",
        "wrong-load",
        "list-fails",
        "busy",
        "kmod-fails",
        "lost-reply",
    ] {
        exercise(case).await;
    }
}
