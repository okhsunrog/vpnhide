use super::*;
use crate::native_boot::prepared::BundleIdentity;
use crate::payloads::{
    Catalog, PayloadKind,
    selection::{Assessment, Candidate, Decision},
};
use sha2::{Digest, Sha256};
use std::os::unix::fs::MetadataExt;
use vpnhide_core_rpc::StateSnapshot;

fn request() -> BackendChoiceRequest {
    BackendChoiceRequest {
        expected_revision: 0,
        choice: BackendChoice::Fixed {
            backend: NativeBackend::Kmod,
        },
        acknowledged_warning: None,
    }
}

fn fixture() -> (tempfile::TempDir, Arc<CoreState>, PreparedBackendChoice) {
    let root = tempfile::tempdir().unwrap();
    let bundle = root.path().join("module");
    std::fs::create_dir_all(&bundle).unwrap();
    std::fs::write(bundle.join("module.prop"), "id=vpnhide_core\n").unwrap();
    let mut manifest = vpnhide_artifact_model::fixtures::manifest();
    for payload in &mut manifest.payloads {
        let bytes = if matches!(payload.implementation, PayloadKind::Kmod { .. }) {
            vpnhide_artifact_model::fixtures::gki_contract_payload(true)
        } else {
            b"payload".to_vec()
        };
        payload.bytes = bytes.len() as u64;
        payload.sha256 = format!("{:x}", Sha256::digest(&bytes));
        let path = bundle.join(payload.implementation.relative_path().unwrap());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    let bytes = serde_json::to_vec(&manifest).unwrap();
    std::fs::write(bundle.join("payloads/manifest.json"), &bytes).unwrap();
    let catalog = Catalog::open(&bundle).unwrap();
    let metadata = std::fs::metadata(&bundle).unwrap();
    let binding = BundleIdentity {
        root: bundle,
        manifest_sha256: format!("{:x}", Sha256::digest(&bytes)),
        manifest_file_sha256: Some(format!("{:x}", Sha256::digest(&bytes))),
        source_commit: catalog.manifest.bundle_commit.clone(),
        root_identity: Some((metadata.dev(), metadata.ino())),
        wire: String::new(),
    };
    let config = CanonicalConfig::default();
    let config_path = root.path().join("config.json");
    std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let state = CoreState::load_with_journal(
        Publisher::new(1, StateSnapshot::default()),
        config_path,
        Some(root.path().join("operations.json")),
        None,
    )
    .unwrap();
    let mut env = crate::native_boot::NativeEnv::from_env(Some(root.path().join("ctl"))).unwrap();
    env.boot_id_path = root.path().join("boot");
    env.ctl = root.path().join("ctl");
    env.modules = None;
    env.zygisk_module = None;
    env.kpm_lock = None;
    env.legacy_root = None;
    env.apatch_kpms = root.path().join("apatch");
    env.state_file = root.path().join("native.json");
    std::fs::write(&env.boot_id_path, "boot-a").unwrap();
    state.install_native_boot(crate::native_boot::NativeBoot::blocked(
        env,
        1,
        "fixture".into(),
    ));
    let prepared = PreparedBackendChoice {
        config,
        preview: ChoicePreview {
            boot_id: "boot-a".into(),
            binding,
            ownership: Ownership::Clear,
            assessment: Assessment {
                kmi: crate::payloads::kmi::KmiResolution {
                    series: None,
                    release_hint: None,
                    conflicting_hints: false,
                    module_hint: None,
                    candidates: vec![],
                },
                decision: Decision::Select(NativeBackend::Kmod),
                candidates: vec![Candidate {
                    backend: NativeBackend::Kmod,
                    payload_indices: vec![],
                    eligibility: Ok(()),
                }],
            },
        },
    };
    (root, state, prepared)
}

fn execute(
    state: &Arc<CoreState>,
    op: OperationId,
    prepared: PreparedBackendChoice,
    fault: Option<(usize, bool)>,
) -> Result<Accepted, SubmitError> {
    state
        .call_writer(move |state| {
            if let Some((write, after)) = fault {
                journal::fail_write(write, after);
            }
            state.execute_backend_choice(
                op,
                BaseRef {
                    revision: state.config_revision(),
                },
                request(),
                None,
                Some(prepared),
            )
        })
        .unwrap()
}

#[test]
fn preference_and_receipt_commit_together_without_touching_config_or_runtime() {
    let (root, state, prepared) = fixture();
    let original = std::fs::read(root.path().join("config.json")).unwrap();
    let revision = state.config_revision();
    let op = state.issue_operation_id().unwrap();
    assert!(execute(&state, op, prepared, None).is_ok());
    let snapshot = state.snapshot();
    let choice = snapshot.backend_choice.unwrap();
    assert_eq!(choice.revision, 1);
    assert_eq!(choice.operation, Some(op));
    assert_eq!(choice.stage, BackendChoiceStage::PendingRestart);
    assert_eq!(snapshot.backend.selected, None);
    assert_eq!(
        std::fs::read(root.path().join("config.json")).unwrap(),
        original
    );
    assert_eq!(state.config_revision(), revision);
    assert_eq!(
        state.get_operation_record(op).unwrap().settled,
        Some(PhaseOutcome::Confirmed)
    );
    let duplicate = state
        .call_writer(move |state| {
            state.execute_backend_choice(op, BaseRef { revision }, request(), None, None)
        })
        .unwrap();
    assert!(duplicate.is_ok());
    let reused = state
        .call_writer(move |state| {
            state.execute_backend_choice(
                op,
                BaseRef { revision },
                BackendChoiceRequest {
                    choice: BackendChoice::Auto,
                    ..request()
                },
                None,
                None,
            )
        })
        .unwrap();
    assert_eq!(reused, Err(SubmitError::IdReused));
    state.shutdown();
    let journal = journal::read(&root.path().join("operations.json")).unwrap();
    assert_eq!(journal.backend_choice.unwrap().revision, 1);
}

#[test]
fn faults_before_and_after_rename_recover_without_replaying_or_false_refusal() {
    for write in [1, 2] {
        for after in [false, true] {
            let (root, state, prepared) = fixture();
            let op = state.issue_operation_id().unwrap();
            let result = execute(&state, op, prepared, Some((write, after)));
            assert_eq!(result.is_ok(), write == 2);
            assert!(state.snapshot().storage_fault.is_some());
            state.shutdown();
            let restarted = CoreState::load_with_journal(
                Publisher::new(2, StateSnapshot::default()),
                root.path().join("config.json"),
                Some(root.path().join("operations.json")),
                None,
            )
            .unwrap();
            let record = restarted.get_operation_record(op);
            let saved = journal::read(&root.path().join("operations.json")).unwrap();
            if write == 2 && after {
                assert_eq!(saved.backend_choice.unwrap().operation, op);
                assert_eq!(record.unwrap().settled, Some(PhaseOutcome::Confirmed));
            } else {
                assert!(saved.backend_choice.is_none());
                assert_eq!(
                    record.map(|r| r.settled),
                    if write == 1 && !after {
                        None
                    } else {
                        Some(Some(PhaseOutcome::FailedKnown))
                    }
                );
            }
            restarted.shutdown();
        }
    }
}

#[test]
fn stale_revision_and_replaced_bundle_are_refused_before_preference_commit() {
    for changed_bundle in [false, true] {
        let (root, state, prepared) = fixture();
        let op = state.issue_operation_id().unwrap();
        if changed_bundle {
            std::fs::write(
                prepared.preview.binding.root.join("payloads/manifest.json"),
                b"{}",
            )
            .unwrap();
        }
        let result = state
            .call_writer(move |state| {
                state.execute_backend_choice(
                    op,
                    BaseRef {
                        revision: if changed_bundle {
                            state.config_revision()
                        } else {
                            u64::MAX
                        },
                    },
                    request(),
                    None,
                    Some(prepared),
                )
            })
            .unwrap();
        assert!(result.is_err());
        assert!(
            journal::read(&root.path().join("operations.json"))
                .unwrap()
                .backend_choice
                .is_none()
        );
        assert!(state.get_operation_record(op).is_none());
        state.shutdown();
    }
}

#[test]
fn warning_acknowledgement_cannot_prepare_zygisk_without_its_projection_executor() {
    use vpnhide_core_rpc::ZygiskChoiceWarning;
    let (root, state, _) = fixture();
    for warning in [None, Some(ZygiskChoiceWarning::ProcessInjectionV1)] {
        let op = state.issue_operation_id().unwrap();
        let result = state.submit_command(
            op,
            CoreCommand::StageBackendChoice {
                base: BaseRef {
                    revision: state.config_revision(),
                },
                request: BackendChoiceRequest {
                    expected_revision: 0,
                    choice: BackendChoice::Fixed {
                        backend: NativeBackend::Zygisk,
                    },
                    acknowledged_warning: warning,
                },
            },
        );
        assert!(matches!(result, Err(SubmitError::Rejected { .. })));
        let journal = journal::read(&root.path().join("operations.json")).unwrap();
        assert!(journal.backend_choice.is_none());
        assert!(journal.get(op).is_none());
        assert!(journal.zygisk_authority.is_none());
    }
    state.shutdown();
}
