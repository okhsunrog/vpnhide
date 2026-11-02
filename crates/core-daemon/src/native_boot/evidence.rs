//! Typed rejection evidence is independent of human-readable loader diagnostics.
//! Only this exact prepared load and a fresh clear-ownership observation may
//! discharge its uncertainty; they never authorize a neighboring KMI.
use super::prepared::{PayloadIdentity, TransitionAction, TransitionIntent};
use serde::{Deserialize, Serialize};
use vpnhide_activator::KmodLoadOrigin;
use vpnhide_core_rpc::{NativeBackend, NativeBootOutcome, ZygiskTargets};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CapturedFailure {
    Kmod { origin: KmodLoadOrigin, code: i32 },
    KpmBeforeDispatch,
    KpmAfterDispatch,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RejectionCause {
    KmodBeforeDispatch { code: i32 },
    KpmBeforeDispatch,
    ArtifactAbsent,
    AndroidSignatureRejected { errno: i32 },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RejectionProof {
    pub boot_id: String,
    pub payload: PayloadIdentity,
    pub manifest_sha256: String,
    pub source_commit: String,
    pub cause: RejectionCause,
    /// Actual BOOTTIME after checking kernel/KPM/legacy absence in this boot.
    pub ownership_checked_boot_ms: u64,
}
impl RejectionProof {
    pub(crate) fn valid_for(&self, attempt: &crate::journal::NativeStartup) -> bool {
        let Some(intent) = attempt.intent.as_ref() else {
            return false;
        };
        let Some(bundle) = intent.bundle.as_ref() else {
            return false;
        };
        let Some(result) = attempt.completed.as_ref() else {
            return false;
        };
        if self.boot_id != attempt.boot_id
            || intent.payload.as_ref() != Some(&self.payload)
            || self.manifest_sha256 != bundle.manifest_sha256
            || self.source_commit != bundle.source_commit
            || self.ownership_checked_boot_ms == 0
            || result.selection.is_some()
            || matches!(
                result.status.zygisk_targets,
                ZygiskTargets::RetireFailed { .. }
            )
        {
            return false;
        }
        match (intent.action, &result.status.outcome, self.cause) {
            (
                TransitionAction::Load(NativeBackend::Kmod),
                NativeBootOutcome::LoadFailed {
                    backend: NativeBackend::Kmod,
                    exit: Some(code),
                    ..
                },
                RejectionCause::KmodBeforeDispatch { code: expected },
            ) => *code == expected,
            (
                TransitionAction::Load(NativeBackend::Kpm),
                NativeBootOutcome::LoadFailed {
                    backend: NativeBackend::Kpm,
                    ..
                },
                RejectionCause::KpmBeforeDispatch,
            ) => true,
            (
                TransitionAction::Load(NativeBackend::Kmod),
                NativeBootOutcome::LoadFailed {
                    backend: NativeBackend::Kmod,
                    exit: Some(code),
                    ..
                },
                RejectionCause::AndroidSignatureRejected { errno },
            ) => {
                *code == errno
                    && matches!(errno, libc::ENOKEY | libc::EKEYREJECTED)
                    && self.payload.init_contract == Some(1)
            }
            (
                TransitionAction::Load(backend),
                NativeBootOutcome::MissingArtifact {
                    backend: observed, ..
                },
                RejectionCause::ArtifactAbsent,
            ) => backend == *observed,
            _ => false,
        }
    }
}
fn cause(
    failure: Option<CapturedFailure>,
    contract: Option<u32>,
    outcome: &NativeBootOutcome,
) -> Option<RejectionCause> {
    match failure {
        Some(CapturedFailure::Kmod {
            origin: KmodLoadOrigin::NotDispatched,
            code,
        }) => Some(RejectionCause::KmodBeforeDispatch { code }),
        Some(CapturedFailure::Kmod {
            origin: KmodLoadOrigin::AndroidFinitModuleErrno,
            code,
        }) if contract == Some(1) && matches!(code, libc::ENOKEY | libc::EKEYREJECTED) => {
            Some(RejectionCause::AndroidSignatureRejected { errno: code })
        }
        Some(CapturedFailure::KpmBeforeDispatch) => Some(RejectionCause::KpmBeforeDispatch),
        None if matches!(outcome, NativeBootOutcome::MissingArtifact { .. }) => {
            Some(RejectionCause::ArtifactAbsent)
        }
        _ => None,
    }
}
/// No classification by exit number, stderr, dmesg or absence alone.
pub(super) fn rejection(
    intent: &TransitionIntent,
    boot: &super::NativeBoot,
    failure: Option<CapturedFailure>,
) -> Option<RejectionProof> {
    let payload = intent.payload.as_ref()?;
    let bundle = intent.bundle.as_ref()?;
    let cause = cause(failure, payload.init_contract, &boot.status.outcome)?;
    if boot.selection.is_some() || !ownership_clear(&boot.env, &intent.boot_id) {
        return None;
    }
    let mut time: libc::timespec = unsafe { std::mem::zeroed() };
    if unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut time) } != 0 || time.tv_sec < 0 {
        return None;
    }
    let ownership_checked_boot_ms = time.tv_sec as u64 * 1000 + time.tv_nsec as u64 / 1_000_000;
    (ownership_checked_boot_ms > 0).then(|| RejectionProof {
        boot_id: intent.boot_id.clone(),
        payload: payload.clone(),
        manifest_sha256: bundle.manifest_sha256.clone(),
        source_commit: bundle.source_commit.clone(),
        cause,
        ownership_checked_boot_ms,
    })
}
pub(super) fn module_absent() -> bool {
    matches!(std::fs::symlink_metadata("/sys/module/vpnhide_kmod"), Err(error)
        if error.kind() == std::io::ErrorKind::NotFound)
}
pub(super) fn ownership_clear(env: &super::NativeEnv, boot: &str) -> bool {
    use crate::payloads::selection::Ownership;
    if super::read_boot_id(&env.boot_id_path) != boot {
        return false;
    }
    if !module_absent() {
        return false;
    }
    let probe = super::probe_kpm(env.kpm_lock.as_deref());
    let facts = super::gather(env, probe.fact);
    crate::payloads::device::resident_ownership(
        &facts,
        env.modules
            .as_deref()
            .is_some_and(|p| !super::obsolete_modules(p).is_empty()),
    ) == Ownership::Clear
}

#[cfg(test)]
mod tests {
    use super::*;
    fn outcome() -> NativeBootOutcome {
        NativeBootOutcome::LoadFailed {
            backend: NativeBackend::Kmod,
            exit: Some(129),
            stderr: "Key was rejected by service".into(),
            dmesg_tail: vec!["module verification failed".into()],
            kprobes: None,
            kretprobes: None,
            signature_rejected: true,
        }
    }
    #[test]
    fn numeric_status_and_diagnostic_signature_text_never_prove_no_effect() {
        for code in [126, 129] {
            assert_eq!(
                cause(
                    Some(CapturedFailure::Kmod {
                        origin: KmodLoadOrigin::HostProcessStatus,
                        code
                    }),
                    Some(1),
                    &outcome()
                ),
                None
            );
            assert_eq!(
                cause(
                    Some(CapturedFailure::Kmod {
                        origin: KmodLoadOrigin::AndroidFinitModuleErrno,
                        code
                    }),
                    None,
                    &outcome()
                ),
                None
            );
            assert!(matches!(
                cause(
                    Some(CapturedFailure::Kmod {
                        origin: KmodLoadOrigin::AndroidFinitModuleErrno,
                        code
                    }),
                    Some(1),
                    &outcome()
                ),
                Some(RejectionCause::AndroidSignatureRejected { .. })
            ));
        }
        assert_eq!(cause(None, Some(1), &outcome()), None);
        assert_eq!(
            cause(Some(CapturedFailure::KpmAfterDispatch), Some(1), &outcome()),
            None
        );
        assert_eq!(
            cause(
                Some(CapturedFailure::Kmod {
                    origin: KmodLoadOrigin::AndroidFinitModuleErrno,
                    code: 117
                }),
                Some(1),
                &outcome()
            ),
            None
        );
    }
    fn attempt() -> crate::journal::NativeStartup {
        use super::super::prepared::BundleIdentity;
        let payload = PayloadIdentity {
            source: "/bundle/payload.ko".into(),
            sha256: "a".repeat(64),
            bytes: 100,
            init_contract: Some(1),
        };
        let proof = RejectionProof {
            boot_id: "boot".into(),
            payload: payload.clone(),
            manifest_sha256: "b".repeat(64),
            source_commit: "c".repeat(40),
            cause: RejectionCause::AndroidSignatureRejected { errno: 129 },
            ownership_checked_boot_ms: 1,
        };
        crate::journal::NativeStartup {
            boot_id: "boot".into(),
            config_revision: 1,
            filesystem_hiding: false,
            repair_op: None,
            intent: Some(TransitionIntent {
                boot_id: "boot".into(),
                filesystem_hiding: false,
                action: TransitionAction::Load(NativeBackend::Kmod),
                payload: Some(payload),
                bundle: Some(BundleIdentity {
                    root: "/bundle".into(),
                    manifest_sha256: "b".repeat(64),
                    source_commit: "c".repeat(40),
                    root_identity: Some((1, 2)),
                    wire: "vpnhide 2 config\ndebug 0\ndefault 0\nend 0\n".into(),
                }),
                retire_zygisk: None,
                retire_directory_identity: None,
            }),
            completed: Some(crate::journal::NativeStartupResult {
                status: vpnhide_core_rpc::NativeBootStatus {
                    boot_id: "boot".into(),
                    decided_epoch: 1,
                    decided_at_ms: 1,
                    updated_at_ms: 1,
                    outcome: outcome(),
                    installed: vec![],
                    live: vec![],
                    staged: vec![],
                    kmod: None,
                    zygisk_targets: ZygiskTargets::NotInstalled,
                    ports: None,
                    obsolete_modules: vec![],
                },
                selection: None,
                rejection: Some(proof),
            }),
        }
    }
    #[test]
    fn proof_is_bound_to_the_exact_durable_attempt_and_old_signature_reports_stay_unknown() {
        let original = attempt();
        assert!(!original.uncertain());
        for field in 0..7 {
            let mut changed = original.clone();
            let proof = changed
                .completed
                .as_mut()
                .unwrap()
                .rejection
                .as_mut()
                .unwrap();
            match field {
                0 => proof.boot_id = "different".into(),
                1 => proof.payload.sha256 = "d".repeat(64),
                2 => proof.manifest_sha256 = "d".repeat(64),
                3 => proof.source_commit = "d".repeat(40),
                4 => proof.payload.init_contract = None,
                5 => proof.cause = RejectionCause::AndroidSignatureRejected { errno: 126 },
                _ => proof.ownership_checked_boot_ms = 0,
            }
            assert!(changed.uncertain());
        }
        let mut old = original.clone();
        old.completed.as_mut().unwrap().rejection = None;
        assert!(old.uncertain());
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("journal.json");
        let mut journal = crate::journal::Journal {
            native_startup: Some(old.clone()),
            ..Default::default()
        };
        let mut json = serde_json::to_value(&journal).unwrap();
        json["version"] = 10.into();
        json["native_startup"]["intent"]["payload"]
            .as_object_mut()
            .unwrap()
            .remove("init_contract");
        json["native_startup"]["completed"]
            .as_object_mut()
            .unwrap()
            .remove("rejection");
        std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        let migrated = crate::journal::read(&path).unwrap();
        assert!(migrated.native_startup_uncertain());
        journal.native_startup = Some(original);
        crate::journal::write_atomic(&path, &journal).unwrap();
        assert!(
            !crate::journal::read(&path)
                .unwrap()
                .native_startup_uncertain()
        );
        journal
            .native_startup
            .as_mut()
            .unwrap()
            .completed
            .as_mut()
            .unwrap()
            .rejection
            .as_mut()
            .unwrap()
            .payload
            .bytes += 1;
        crate::journal::write_atomic(&path, &journal).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(crate::journal::read(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}
