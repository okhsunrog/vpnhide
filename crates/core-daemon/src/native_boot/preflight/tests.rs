use super::*;
use crate::journal::{
    self, Journal,
    preflight::{LoaderEvidence, PreflightCompletion, PreflightState},
};
use crate::payloads::{Manifest, Payload, PayloadKind};
use sha2::{Digest, Sha256};
use std::cell::Cell;
use std::fs;

fn fixture() -> (tempfile::TempDir, Catalog, PreflightPlan) {
    let root = tempfile::tempdir().unwrap();
    let payloads = ["android13-5.15", "android14-5.15"]
        .into_iter()
        .enumerate()
        .map(|(i, kmi)| {
            let kind = PayloadKind::Kmod { kmi: kmi.into() };
            let mut bytes = vpnhide_artifact_model::fixtures::gki_contract_payload(true);
            bytes.push(i as u8);
            let path = root.path().join(kind.relative_path().unwrap());
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, &bytes).unwrap();
            Payload {
                implementation: kind,
                version: "test".into(),
                source_commit: "a".repeat(40),
                sha256: format!("{:x}", Sha256::digest(&bytes)),
                bytes: bytes.len() as u64,
                control_version: 2,
                preflight_protocol: Some(1),
                init_contract: Some(1),
            }
        })
        .collect();
    let manifest = Manifest {
        schema: 1,
        bundle_commit: "b".repeat(40),
        payloads,
    };
    fs::write(
        root.path().join("payloads/manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let catalog = Catalog::open(root.path()).unwrap();
    let plan = PreflightPlan::from_catalog(&catalog, "5.15.123-custom", "boot-a".into()).unwrap();
    (root, catalog, plan)
}
fn control() -> io::Result<Controls> {
    Ok(Controls {
        boot_id: "boot-a".into(),
        kprobes: b"unrelated probe\n".to_vec(),
    })
}
fn sentinel(_: &std::fs::File) -> io::Result<()> {
    Err(io::Error::from_raw_os_error(libc::EUCLEAN))
}

#[test]
fn final_snapshot_and_synced_intent_precede_effect_and_duplicate_never_dispatches() {
    let (root, catalog, plan) = fixture();
    let sha = &plan.candidates[0].sha256;
    let prepared = PreparedPreflight::from_catalog(&catalog, &plan, sha).unwrap();
    let source = root
        .path()
        .join(plan.candidates[0].implementation.relative_path().unwrap());
    fs::write(source, b"late replacement").unwrap();
    let path = root.path().join("journal.json");
    let mut journal = Journal::default();
    let result = prepared
        .dispatch_with(&path, &mut journal, control, |file| {
            assert!(journal::read(&path).unwrap().preflight_uncertain());
            let mut bytes = vec![0; prepared.payload().bytes as usize];
            file.read_exact_at(&mut bytes, 0).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(bytes)), *sha);
            sentinel(file)
        })
        .unwrap();
    assert_eq!(
        result,
        PreflightState::Finished(PreflightCompletion::CleanSentinel)
    );
    let mut restarted = journal::read(&path).unwrap();
    assert!(
        prepared
            .dispatch_with(&path, &mut restarted, control, |_| panic!(
                "duplicate syscall"
            ))
            .is_err()
    );
    assert!(!restarted.preflight_uncertain());
}

#[test]
fn altered_candidate_set_missing_contract_and_mutated_source_refuse_before_intent() {
    let (root, catalog, plan) = fixture();
    let sha = &plan.candidates[0].sha256;
    let mut subset = plan.clone();
    subset.candidates.pop();
    assert!(PreparedPreflight::from_catalog(&catalog, &subset, sha).is_err());
    let source = root
        .path()
        .join(plan.candidates[0].implementation.relative_path().unwrap());
    fs::write(source, b"changed after Catalog::open").unwrap();
    assert!(PreparedPreflight::from_catalog(&catalog, &plan, sha).is_err());
    let (_root, mut catalog, _) = fixture();
    catalog.manifest.payloads[0].init_contract = None;
    catalog.payloads[0].metadata.init_contract = None;
    let plan = PreflightPlan::from_catalog(&catalog, "5.15.123", "boot-a".into()).unwrap();
    assert!(PreparedPreflight::from_catalog(&catalog, &plan, &plan.candidates[0].sha256).is_err());
}

#[test]
fn control_failure_before_during_or_after_admission_never_claims_clean_sentinel() {
    for fail_at in 0..=2 {
        for changed_instead_of_unavailable in [false, true] {
            let (root, catalog, plan) = fixture();
            let prepared =
                PreparedPreflight::from_catalog(&catalog, &plan, &plan.candidates[0].sha256)
                    .unwrap();
            let mut journal = Journal::default();
            let controls = Cell::new(0);
            let effects = Cell::new(0);
            let result = prepared.dispatch_with(
                &root.path().join("journal.json"),
                &mut journal,
                || {
                    let step = controls.replace(controls.get() + 1);
                    if step == fail_at {
                        if changed_instead_of_unavailable {
                            let mut changed = control()?;
                            changed.boot_id = "boot-b".into();
                            Ok(changed)
                        } else {
                            Err(io::Error::other("observation denied"))
                        }
                    } else {
                        control()
                    }
                },
                |file| {
                    effects.set(effects.get() + 1);
                    sentinel(file)
                },
            );
            if fail_at == 0 {
                assert!(result.is_err());
                assert!(journal.kmod_preflights.is_none());
            } else {
                assert!(matches!(result.unwrap(), PreflightState::Unknown { .. }));
                assert!(journal.preflight_uncertain());
                assert!(
                    prepared
                        .dispatch_with(
                            &root.path().join("journal.json"),
                            &mut journal,
                            control,
                            |_| panic!("held retry")
                        )
                        .is_err()
                );
            }
            assert_eq!(effects.get(), usize::from(fail_at == 2));
        }
    }
}

#[test]
fn multiple_sentinels_stay_ambiguous_and_loader_policy_failure_stops_further_attempts() {
    for errno in [
        libc::EUCLEAN,
        libc::ENOEXEC,
        libc::EKEYREJECTED,
        libc::EINVAL,
        0,
    ] {
        let (root, catalog, plan) = fixture();
        let mut journal = Journal::default();
        let path = root.path().join("journal.json");
        let first =
            PreparedPreflight::from_catalog(&catalog, &plan, &plan.candidates[0].sha256).unwrap();
        let state = first
            .dispatch_with(&path, &mut journal, control, |_| {
                if errno == 0 {
                    Ok(())
                } else {
                    Err(io::Error::from_raw_os_error(errno))
                }
            })
            .unwrap();
        let second =
            PreparedPreflight::from_catalog(&catalog, &plan, &plan.candidates[1].sha256).unwrap();
        if matches!(errno, libc::EUCLEAN | libc::ENOEXEC) {
            second
                .dispatch_with(&path, &mut journal, control, sentinel)
                .unwrap();
            let evidence = journal.kmod_preflights.as_ref().unwrap().evidence();
            if errno == libc::EUCLEAN {
                assert!(
                    matches!(evidence, LoaderEvidence::MultipleSentinels { sha256 } if sha256.len() == 2)
                );
            } else {
                assert!(matches!(evidence, LoaderEvidence::OneSentinel { .. }));
            }
        } else {
            assert!(
                second
                    .dispatch_with(&path, &mut journal, control, |_| panic!(
                        "blocked next candidate"
                    ))
                    .is_err()
            );
            assert_eq!(journal.preflight_uncertain(), errno != libc::EKEYREJECTED);
            if errno == libc::EKEYREJECTED {
                assert_eq!(
                    state,
                    PreflightState::Finished(PreflightCompletion::LoaderBlocked { errno })
                );
            }
        }
        assert!(
            journal.native_load_budget.is_none(),
            "experiment never consumes or grants a real load"
        );
    }
}

#[test]
fn storage_failures_and_late_kprobe_changes_keep_the_durable_hold() {
    for (skip, after_rename) in [(1, false), (1, true), (2, false), (2, true)] {
        let (root, catalog, plan) = fixture();
        let prepared =
            PreparedPreflight::from_catalog(&catalog, &plan, &plan.candidates[0].sha256).unwrap();
        let mut journal = Journal::default();
        let effects = Cell::new(0);
        journal::fail_write(skip, after_rename);
        assert!(
            prepared
                .dispatch_with(
                    &root.path().join("journal.json"),
                    &mut journal,
                    control,
                    |file| {
                        effects.set(effects.get() + 1);
                        sentinel(file)
                    }
                )
                .is_err()
        );
        assert_eq!(effects.get(), usize::from(skip == 2));
        assert!(journal.preflight_uncertain());
    }
    let (root, catalog, plan) = fixture();
    let prepared =
        PreparedPreflight::from_catalog(&catalog, &plan, &plan.candidates[0].sha256).unwrap();
    let mut journal = Journal::default();
    let count = Cell::new(0);
    let state = prepared
        .dispatch_with(
            &root.path().join("journal.json"),
            &mut journal,
            || {
                let mut observed = control()?;
                if count.replace(count.get() + 1) == 2 {
                    observed.kprobes.extend_from_slice(b"new probe\n");
                }
                Ok(observed)
            },
            sentinel,
        )
        .unwrap();
    assert_eq!(
        state,
        PreflightState::Unknown {
            errno: Some(libc::EUCLEAN)
        }
    );
    assert!(journal.preflight_uncertain());
}
