//! Host integration: a config whose native target set exceeds backend capacity is refused
//! whole. Save fails with a typed error and nothing is persisted, delivered or recorded;
//! Repair of an over-capacity persisted config writes nothing to the backend and reports
//! a typed outcome. Never a truncated, partially applied target set.

mod common;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Child, Stdio};
use std::time::Duration;

use vpnhide_core_client::CoreClient;
use vpnhide_core_daemon::config_file;
use vpnhide_core_rpc::{
    AppConfig, BaseRef, CanonicalConfig, ConfigEdit, ConfigPhase, NativeBackend, NativeBootOutcome,
    NativeRole, OperationId, PhaseOutcome, SubmitError,
};

const CAPACITY: usize = 160;
const BUILTIN_STATUS: &str = "vpnhide 1 status\nbackend 0x4\nkver 0x0\nhooks 0x0\nerror 0x0\n";

struct CoreProcess(Child);

impl Drop for CoreProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn package(i: usize) -> String {
    format!("com.example.p{i:03}")
}

/// A `cmd package` stand-in (builtins only: the test PATH holds nothing else) that knows
/// the app itself and `count` ordinary packages.
fn fake_cmd(path: &Path, count: usize) {
    let mut lines = vec!["package:dev.okhsunrog.vpnhide uid:10123".to_string()];
    lines.extend((0..count).map(|i| format!("package:{} uid:{}", package(i), 10_200 + i)));
    let quoted = lines
        .iter()
        .map(|l| format!("'{l}'"))
        .collect::<Vec<_>>()
        .join(" ");
    let script = format!(
        "#!/bin/sh\n[ \"$1\" = package ] || exit 64\nshift\nif [ \"$2\" = users ]; then\n  printf 'Users:\\n UserInfo{{0:Owner:13}} running\\n'\nelse\n  printf '%s\\n' {quoted}\nfi\n"
    );
    fs::write(path, script).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn config_with(count: usize) -> CanonicalConfig {
    let mut config = CanonicalConfig::default();
    for i in 0..count {
        config.apps.insert(
            package(i),
            AppConfig {
                native: NativeRole {
                    enabled: true,
                    kernel: None,
                    zygisk: None,
                },
                ..Default::default()
            },
        );
    }
    config
}

struct Harness {
    _dir: tempfile::TempDir,
    _child: CoreProcess,
    client: std::sync::Arc<CoreClient>,
    ctl: std::path::PathBuf,
    config_path: std::path::PathBuf,
}

async fn start(initial: &CanonicalConfig, installed: usize) -> Harness {
    start_with_preference(initial, installed, None).await
}

async fn start_with_preference(
    initial: &CanonicalConfig,
    installed: usize,
    preference: Option<NativeBackend>,
) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("core.sock");
    let ctl = dir.path().join("ctl");
    let config_path = dir.path().join("config.json");
    fs::create_dir_all(dir.path().join("modules")).unwrap();
    // The built-in driver is live and adopted: it is the selected backend.
    fs::write(&ctl, BUILTIN_STATUS).unwrap();
    config_file::write_atomic(&config_path, initial).unwrap();
    fake_cmd(&dir.path().join("cmd"), installed);
    if let Some(backend) = preference {
        let journal = vpnhide_core_daemon::journal::Journal {
            issued_through: 1,
            backend_choice: Some(
                vpnhide_core_daemon::journal::backend_choice::BackendChoiceRecord {
                    revision: 1,
                    operation: OperationId(1),
                    requested_in_boot: "previous-kernel-boot".into(),
                    request: vpnhide_core_rpc::BackendChoiceRequest {
                        expected_revision: 0,
                        choice: vpnhide_core_rpc::BackendChoice::Fixed { backend },
                        acknowledged_warning: None,
                    },
                },
            ),
            ..Default::default()
        };
        vpnhide_core_daemon::journal::write_atomic(
            &config_path.with_file_name("core_operations.json"),
            &journal,
        )
        .unwrap();
    }

    let mut child = CoreProcess(
        common::core_command()
            .arg(&socket)
            .env("PATH", dir.path())
            .env("VPNHIDE_CONFIG_PATH", &config_path)
            .env("VPNHIDE_CORE_OWNER_DIR", dir.path().join("owner"))
            .env("VPNHIDE_KMOD_CTL", &ctl)
            .env("VPNHIDE_KERNEL_MODULES", dir.path().join("modules"))
            .env_remove("VPNHIDE_KPM_LOCK")
            .env_remove("VPNHIDE_ZYGISK_MODULE")
            .env_remove("VPNHIDE_BOOT_APPLY")
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
    let client = CoreClient::connect(&socket);
    common::reserve_operation_ids(&client, 2).await;
    client.hello().await.unwrap();
    Harness {
        _dir: dir,
        _child: child,
        client,
        ctl,
        config_path,
    }
}

#[tokio::test]
async fn an_over_capacity_save_is_refused_and_writes_nothing() {
    let installed = CAPACITY + 10;
    let h = start(&CanonicalConfig::default(), installed).await;

    // A config that fits is applied normally.
    let fits = config_with(5);
    h.client
        .submit_edit(
            OperationId(1),
            BaseRef { revision: 1 },
            ConfigEdit::Replace {
                config: fits.clone(),
            },
        )
        .await
        .unwrap()
        .unwrap();
    let record = common::settled(&h.client, OperationId(1)).await;
    assert_eq!(record.settled, Some(PhaseOutcome::Confirmed));
    let applied_wire = fs::read_to_string(&h.ctl).unwrap();
    assert!(
        applied_wire.starts_with("vpnhide 2 config\n"),
        "{applied_wire}"
    );
    let revision = h.client.current().await.unwrap().snapshot.config_revision;

    // One that does not fit is refused at Save, typed, with what to deselect.
    let refused = h
        .client
        .submit_edit(
            OperationId(2),
            BaseRef { revision },
            ConfigEdit::Replace {
                config: config_with(installed),
            },
        )
        .await
        .unwrap();
    let Err(SubmitError::NativeCapacityExceeded(report)) = refused else {
        panic!("expected a capacity refusal, got {refused:?}");
    };
    assert_eq!(report.capacity as usize, CAPACITY);
    assert!(report.total as usize > CAPACITY, "{report:?}");
    let listed: Vec<_> = report.packages.iter().map(|p| p.package.as_str()).collect();
    assert!(listed.contains(&package(installed - 1).as_str()));
    assert!(report.packages.iter().all(|p| !p.uids.is_empty()));

    // No partial write anywhere: the backend keeps the previous policy byte for byte,
    // the durable config and revision are unchanged, and no operation was recorded.
    assert_eq!(fs::read_to_string(&h.ctl).unwrap(), applied_wire);
    assert_eq!(
        config_file::read(&h.config_path).unwrap(),
        Some(fits.clone())
    );
    let published = h.client.current().await.unwrap();
    assert_eq!(published.snapshot.config_revision, revision);
    assert_eq!(published.snapshot.config, Some(fits));
    assert_eq!(h.client.get_operation(OperationId(2)).await.unwrap(), None);
}

#[tokio::test]
async fn repair_of_an_over_capacity_config_reports_it_and_writes_nothing() {
    let installed = CAPACITY + 3;
    // E.g. more clones of selected apps appeared since the config was saved.
    let h = start(&config_with(installed), installed).await;
    let revision = h.client.current().await.unwrap().snapshot.config_revision;

    h.client
        .submit_edit(
            OperationId(1),
            BaseRef { revision },
            ConfigEdit::RepairActivation,
        )
        .await
        .unwrap()
        .unwrap();
    let record = common::settled(&h.client, OperationId(1)).await;
    assert_eq!(record.settled, Some(PhaseOutcome::FailedKnown));
    let native = record
        .phases
        .iter()
        .find(|p| p.phase == ConfigPhase::Native)
        .unwrap();
    assert_eq!(native.outcome, PhaseOutcome::FailedKnown);
    assert_eq!(fs::read_to_string(&h.ctl).unwrap(), BUILTIN_STATUS);

    // The durable receipt can be read before the separate native read-model
    // publication completes. Require the typed refusal, not incidental timing
    // between two RPCs. Never retry the mutation or accept another outcome.
    let boot = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(boot) = h.client.current().await.unwrap().snapshot.backend.boot
                && matches!(&boot.outcome, NativeBootOutcome::CapacityExceeded { .. })
            {
                break boot;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("capacity refusal published after terminal receipt");
    assert_eq!(fs::read_to_string(&h.ctl).unwrap(), BUILTIN_STATUS);
    let NativeBootOutcome::CapacityExceeded { backend, report } = boot.outcome else {
        panic!("expected CapacityExceeded, got {:?}", boot.outcome);
    };
    assert_eq!(backend, NativeBackend::Builtin);
    assert_eq!(report.capacity as usize, CAPACITY);
    assert!(report.total as usize > CAPACITY);
}

#[tokio::test]
async fn save_cannot_overflow_retained_desired_family_even_when_current_wire_fits() {
    // A kernel upgrade may reveal Builtin while retaining an earlier KPM choice.
    // Admission still preserves the desired family's independent byte ceiling.
    let h = start_with_preference(
        &CanonicalConfig::default(),
        CAPACITY,
        Some(NativeBackend::Kpm),
    )
    .await;
    let mut candidate = config_with(CAPACITY - 1);
    for (i, app) in candidate.apps.values_mut().enumerate() {
        // Different masks deliberately defeat compact same-mask grouping.
        let names = [
            "dev_ioctl",
            "sock_ioctl",
            "rtnl_fill_ifinfo",
            "inet_fill_ifaddr",
            "inet6_fill_ifaddr",
            "fib_dump_info",
            "rt6_fill_node",
        ];
        let bits = (i % ((1 << names.len()) - 1)) + 1;
        app.native.kernel = Some(
            names
                .iter()
                .enumerate()
                .filter(|(bit, _)| bits & (1 << bit) != 0)
                .map(|(_, name)| (*name).into())
                .collect(),
        );
    }
    let before = h.client.current().await.unwrap().snapshot;
    let canonical_before = fs::read(&h.config_path).unwrap();
    let ctl_before = fs::read(&h.ctl).unwrap();
    let resolver = vpnhide_activator::parse_pm_packages(
        &(0..CAPACITY)
            .map(|i| format!("package:{} uid:{}\n", package(i), 10_200 + i))
            .collect::<String>(),
    );
    let wire = vpnhide_core_daemon::activation::project_native_wire_with_resolver(
        &candidate,
        NativeBackend::Builtin,
        &resolver,
    )
    .unwrap();
    assert!(
        wire.len()
            <= vpnhide_activator::NativeHookFamily::Builtin
                .limits()
                .wire_bytes
    );
    assert!(wire.len() > vpnhide_activator::NativeHookFamily::Kpm.limits().wire_bytes);
    let refusal = h
        .client
        .submit_edit(
            OperationId(2),
            BaseRef {
                revision: before.config_revision,
            },
            ConfigEdit::Replace { config: candidate },
        )
        .await
        .unwrap();
    assert!(matches!(refusal, Err(SubmitError::Rejected { .. })));
    assert_eq!(fs::read(&h.config_path).unwrap(), canonical_before);
    assert_eq!(fs::read(&h.ctl).unwrap(), ctl_before);
    assert!(
        h.client
            .get_operation(OperationId(2))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        h.client.current().await.unwrap().snapshot.config_revision,
        before.config_revision
    );
}
