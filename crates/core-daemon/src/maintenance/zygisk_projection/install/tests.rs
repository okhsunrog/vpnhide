use super::*;
use std::os::unix::fs::PermissionsExt;
use vpnhide_core_rpc::{
    AppConfig, BaseRef, CanonicalConfig, ConfigEdit, OperationId, OperationRecord, PhaseRecord,
};
const BOOT_B: &str = "22222222-2222-2222-2222-222222222222";

pub(crate) fn fixture() -> (tempfile::TempDir, ProjectionPaths, Journal, PathBuf) {
    let (root, prepared) = super::super::tests::fixture();
    let paths = prepared.paths.clone();
    drop(prepared);
    fs::remove_dir_all(paths.source().join("bin")).unwrap();
    let incoming = paths.root.join("data/adb/modules_update/vpnhide_core");
    vpnhide_artifact_model::fixtures::write_passive_bundle_tree(&incoming).unwrap();
    fs::write(
        incoming.join("vpnhide-core"),
        fs::read(paths.source().join("vpnhide-core")).unwrap(),
    )
    .unwrap();
    fs::set_permissions(
        incoming.join("vpnhide-core"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    fs::remove_dir_all(incoming.join("bin")).unwrap();
    for path in [paths.source(), incoming.clone()] {
        fs::write(
            path.join("module.prop"),
            b"id=vpnhide_core\nauthor=okhsunrog\nvpnhideControl=2\nvpnhideActivation=1\n",
        )
        .unwrap();
    }
    let old_manifest = paths.source().join("payloads/manifest.json");
    let mut manifest = crate::payloads::Manifest::parse(&fs::read(&old_manifest).unwrap()).unwrap();
    manifest.bundle_commit = "1".repeat(40);
    fs::write(old_manifest, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    fs::create_dir(paths.source().join("zygisk")).unwrap();
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        fs::copy(
            paths.source().join(format!("payloads/zygisk/{abi}.so")),
            paths.source().join(format!("zygisk/{abi}.so")),
        )
        .unwrap();
    }
    let target = crate::zygisk::ZygiskTarget::open(&paths.source()).unwrap();
    let wire = vpnhide_protocol::format_config(
        false,
        0,
        &[vpnhide_protocol::Target {
            uid: 10001,
            hookmask: 1,
        }],
    );
    let authority = ZygiskAuthorityRecord {
        operation: OperationId(8),
        intent: crate::journal::NativeIntent {
            backend: NativeBackend::Zygisk,
            wire: wire.clone(),
            zygisk_module: Some(paths.source()),
            zygisk_authorization: Some(crate::journal::ZygiskAuthorizationIntent {
                boot_id: paths.boot().unwrap(),
                generation: 8,
                directory_identity: target.identity().unwrap(),
            }),
        },
    };
    target
        .publish_authorized(&vpnhide_protocol::zygisk_activation::Authorization {
            boot_id: paths.boot().unwrap(),
            generation: 8,
            wire,
        })
        .unwrap();
    let mut config = CanonicalConfig::default();
    let mut app = AppConfig::default();
    app.native.enabled = true;
    config.apps.insert("com.example.game".into(), app);
    let mut journal = Journal {
        issued_through: 8,
        zygisk_authority: Some(authority.clone()),
        ..Default::default()
    };
    journal.upsert(JournalEntry {
        admission: Some(crate::journal::Admission {
            from_revision: 0,
            commit_revision: 0,
            writes: vec![],
        }),
        command: Some(crate::journal::CommandIntent::Edit),
        maintenance: None,
        maintenance_report: None,
        superseded_by: None,
        superseded_phases: vec![],
        finalized_unknown: false,
        base: BaseRef { revision: 0 },
        edit: ConfigEdit::RepairActivation,
        candidate: Some(config.clone()),
        native_intent: Some(authority.intent),
        record: OperationRecord {
            op: OperationId(8),
            phases: vec![PhaseRecord {
                phase: ConfigPhase::Native,
                outcome: PhaseOutcome::Confirmed,
            }],
            settled: Some(PhaseOutcome::Confirmed),
            ports: None,
        },
    });
    journal.note_zygisk_exposure(&paths.boot().unwrap());
    fs::create_dir_all(paths.root.join("data/system")).unwrap();
    crate::config_file::write_atomic(&paths.root.join("data/system/vpnhide_config.json"), &config)
        .unwrap();
    crate::journal::write_atomic(&paths.persistent.join("operations.json"), &journal).unwrap();
    (root, paths, journal, incoming)
}
pub(crate) fn dispatch(
    paths: &ProjectionPaths,
    incoming: &Path,
    checkpoint: impl FnMut(&str) -> io::Result<()>,
) -> io::Result<()> {
    dispatch_owned(
        paths,
        incoming,
        &File::open(incoming.join("vpnhide-core"))?,
        &File::open(paths.source().join("vpnhide-core"))?,
        &paths.persistent.join("operations.json"),
        unsafe { libc::geteuid() },
        checkpoint,
    )
}
pub(crate) fn promote(paths: &ProjectionPaths, incoming: &Path) {
    fs::rename(paths.source(), paths.root.join("old-core-retained")).unwrap();
    fs::rename(incoming, paths.source()).unwrap();
    fs::rename(paths.staged(), paths.active()).unwrap();
    fs::write(paths.root.join("proc/sys/kernel/random/boot_id"), BOOT_B).unwrap();
}
#[test]
fn install_preserves_old_runtime_and_journal_then_accepts_only_real_new_boot_code() {
    let (_root, paths, journal, incoming) = fixture();
    let old = ImageFence::lease_owned(&paths.source(), unsafe { libc::geteuid() }).unwrap();
    let grant = fs::read(
        paths
            .source()
            .join(vpnhide_protocol::zygisk_activation::FILE_NAME),
    )
    .unwrap();
    let operations = fs::read(paths.persistent.join("operations.json")).unwrap();
    let canonical = fs::read(paths.root.join("data/system/vpnhide_config.json")).unwrap();
    let mut boundaries = Vec::new();
    dispatch(&paths, &incoming, |boundary| {
        boundaries.push(boundary.to_string());
        assert_eq!(
            fs::read(paths.persistent.join("operations.json"))?,
            operations
        );
        assert_eq!(
            fs::read(paths.root.join("data/system/vpnhide_config.json"))?,
            canonical
        );
        assert_eq!(
            fs::read(
                paths
                    .source()
                    .join(vpnhide_protocol::zygisk_activation::FILE_NAME)
            )?,
            grant
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(boundaries, ["intent", "published", "receipt"]);
    old.0
        .require_unchanged_owned(&paths.source(), unsafe { libc::geteuid() })
        .unwrap();
    assert!(
        !paths
            .staged()
            .join(vpnhide_protocol::zygisk_activation::FILE_NAME)
            .exists()
    );
    assert!(
        InstallReadback::prepare_owned(
            &paths,
            &File::open(incoming.join("vpnhide-core")).unwrap(),
            unsafe { libc::geteuid() }
        )
        .is_err()
    );
    // Root-manager staging may update friendly metadata but must retain the old
    // activation contract until normal boot promotion replaces the old code.
    fs::copy(
        incoming.join("module.prop"),
        paths.source().join("module.prop"),
    )
    .unwrap();
    crate::zygisk::ZygiskTarget::open(&paths.source())
        .unwrap()
        .require_authorization(journal.zygisk_authority.as_ref().unwrap())
        .unwrap();
    promote(&paths, &incoming);
    let running = File::open(paths.source().join("vpnhide-core")).unwrap();
    let readback = InstallReadback::prepare_owned(&paths, &running, unsafe { libc::geteuid() })
        .unwrap()
        .unwrap();
    assert!(
        readback.origin.intent.accepts(
            &journal,
            journal
                .get(OperationId(8))
                .unwrap()
                .candidate
                .as_ref()
                .unwrap()
        )
    );
    fs::write(paths.active().join("zygisk/arm64-v8a.so"), b"replaced").unwrap();
    assert!(
        readback
            .require_owned(&paths, &running, unsafe { libc::geteuid() })
            .is_err()
    );
}
#[test]
fn each_crash_boundary_is_readback_only_and_never_rewrites_old_state() {
    for stop in ["intent", "published", "receipt"] {
        let (_root, paths, _journal, incoming) = fixture();
        let old_journal = fs::read(paths.persistent.join("operations.json")).unwrap();
        assert!(
            dispatch(&paths, &incoming, |boundary| if boundary == stop {
                Err(io::Error::other("crash"))
            } else {
                Ok(())
            })
            .is_err()
        );
        assert_eq!(
            fs::read(paths.persistent.join("operations.json")).unwrap(),
            old_journal
        );
        let stage = paths.staged().is_dir();
        assert_eq!(stage, stop != "intent");
        let intent = fs::read(paths.persistent.join(STORE).join("intent.json")).unwrap();
        let mut calls = 0;
        let retry = dispatch(&paths, &incoming, |_| {
            calls += 1;
            Ok(())
        });
        assert_eq!(retry.is_ok(), stage);
        assert_eq!(calls, 0);
        assert_eq!(
            fs::read(paths.persistent.join(STORE).join("intent.json")).unwrap(),
            intent
        );
        if stage {
            promote(&paths, &incoming);
            assert!(
                InstallReadback::prepare_owned(
                    &paths,
                    &File::open(paths.source().join("vpnhide-core")).unwrap(),
                    unsafe { libc::geteuid() }
                )
                .unwrap()
                .is_some()
            );
        }
    }
}
#[test]
fn changed_grant_user_intent_namespace_or_destination_never_publish() {
    for variant in 0..5 {
        let (_root, paths, mut journal, incoming) = fixture();
        match variant {
            0 => {
                fs::write(
                    paths
                        .source()
                        .join(vpnhide_protocol::zygisk_activation::FILE_NAME),
                    b"foreign",
                )
                .unwrap();
            }
            1 => {
                journal.get_mut(OperationId(8)).unwrap().record.phases[0].outcome =
                    PhaseOutcome::Unknown;
                journal.get_mut(OperationId(8)).unwrap().record.settled =
                    Some(PhaseOutcome::Unknown);
                crate::journal::write_atomic(&paths.persistent.join("operations.json"), &journal)
                    .unwrap();
            }
            2 => {
                fs::create_dir(paths.staged()).unwrap();
            }
            3 => {
                fs::write(
                    incoming.join("module.prop"),
                    b"id=vpnhide_core\nvpnhideControl=2\n",
                )
                .unwrap();
            }
            _ => {
                let mut config = journal
                    .get(OperationId(8))
                    .unwrap()
                    .candidate
                    .clone()
                    .unwrap();
                config.apps.clear();
                crate::config_file::write_atomic(
                    &paths.root.join("data/system/vpnhide_config.json"),
                    &config,
                )
                .unwrap();
            }
        }
        let before = fs::read(paths.persistent.join("operations.json")).unwrap();
        assert!(
            dispatch(&paths, &incoming, |_| Ok(())).is_err(),
            "variant={variant}"
        );
        assert_eq!(
            fs::read(paths.persistent.join("operations.json")).unwrap(),
            before
        );
        assert!(!paths.staged().join(FILE_NAME).exists());
    }
}

#[test]
fn install_intent_fsync_rechecks_incoming_and_preserves_concurrent_user_edits() {
    for variant in 0..3 {
        let (_root, paths, _journal, incoming) = fixture();
        let grant = fs::read(
            paths
                .source()
                .join(vpnhide_protocol::zygisk_activation::FILE_NAME),
        )
        .unwrap();
        let outcome = dispatch(&paths, &incoming, |boundary| {
            if boundary == "intent" {
                match variant {
                    0 => fs::write(incoming.join("payloads/manifest.json"), b"changed incoming")?,
                    1 => {
                        let path = paths.root.join("data/system/vpnhide_config.json");
                        let mut config = crate::config_file::read(&path)?.unwrap();
                        config.debug_switch = true;
                        crate::config_file::write_atomic(&path, &config)?;
                    }
                    _ => fs::write(paths.source().join("service.sh"), b"source replaced")?,
                }
            }
            Ok(())
        });
        assert!(outcome.is_err(), "variant={variant}");
        assert!(!paths.staged().exists());
        assert_eq!(
            fs::read(
                paths
                    .source()
                    .join(vpnhide_protocol::zygisk_activation::FILE_NAME)
            )
            .unwrap(),
            grant
        );
        if variant == 1 {
            assert!(
                crate::config_file::read(&paths.root.join("data/system/vpnhide_config.json"))
                    .unwrap()
                    .unwrap()
                    .debug_switch
            );
        }
    }
}
#[test]
fn install_latest_kernel_receipt_overrides_historical_zygisk_authority() {
    let (_root, paths, mut journal, incoming) = fixture();
    let mut kernel = journal.get(OperationId(8)).unwrap().clone();
    kernel.record.op = OperationId(9);
    kernel.native_intent.as_mut().unwrap().backend = NativeBackend::Builtin;
    kernel.native_intent.as_mut().unwrap().zygisk_module = None;
    kernel.native_intent.as_mut().unwrap().zygisk_authorization = None;
    journal.upsert(kernel);
    crate::journal::write_atomic(&paths.persistent.join("operations.json"), &journal).unwrap();
    assert!(dispatch(&paths, &incoming, |_| Ok(())).is_err());
    assert!(!paths.staged().exists());
}
#[test]
fn install_compatibility_metadata_cannot_turn_a_passive_core_into_a_recipient() {
    let (_root, paths, _journal, incoming) = fixture();
    assert!(crate::zygisk::ZygiskTarget::open(&incoming).is_err());
    assert!(!incoming.join("zygisk").exists());
    assert!(!paths.active().exists());
}

#[test]
fn unused_autoload_never_creates_standby_and_does_not_block_a_known_kernel_update() {
    let (_root, paths, mut journal, _incoming) = fixture();
    assert!(!unused_root_owned(&paths, &journal).unwrap());
    let entry = journal.get_mut(OperationId(8)).unwrap();
    entry.native_intent.as_mut().unwrap().backend = NativeBackend::Builtin;
    entry.native_intent.as_mut().unwrap().zygisk_module = None;
    entry.native_intent.as_mut().unwrap().zygisk_authorization = None;
    fs::remove_file(
        paths
            .source()
            .join(vpnhide_protocol::zygisk_activation::FILE_NAME),
    )
    .unwrap();
    assert!(unused_root_owned(&paths, &journal).unwrap());
    assert!(!paths.staged().exists());
    assert!(!paths.persistent.join(STORE).exists());
}
#[test]
fn install_metadata_and_real_executable_fences_reject_even_identical_other_inodes() {
    let (_root, paths, _journal, incoming) = fixture();
    let result = dispatch_owned(
        &paths,
        &incoming,
        &File::open(incoming.join("vpnhide-core")).unwrap(),
        &File::open(incoming.join("vpnhide-core")).unwrap(),
        &paths.persistent.join("operations.json"),
        unsafe { libc::geteuid() },
        |_| Ok(()),
    );
    assert!(result.is_err());
    assert!(!paths.staged().exists());
    assert!(!paths.persistent.join(STORE).exists());
}

pub(crate) fn manager(paths: &ProjectionPaths) -> Manager {
    Manager::discover_owned(&paths.root, unsafe { libc::geteuid() }).unwrap()
}
