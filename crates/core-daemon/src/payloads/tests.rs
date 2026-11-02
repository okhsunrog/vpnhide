use super::selection::*;
use super::*;
use std::fs;
use std::os::unix::fs::symlink;
use vpnhide_core_rpc::NativeBackend as B;

fn payload(implementation: PayloadKind) -> Payload {
    Payload {
        implementation,
        version: "test".into(),
        source_commit: "a".repeat(40),
        sha256: format!("{:x}", Sha256::digest(b"payload")),
        bytes: 7,
        control_version: 2,
        preflight_protocol: None,
        init_contract: None,
    }
}
pub(super) fn manifest() -> Manifest {
    let mut payloads: Vec<_> = KMIS
        .iter()
        .map(|kmi| payload(PayloadKind::Kmod { kmi: (*kmi).into() }))
        .collect();
    payloads.extend([
        payload(PayloadKind::Kpm),
        payload(PayloadKind::Zygisk { abi: Abi::Arm64 }),
        payload(PayloadKind::Zygisk { abi: Abi::Arm32 }),
    ]);
    Manifest {
        schema: 1,
        bundle_commit: "b".repeat(40),
        payloads,
    }
}
fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let manifest = manifest();
    for p in &manifest.payloads {
        let path = root.path().join(p.implementation.relative_path().unwrap());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"payload").unwrap();
    }
    fs::write(
        root.path().join("payloads/manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    root
}
#[test]
fn complete_manifest_roundtrip_and_pinned_bytes() {
    let root = fixture();
    let mut catalog = Catalog::open(root.path()).unwrap();
    catalog.manifest.require_complete().unwrap();
    let file = &mut catalog.payloads[0];
    let path = root
        .path()
        .join(file.metadata.implementation.relative_path().unwrap());
    fs::rename(&path, path.with_extension("old")).unwrap();
    fs::write(path, b"replacement").unwrap();
    let mut bytes = Vec::new();
    file.file.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"payload");
}

#[test]
fn catalog_rechecks_advertised_capability_against_the_exact_hashed_elf() {
    let root = fixture();
    let mut manifest = manifest();
    manifest.payloads[0].preflight_protocol = Some(1);
    // Digest-valid old payloads still cannot inherit a new advertised protocol.
    fs::write(
        root.path().join("payloads/manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    assert!(Catalog::open(root.path()).is_err());
    let elf = super::elf::tests::fixture(
        b"vpnhide_preflight=1\0parmtype=preflight_only:bool\0",
        false,
    );
    let payload = &mut manifest.payloads[0];
    payload.bytes = elf.len() as u64;
    payload.sha256 = format!("{:x}", Sha256::digest(&elf));
    fs::write(
        root.path()
            .join(payload.implementation.relative_path().unwrap()),
        elf,
    )
    .unwrap();
    fs::write(
        root.path().join("payloads/manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    assert!(Catalog::open(root.path()).is_ok());
    manifest.payloads.last_mut().unwrap().preflight_protocol = Some(1);
    assert!(manifest.validate().is_err());
}
#[test]
fn invalid_metadata_and_duplicates_fail_closed() {
    let valid = manifest();
    let mut cases = Vec::new();
    let mut m = valid.clone();
    m.schema = 2;
    cases.push(m);
    let mut m = valid.clone();
    m.payloads.push(m.payloads[0].clone());
    cases.push(m);
    let mut m = valid.clone();
    m.payloads[0].bytes = u64::MAX;
    cases.push(m);
    let mut m = valid.clone();
    m.payloads[0].sha256 = "no".into();
    cases.push(m);
    let mut m = valid.clone();
    m.payloads[0].control_version = 3;
    cases.push(m);
    let mut m = valid.clone();
    m.payloads[0].source_commit.clear();
    cases.push(m);
    let mut m = valid.clone();
    m.payloads[0].implementation = PayloadKind::Kmod {
        kmi: "../../outside".into(),
    };
    cases.push(m);
    for m in cases {
        assert!(m.validate().is_err(), "{m:?}");
    }
    let mut m = valid;
    m.payloads.pop();
    assert!(m.require_complete().is_err());
    assert!(Manifest::parse(&vec![b' '; MAX_MANIFEST_BYTES as usize + 1]).is_err());
    let json = serde_json::to_string(&manifest()).unwrap();
    assert!(Manifest::parse(json.replacen('{', "{\"unknown\":true,", 1).as_bytes()).is_err());
    assert!(Manifest::parse(json.replacen('{', "{\"schema\":1,", 1).as_bytes()).is_err());
}
#[test]
fn bounded_total_and_empty_files() {
    let mut m = manifest();
    m.payloads[0].bytes = 0;
    assert!(m.validate().is_err());
    for p in &mut m.payloads {
        p.bytes = MAX_PAYLOAD_BYTES;
    }
    assert!(m.validate().is_err());
}
#[test]
fn reject_corruption_missing_and_nonregular_payloads() {
    for variant in 0..4 {
        let root = fixture();
        let path = root.path().join("payloads/kpm/arm64/vpnhide.kpm");
        match variant {
            0 => fs::write(&path, b"changed").unwrap(),
            1 => fs::remove_file(&path).unwrap(),
            2 => {
                fs::remove_file(&path).unwrap();
                fs::create_dir(&path).unwrap();
            }
            _ => {
                fs::write(&path, b"longer payload").unwrap();
            }
        }
        assert!(Catalog::open(root.path()).is_err());
    }
}
#[test]
fn reject_leaf_parent_and_manifest_symlinks() {
    for relative in [
        "payloads/kpm/arm64/vpnhide.kpm",
        "payloads/kpm",
        "payloads/manifest.json",
    ] {
        let root = fixture();
        let path = root.path().join(relative);
        let target = path.with_extension("real");
        fs::rename(&path, &target).unwrap();
        symlink(&target, &path).unwrap();
        assert!(Catalog::open(root.path()).is_err(), "{relative}");
    }
}
fn device() -> Capabilities {
    Capabilities {
        abi: Abi::Arm64,
        kernel_release: "6.1.157-android14".into(),
        builtin: Runtime::Absent,
        module_loading: Runtime::Ready,
        kpm: Runtime::Ready,
        zygisk: Runtime::Ready,
        needs_arm32_zygisk: true,
        ownership: Ownership::Clear,
    }
}
fn policy() -> Policy {
    Policy {
        resolved: true,
        targets: 160,
        wire_bytes: 1023,
        default_mask: 0,
        required_hooks: 3,
    }
}
fn support() -> Vec<(B, Support)> {
    [B::Builtin, B::Kmod, B::Kpm, B::Zygisk]
        .map(|b| {
            (
                b,
                Support {
                    hooks: 3,
                    target_capacity: 160,
                    wire_capacity: 1023,
                    default_mask: b != B::Zygisk,
                },
            )
        })
        .to_vec()
}
#[test]
fn bundle_multiplicity_is_not_a_conflict_and_priority_is_stable() {
    let mut d = device();
    for (expected, disable) in [
        (B::Builtin, None),
        (B::Kmod, Some(B::Builtin)),
        (B::Kpm, Some(B::Kmod)),
        (B::Zygisk, Some(B::Kpm)),
    ] {
        if disable.is_none() {
            d.builtin = Runtime::Ready;
        }
        match disable {
            Some(B::Builtin) => d.builtin = Runtime::Absent,
            Some(B::Kmod) => d.module_loading = Runtime::Unsupported,
            Some(B::Kpm) => d.kpm = Runtime::AuthenticationRequired,
            _ => {}
        }
        let mut m = manifest();
        m.payloads.reverse();
        assert_eq!(
            assess(&m, &d, policy(), &support()).unwrap().decision,
            Decision::Select(expected)
        );
    }
}
#[test]
fn residual_effects_block_selection_even_with_all_payloads() {
    for owner in [
        Ownership::Unknown,
        Ownership::Foreign,
        Ownership::Resident(B::Kmod),
        Ownership::Resident(B::Builtin),
        Ownership::Resident(B::Kpm),
        Ownership::Resident(B::Zygisk),
    ] {
        let mut d = device();
        d.ownership = owner;
        let expected = match owner {
            Ownership::Resident(b) => Decision::ReconcileResident(b),
            _ => Decision::BlockedOwnership,
        };
        assert_eq!(
            assess(&manifest(), &d, policy(), &support())
                .unwrap()
                .decision,
            expected
        );
    }
}
#[test]
fn policy_is_never_truncated_or_invented() {
    for reason in [
        Rejection::PolicyUnresolved,
        Rejection::TargetCapacity,
        Rejection::WireCapacity,
        Rejection::RequiredHooksMissing,
    ] {
        let mut p = policy();
        match reason {
            Rejection::PolicyUnresolved => p.resolved = false,
            Rejection::TargetCapacity => p.targets += 1,
            Rejection::WireCapacity => p.wire_bytes += 1,
            _ => p.required_hooks = 4,
        }
        let report = assess(&manifest(), &device(), p, &support()).unwrap();
        assert_eq!(report.decision, Decision::NoEligibleCandidate);
        assert_eq!(report.candidates[1].eligibility, Err(reason));
    }
}
#[test]
fn zygisk_requires_all_needed_abis_and_rejects_default_mask() {
    let mut d = device();
    d.module_loading = Runtime::Absent;
    d.kpm = Runtime::Absent;
    let mut m = manifest();
    m.payloads.pop();
    assert_eq!(
        assess(&m, &d, policy(), &support()).unwrap().decision,
        Decision::NoEligibleCandidate
    );
    d.needs_arm32_zygisk = false;
    assert_eq!(
        assess(&m, &d, policy(), &support()).unwrap().decision,
        Decision::Select(B::Zygisk)
    );
    let mut p = policy();
    p.default_mask = 1;
    assert_eq!(
        assess(&m, &d, p, &support()).unwrap().candidates[3].eligibility,
        Err(Rejection::DefaultMaskUnsupported)
    );
}
#[test]
fn runtime_and_kmi_uncertainty_are_not_capabilities() {
    let mut d = device();
    d.kernel_release.clear();
    d.kpm = Runtime::Unknown;
    d.zygisk = Runtime::AuthenticationRequired;
    let r = assess(&manifest(), &d, policy(), &support()).unwrap();
    assert_eq!(r.decision, Decision::NoEligibleCandidate);
    assert_eq!(r.candidates[1].eligibility, Err(Rejection::UnknownKmi));
    assert_eq!(r.candidates[2].eligibility, Err(Rejection::RuntimeUnknown));
    d.kernel_release = "6.18.0-android17".into();
    assert_eq!(
        assess(&manifest(), &d, policy(), &support())
            .unwrap()
            .candidates[1]
            .eligibility,
        Err(Rejection::NoMatchingKmi)
    );
}

#[test]
fn ambiguous_kmi_is_never_selected_by_uname_hint_or_catalog_order() {
    let mut device = device();
    device.kernel_release = "5.15.104-android14-custom".into();
    for reverse in [false, true] {
        let mut manifest = manifest();
        if reverse {
            manifest.payloads.reverse();
        }
        let report = assess(&manifest, &device, policy(), &support()).unwrap();
        assert_eq!(report.kmi.candidates.len(), 2);
        assert_eq!(report.candidates[1].payload_indices.len(), 2);
        assert_eq!(
            report.candidates[1].eligibility,
            Err(Rejection::AmbiguousKmi)
        );
        assert_eq!(report.decision, Decision::Select(B::Kpm));
        device.ownership = Ownership::Resident(B::Kmod);
        let report = assess(&manifest, &device, policy(), &support()).unwrap();
        assert!(!super::device::resident_eligible(&report, B::Kmod));
        device.ownership = Ownership::Clear;
    }
    device.kernel_release = "6.1.157".into();
    assert_eq!(
        assess(&manifest(), &device, policy(), &support())
            .unwrap()
            .decision,
        Decision::Select(B::Kmod)
    );
    for release in ["6.1.157-android13", "6.1.157-android14-android14"] {
        device.kernel_release = release.into();
        assert_eq!(
            assess(&manifest(), &device, policy(), &support())
                .unwrap()
                .candidates[1]
                .eligibility,
            Err(Rejection::ConflictingKmiEvidence)
        );
    }
}
#[test]
fn arm32_and_missing_evidence_fail_closed() {
    let mut d = device();
    d.abi = Abi::Arm32;
    let r = assess(&manifest(), &d, policy(), &support()).unwrap();
    assert_eq!(
        r.candidates[1].eligibility,
        Err(Rejection::WrongArchitecture)
    );
    assert_eq!(
        r.candidates[2].eligibility,
        Err(Rejection::WrongArchitecture)
    );
    assert_eq!(r.decision, Decision::Select(B::Zygisk));
    assert_eq!(
        assess(&manifest(), &d, policy(), &[]).unwrap().decision,
        Decision::NoEligibleCandidate
    );
}

#[test]
fn python_assembled_bundle_matches_runtime_catalog_with_inert_autoload() {
    let root = tempfile::tempdir().unwrap();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/test_payload_bundle.py");
    let status = std::process::Command::new("python3")
        .arg(fixture)
        .arg("--fixture")
        .arg(root.path())
        .status()
        .unwrap();
    assert!(status.success());
    let catalog = Catalog::open(root.path()).unwrap();
    catalog.manifest.require_complete().unwrap();
    assert_eq!(catalog.payloads.len(), 10);
    assert!(crate::zygisk::ZygiskTarget::open(root.path()).is_ok());
    assert!(!root.path().join("zygisk-activation.txt").exists());
    assert!(!root.path().join("targets.txt").exists());
    assert_eq!(catalog.manifest.bundle_commit, "a".repeat(40));
}

#[test]
fn candidate_specific_projections_preserve_distinct_hook_spaces_and_refuse_gaps() {
    let mut device = device();
    device.module_loading = Runtime::Unsupported;
    device.kpm = Runtime::Absent;
    let mut zygisk = policy();
    zygisk.required_hooks = vpnhide_protocol::hook_ids::Hook::ZygiskIoctl.bit();
    let mut support = support();
    support
        .iter_mut()
        .find(|(b, _)| *b == B::Zygisk)
        .unwrap()
        .1
        .hooks = zygisk.required_hooks;
    let mut projections = vec![(B::Kmod, policy()), (B::Zygisk, zygisk)];
    assert_eq!(
        assess_projected(&manifest(), &device, &projections, &support)
            .unwrap()
            .decision,
        Decision::Select(B::Zygisk)
    );
    assert_eq!(
        assess(&manifest(), &device, policy(), &support)
            .unwrap()
            .decision,
        Decision::NoEligibleCandidate
    );
    projections.push((B::Zygisk, zygisk));
    let duplicate = assess_projected(&manifest(), &device, &projections, &support).unwrap();
    assert_eq!(
        duplicate.candidates[3].eligibility,
        Err(Rejection::PolicyUnresolved)
    );
    projections.pop();
    projections[1].1.default_mask = zygisk.required_hooks;
    assert_eq!(
        assess_projected(&manifest(), &device, &projections, &support)
            .unwrap()
            .candidates[3]
            .eligibility,
        Err(Rejection::DefaultMaskUnsupported)
    );
    projections.pop();
    assert_eq!(
        assess_projected(&manifest(), &device, &projections, &support)
            .unwrap()
            .candidates[3]
            .eligibility,
        Err(Rejection::PolicyUnresolved)
    );
}
