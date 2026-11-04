use super::*;
use crate::native_boot::prepared::{
    BundleIdentity, PayloadIdentity, TransitionAction, TransitionIntent,
};
use vpnhide_core_rpc::{NativeSelectionDecision, NativeSelectionEvidence};

pub(crate) fn fixture() -> NativeSelectionEvidence {
    NativeSelectionEvidence {
        boot_id: "test".into(),
        observed_at_boot_ms: Some(7),
        kernel_release: "5.15.123-custom".into(),
        kernel_machine: "aarch64".into(),
        core_abi: "arm64".into(),
        manifest_sha256: "a".repeat(64),
        manifest_file_sha256: "c".repeat(64),
        source_commit: "b".repeat(40),
        candidate_kmis: vec!["android13-5.15".into(), "android14-5.15".into()],
        uname_hint: None,
        module_hint: None,
        conflicting_hints: true,
        modules: vec![],
        inventory_complete: true,
        unbound_files: 0,
        decision: NativeSelectionDecision::NoEligibleCandidate,
    }
}
fn intent(evidence: Option<NativeSelectionEvidence>) -> TransitionIntent {
    TransitionIntent {
        selection_evidence: evidence,
        boot_id: "test".into(),
        filesystem_hiding: false,
        action: TransitionAction::Load(NativeBackend::Kmod),
        payload: Some(PayloadIdentity {
            source: "/verified/vpnhide.ko".into(),
            sha256: "c".repeat(64),
            bytes: 100,
            init_contract: Some(1),
        }),
        bundle: Some(BundleIdentity {
            manifest_file_sha256: Some("c".repeat(64)),
            root: "/verified".into(),
            manifest_sha256: "a".repeat(64),
            source_commit: "b".repeat(40),
            root_identity: None,
            wire: "vpnhide 2 config\ndebug 0\nend 0\n".into(),
        }),
        retire_zygisk: None,
        retire_directory_identity: None,
    }
}

#[test]
fn incomplete_observation_and_receipt_pruning_cannot_erase_current_boot_conflict() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("journal.json");
    let mut journal = Journal {
        issued_through: 99,
        expired_through: 80,
        expired_holds: vec![ConfigPhase::Ports],
        native_load_budget: Some(NativeLoadBudget {
            boot_id: "test".into(),
            attempted: vec![NativeBackend::Kpm],
        }),
        ..Default::default()
    };
    let contrary = fixture();
    journal.record_selection_evidence(contrary.clone());
    let mut incomplete = contrary.clone();
    incomplete.observed_at_boot_ms = Some(8);
    incomplete.conflicting_hints = false;
    incomplete.inventory_complete = false;
    journal.record_selection_evidence(incomplete.clone());
    journal.prune();
    write_atomic(&path, &journal).unwrap();
    let mut recovered = read(&path).unwrap();
    recovered.fence_unresolved();
    assert_eq!(
        recovered.native_selection_evidence,
        Some(incomplete.clone())
    );
    assert_eq!(recovered.native_selection_conflict, Some(contrary));
    assert!(
        recovered
            .check_native_safety(&intent(Some(incomplete)))
            .is_err()
    );
    assert!(recovered.check_native_safety(&intent(None)).is_err());
    assert_eq!(recovered.expired_holds, vec![ConfigPhase::Ports]);
    assert_eq!(
        recovered.native_load_budget.as_ref().unwrap().attempted,
        vec![NativeBackend::Kpm]
    );
    let mut qualified = fixture();
    qualified.conflicting_hints = false;
    qualified.module_hint = Some("android13-5.15".into());
    assert!(
        recovered
            .check_native_safety(&intent(Some(qualified.clone())))
            .is_err()
    );
    qualified.modules = [("wifi", 'a'), ("ksu_next", 'b')]
        .into_iter()
        .map(|(name, hash)| vpnhide_core_rpc::NativeModuleEvidence {
            name: name.into(),
            sha256: hash.to_string().repeat(64),
            source: format!("/vendor_dlkm/lib/modules/{name}.ko"),
            vermagic: "5.15.123-android13 SMP aarch64".into(),
            srcversion: None,
            build_id: "01020304".into(),
        })
        .collect();
    assert!(
        recovered
            .check_native_safety(&intent(Some(qualified.clone())))
            .is_ok()
    );
    recovered.record_selection_evidence(qualified.clone());
    write_atomic(&path, &recovered).unwrap();
    let cleared = read(&path).unwrap();
    assert!(cleared.native_selection_conflict.is_none());
    assert_eq!(cleared.native_selection_evidence, Some(qualified));
    assert_eq!(cleared.expired_holds, vec![ConfigPhase::Ports]);
}

#[test]
fn old_journal_keeps_holds_and_budgets_but_cannot_claim_new_evidence() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("journal.json");
    let mut old = Journal {
        version: 14,
        issued_through: 44,
        expired_through: 40,
        expired_holds: vec![ConfigPhase::Native],
        native_load_budget: Some(NativeLoadBudget {
            boot_id: "test".into(),
            attempted: vec![NativeBackend::Kmod],
        }),
        ..Default::default()
    };
    write_atomic(&path, &old).unwrap();
    let migrated = read(&path).unwrap();
    assert_eq!(migrated.version, JOURNAL_VERSION);
    assert_eq!(migrated.held_resources(), vec![ConfigPhase::Native]);
    assert_eq!(migrated.native_load_budget, old.native_load_budget);
    assert_eq!(migrated.expired_through, 40);
    old.record_selection_evidence(fixture());
    write_atomic(&path, &old).unwrap();
    let bytes = fs::read(&path).unwrap();
    assert!(read(&path).is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    old.version = JOURNAL_VERSION + 1;
    write_atomic(&path, &old).unwrap();
    assert!(read(&path).is_err());
}

#[test]
fn evidence_must_bind_the_same_boot_and_catalog_before_native_admission() {
    let mut valid = intent(Some(fixture()));
    assert!(valid.valid_selection_evidence());
    valid.selection_evidence.as_mut().unwrap().boot_id = "other-boot".into();
    assert!(!valid.valid_selection_evidence());
    valid.selection_evidence.as_mut().unwrap().boot_id = "test".into();
    valid.bundle.as_mut().unwrap().manifest_sha256 = "d".repeat(64);
    assert!(Journal::default().check_native_safety(&valid).is_err());
}
