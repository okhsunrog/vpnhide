//! Writer-owned transition effects shared by startup, repair and reset. Each
//! effect has a separate durable boundary; a receipt cannot redirect its target.
use crate::{
    journal::{
        Journal, ZygiskAuthorityRecord,
        revocation::{RevocationState, ZygiskRevocation},
    },
    zygisk::{PublishError, ZygiskTarget},
};
use std::io;
use vpnhide_core_rpc::{OperationId, PhaseOutcome};

pub(crate) fn revoke(
    journal: &mut Journal,
    expected: &ZygiskAuthorityRecord,
    operation: Option<OperationId>,
    current_boot: &str,
    persist: &mut impl FnMut(&Journal) -> io::Result<()>,
    can_dispatch: impl Fn() -> bool,
) -> Result<(), PhaseOutcome> {
    let preflight = || -> Result<ZygiskTarget, String> {
        let reference = expected
            .reference()
            .ok_or("invalid authorization identity")?;
        if (journal.zygisk_authority.as_ref() != Some(expected)
            && journal.prior_zygisk_authority.as_ref() != Some(expected))
            || current_boot.is_empty()
            || current_boot.len() > 128
        {
            return Err("authorization source changed".into());
        }
        if journal
            .zygisk_revocation
            .as_ref()
            .is_some_and(|r| r.uncertain() && r.authority != *expected)
        {
            return Err("another authorization revocation is unresolved".into());
        }
        let target = ZygiskTarget::for_revocation(
            expected
                .intent
                .zygisk_module
                .as_deref()
                .ok_or("missing recipient")?,
        )?;
        if target.revalidate_identity()? != (reference.directory_device, reference.directory_inode)
            || !target.is_bundled()?
        {
            return Err("authorization recipient changed".into());
        }
        target.check_authorization(expected)?;
        Ok(target)
    };
    let target = preflight().map_err(|error| {
        crate::alog::info(&format!("Zygisk revocation preparation refused: {error}"));
        PhaseOutcome::FailedKnown
    })?;
    let previous = journal
        .zygisk_revocation
        .as_ref()
        .filter(|record| record.authority == *expected)
        .and_then(|record| record.operation);
    if let (Some(previous), Some(next)) = (previous, operation)
        && previous != next
        && let Some(entry) = journal.get_mut(previous)
        && entry.is_revocation_only_repair()
        && entry
            .held_phases()
            .contains(&vpnhide_core_rpc::ConfigPhase::Native)
    {
        // Transfer only this exact grant's unresolved hold to the new Running
        // intent in the SAME durable write. Historical Unknown remains unchanged.
        entry
            .superseded_phases
            .push(vpnhide_core_rpc::ConfigPhase::Native);
        entry.superseded_by = Some(next);
    }
    let record = ZygiskRevocation {
        operation,
        current_boot: current_boot.into(),
        authority: expected.clone(),
        state: RevocationState::Running,
    };
    journal.zygisk_revocation = Some(record);
    if expected
        .intent
        .zygisk_authorization
        .as_ref()
        .is_some_and(|a| a.boot_id == current_boot)
    {
        journal.note_zygisk_exposure(current_boot);
    }
    persist(journal).map_err(|error| {
        if crate::config_file::is_post_rename_error(&error) {
            PhaseOutcome::Unknown
        } else {
            PhaseOutcome::FailedKnown
        }
    })?;
    crate::recovery::maybe_crash_for_test("revocation-intent");
    let result = if can_dispatch() {
        target.revoke_expected(expected)
    } else {
        Err(PublishError::BeforeWrite("writer dispatch fenced".into()))
    };
    crate::recovery::maybe_crash_for_test("revocation-effect");
    // Even a before-write refusal after admission leaves the requested future
    // authorization unrevoked. Retain a hold rather than claim cleanup success.
    journal
        .zygisk_revocation
        .as_mut()
        .expect("revocation admitted")
        .state = if result.is_ok() {
        RevocationState::Confirmed
    } else {
        RevocationState::Unknown
    };
    persist(journal).map_err(|_| PhaseOutcome::Unknown)?;
    crate::recovery::maybe_crash_for_test("revocation-completion");
    result.map_err(|error| {
        crate::alog::info(&format!("Zygisk revocation unconfirmed: {error:?}"));
        PhaseOutcome::Unknown
    })
}

/// Select only an exact retained grant matching this recipient. A failed newer
/// admission may leave an older known grant published; unknown files stay refused.
pub(crate) fn revocable_authority(
    journal: &Journal,
    path: &std::path::Path,
) -> Result<Option<ZygiskAuthorityRecord>, PhaseOutcome> {
    let target = ZygiskTarget::for_revocation(path).map_err(|_| PhaseOutcome::FailedKnown)?;
    for authority in journal
        .zygisk_authority
        .iter()
        .chain(journal.prior_zygisk_authority.iter())
    {
        if authority.intent.zygisk_module.as_deref() == Some(path)
            && target.check_authorization(authority).is_ok()
        {
            return Ok(Some(authority.clone()));
        }
    }
    if target
        .authorization_absent()
        .map_err(|_| PhaseOutcome::FailedKnown)?
    {
        Ok(None)
    } else {
        Err(PhaseOutcome::FailedKnown)
    }
}

pub(crate) fn retire_prepared(
    journal: &mut Journal,
    prepared: &super::prepared::Prepared,
    operation: Option<OperationId>,
    persist: &mut impl FnMut(&Journal) -> io::Result<()>,
    can_dispatch: impl Fn() -> bool,
) -> Result<(), PhaseOutcome> {
    let Some(path) = prepared.bundled_retirement() else {
        return Ok(());
    };
    let target = ZygiskTarget::for_revocation(path).map_err(|_| PhaseOutcome::FailedKnown)?;
    match revocable_authority(journal, path)? {
        Some(expected) => revoke(
            journal,
            &expected,
            operation,
            &prepared.intent.boot_id,
            persist,
            can_dispatch,
        ),
        _ => {
            // No recorded grant may be synthesized from an arbitrary existing file.
            // An already absent grant needs no revocation effect.
            if target
                .read_authorized(&prepared.intent.boot_id)
                .map_err(|_| PhaseOutcome::FailedKnown)?
                .is_none()
            {
                Ok(())
            } else {
                Err(PhaseOutcome::FailedKnown)
            }
        }
    }
}

/// The single attempt for boot and explicit repair. The writer owns every
/// durable boundary, including a replacement command's exact policy recipient.
pub(crate) trait AttemptWriter {
    fn journal(&self) -> &Journal;
    fn begin(&mut self, prepared: &super::prepared::Prepared) -> Result<(), PhaseOutcome>;
    fn retire(&mut self, prepared: &super::prepared::Prepared) -> Result<(), PhaseOutcome>;
    fn complete(&mut self, result: crate::journal::NativeStartupResult)
    -> Result<(), PhaseOutcome>;
    fn can_dispatch(&self) -> bool;
}

pub(crate) fn activate(
    prepared: super::prepared::Prepared,
    writer: &mut impl AttemptWriter,
    epoch: u64,
    explicit: bool,
) -> Result<super::NativeBoot, PhaseOutcome> {
    activate_with(prepared, writer, epoch, explicit, |p, epoch| {
        p.execute_detailed(epoch)
    })
}
pub(super) fn activate_with(
    prepared: super::prepared::Prepared,
    writer: &mut impl AttemptWriter,
    epoch: u64,
    explicit: bool,
    mut execute: impl FnMut(super::prepared::Prepared, u64) -> super::prepared::PreparedExecution,
) -> Result<super::NativeBoot, PhaseOutcome> {
    let prepared = prepared
        .admit(writer.journal(), explicit)
        .map_err(|_| PhaseOutcome::FailedKnown)?;
    prepared.validate_binding().map_err(|error| {
        crate::alog::info(&format!("prepared generation refused: {error}"));
        PhaseOutcome::FailedKnown
    })?;
    writer
        .journal()
        .check_native_attempt(&prepared.intent, explicit)
        .map_err(|_| PhaseOutcome::FailedKnown)?;
    writer.begin(&prepared)?;
    crate::recovery::maybe_crash_for_test("startup-intent");
    writer.retire(&prepared)?;
    if !writer.can_dispatch() {
        return Err(PhaseOutcome::Unknown);
    }
    prepared
        .validate_binding()
        .map_err(|_| PhaseOutcome::Unknown)?;
    if !prepared.dispatch_ownership_clear() {
        return Err(PhaseOutcome::Unknown);
    }
    let execution = execute(prepared, epoch);
    let result = crate::journal::NativeStartupResult {
        status: execution.boot.status.clone(),
        selection: execution.boot.selection,
        rejection: execution.rejection,
    };
    // Keep no-effect proof for recovery and the attempt budget. It does not
    // authorize an automatic load of another backend family.
    writer.complete(result)?;
    crate::recovery::maybe_crash_for_test("startup-completion");
    Ok(execution.boot)
}

#[cfg(test)]
pub(crate) use tests::setup as test_fixture;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::{self, NativeIntent, ZygiskAuthorizationIntent};
    use std::{
        fs::{self, File},
        io::Read,
        path::Path,
    };
    use vpnhide_core_rpc::{ConfigPhase, NativeBackend};
    use vpnhide_protocol::zygisk_activation::{Authorization, FILE_NAME};
    const BOOT: &str = "12345678-1234-1234-1234-123456789abc";
    const WIRE: &str = "vpnhide 2 config\ndebug 0\ndefault 0\nend 0\n";
    pub(crate) fn setup(root: &Path) -> (Journal, ZygiskAuthorityRecord, ZygiskTarget) {
        crate::zygisk::test_fixture(root);
        fs::write(
            root.join("module.prop"),
            "id=vpnhide_core\nvpnhideControl=2\nvpnhideActivation=1\n",
        )
        .unwrap();
        let target = ZygiskTarget::open(root).unwrap();
        target
            .publish_authorized(&Authorization {
                boot_id: BOOT.into(),
                generation: 42,
                wire: WIRE.into(),
            })
            .unwrap();
        let authority = ZygiskAuthorityRecord {
            operation: OperationId(42),
            intent: NativeIntent {
                backend: NativeBackend::Zygisk,
                wire: WIRE.into(),
                zygisk_module: Some(root.into()),
                zygisk_authorization: Some(ZygiskAuthorizationIntent {
                    boot_id: BOOT.into(),
                    generation: 42,
                    directory_identity: target.identity().unwrap(),
                }),
            },
        };
        let journal = Journal {
            issued_through: 43,
            zygisk_authority: Some(authority.clone()),
            ..Default::default()
        };
        (journal, authority, target)
    }
    #[test]
    fn revocation_preserves_grant_old_readers_and_boot_fence_through_restart_and_pruning() {
        let root = tempfile::tempdir().unwrap();
        let (mut journal, authority, _) = setup(root.path());
        let path = root.path().join("operations.json");
        let mut reader = File::open(root.path().join(FILE_NAME)).unwrap();
        let mut writes = vec![];
        revoke(
            &mut journal,
            &authority,
            Some(OperationId(43)),
            BOOT,
            &mut |j| {
                writes.push(j.zygisk_revocation.as_ref().unwrap().state);
                journal::write_atomic(&path, j)
            },
            || true,
        )
        .unwrap();
        assert_eq!(
            writes,
            [RevocationState::Running, RevocationState::Confirmed]
        );
        assert!(!root.path().join(FILE_NAME).exists());
        let mut bytes = vec![];
        reader.read_to_end(&mut bytes).unwrap();
        assert_eq!(Authorization::decode(&bytes, BOOT).unwrap().generation, 42);
        let mut recovered = journal::read(&path).unwrap();
        recovered.retain_settled = 0;
        recovered.prune();
        recovered.fence_unresolved();
        assert_eq!(recovered.zygisk_authority, Some(authority));
        assert!(recovered.zygisk_exposed_in(BOOT));
        assert!(!recovered.revocation_uncertain());
        let intent = super::super::prepared::TransitionIntent {
            boot_id: BOOT.into(),
            filesystem_hiding: false,
            action: super::super::prepared::TransitionAction::Adopt(NativeBackend::Builtin),
            payload: None,
            bundle: None,
            retire_zygisk: None,
            retire_directory_identity: None,
        };
        assert!(recovered.check_native_attempt(&intent, true).is_err());
    }
    #[test]
    fn intent_write_failure_prevents_unlink_completion_write_failure_retains_restart_hold() {
        for write in [1, 2] {
            let root = tempfile::tempdir().unwrap();
            let (mut journal, authority, target) = setup(root.path());
            let path = root.path().join("operations.json");
            journal::write_atomic(&path, &journal).unwrap();
            journal::fail_write(write, false);
            let result = revoke(
                &mut journal,
                &authority,
                Some(OperationId(43)),
                BOOT,
                &mut |j| journal::write_atomic(&path, j),
                || true,
            );
            assert_eq!(
                result,
                Err(if write == 1 {
                    PhaseOutcome::FailedKnown
                } else {
                    PhaseOutcome::Unknown
                })
            );
            assert_eq!(target.authorization_absent().unwrap(), write == 2);
            let mut recovered = journal::read(&path).unwrap();
            recovered.fence_unresolved();
            if write == 2 {
                assert!(recovered.held_resources().contains(&ConfigPhase::Native));
                revoke(
                    &mut recovered,
                    &authority,
                    Some(OperationId(43)),
                    BOOT,
                    &mut |j| journal::write_atomic(&path, j),
                    || true,
                )
                .unwrap();
                assert!(!recovered.revocation_uncertain());
            }
        }
    }
    #[test]
    fn stale_source_replaced_recipient_and_fenced_dispatch_preserve_newer_authorization() {
        for change in [0, 1, 2, 3] {
            let root = tempfile::tempdir().unwrap();
            let module = root.path().join("module");
            let (mut journal, authority, target) = setup(&module);
            let path = root.path().join("operations.json");
            if change == 0 {
                target
                    .publish_authorized(&Authorization {
                        boot_id: BOOT.into(),
                        generation: 43,
                        wire: WIRE.into(),
                    })
                    .unwrap();
            }
            if change == 1 {
                fs::rename(&module, root.path().join("old")).unwrap();
                setup(&module);
            }
            let mut writes = 0;
            let result = revoke(
                &mut journal,
                &authority,
                Some(OperationId(43)),
                BOOT,
                &mut |j| {
                    writes += 1;
                    journal::write_atomic(&path, j)?;
                    if change == 2 && writes == 1 {
                        target
                            .publish_authorized(&Authorization {
                                boot_id: BOOT.into(),
                                generation: 43,
                                wire: WIRE.into(),
                            })
                            .unwrap();
                    }
                    Ok(())
                },
                || change != 3,
            );
            assert!(result.is_err());
            assert!(module.join(FILE_NAME).exists());
            assert_eq!(writes, if change < 2 { 0 } else { 2 });
            if change >= 2 {
                assert!(journal.revocation_uncertain());
            }
        }
    }
    #[test]
    fn inactive_or_damaged_payload_can_revoke_exact_grant_but_cannot_publish() {
        for change in [0, 1, 2] {
            let root = tempfile::tempdir().unwrap();
            let (mut journal, authority, active) = setup(root.path());
            match change {
                0 => fs::write(root.path().join("disable"), b"").unwrap(),
                1 => fs::write(root.path().join("remove"), b"").unwrap(),
                _ => fs::remove_dir_all(root.path().join("zygisk")).unwrap(),
            }
            let auth = Authorization {
                boot_id: BOOT.into(),
                generation: 42,
                wire: WIRE.into(),
            };
            assert!(active.publish_authorized(&auth).is_err());
            let revoker = ZygiskTarget::for_revocation(root.path()).unwrap();
            assert!(revoker.publish_authorized(&auth).is_err());
            let path = root.path().join("operations.json");
            revoke(
                &mut journal,
                &authority,
                Some(OperationId(43)),
                BOOT,
                &mut |j| journal::write_atomic(&path, j),
                || true,
            )
            .unwrap();
            assert!(revoker.authorization_absent().unwrap());
        }
    }
    #[test]
    fn schema_nine_upgrade_preserves_other_holds_and_future_downgrade_source_is_untouched() {
        let root = tempfile::tempdir().unwrap();
        let (mut journal, authority, _) = setup(root.path());
        journal.zygisk_revocation = Some(ZygiskRevocation {
            operation: Some(OperationId(43)),
            current_boot: BOOT.into(),
            authority,
            state: RevocationState::Running,
        });
        let path = root.path().join("operations.json");
        journal::write_atomic(&path, &journal).unwrap();
        let mut recovered = journal::read(&path).unwrap();
        recovered.fence_unresolved();
        assert!(recovered.revocation_uncertain());
        assert_eq!(
            recovered.zygisk_revocation.as_ref().unwrap().state,
            RevocationState::Unknown
        );
        let mut old = serde_json::to_value(&journal).unwrap();
        old["version"] = 9.into();
        old.as_object_mut().unwrap().remove("zygisk_revocation");
        fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        let upgraded = journal::read(&path).unwrap();
        assert_eq!(upgraded.zygisk_authority, journal.zygisk_authority);
        assert!(upgraded.zygisk_revocation.is_none());
        old["version"] = (vpnhide_build_contract::JOURNAL_VERSION + 1).into();
        let bytes = serde_json::to_vec(&old).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(journal::read(&path).is_err());
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    #[test]
    fn failed_newer_admissions_preserve_the_exact_old_published_grant_for_revocation() {
        use vpnhide_core_rpc::{BaseRef, CanonicalConfig, ConfigEdit};
        let root = tempfile::tempdir().unwrap();
        let (mut journal, old, _) = setup(root.path());
        let path = root.path().join("journal");
        for id in 44..80 {
            let mut newer = old.intent.clone();
            newer.zygisk_authorization.as_mut().unwrap().generation = id;
            journal::record_phase_running(
                &path,
                &mut journal,
                OperationId(id),
                BaseRef { revision: 0 },
                &ConfigEdit::RepairActivation,
                &CanonicalConfig::default(),
                &[ConfigPhase::Native],
                &[],
                ConfigPhase::Native,
                Some(newer),
            )
            .unwrap();
            journal.operations.clear();
        }
        journal::write_atomic(&path, &journal).unwrap();
        let mut journal = journal::read(&path).unwrap();
        assert_eq!(journal.prior_zygisk_authority, Some(old.clone()));
        let expected = revocable_authority(&journal, root.path()).unwrap().unwrap();
        assert_eq!(expected, old);
        revoke(
            &mut journal,
            &expected,
            Some(OperationId(79)),
            BOOT,
            &mut |j| journal::write_atomic(&path, j),
            || true,
        )
        .unwrap();
        assert!(!root.path().join(FILE_NAME).exists());
        assert!(journal.zygisk_exposed_in(BOOT));
    }
}
