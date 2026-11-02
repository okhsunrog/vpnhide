use super::*;
use std::{
    collections::VecDeque,
    os::unix::fs::{PermissionsExt, symlink},
};
use vpnhide_maintenance_model::{RESET_SCHEMA, ResetStatus};
struct Fake {
    device: DeviceIdentity,
    replies: VecDeque<Option<ResetStatus>>,
    calls: Vec<(String, Option<u64>)>,
}
impl Fake {
    fn new(statuses: &[Option<ResetStatus>]) -> Self {
        Self {
            device: DeviceIdentity {
                serial: "lab-device".into(),
                boot_id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".into(),
            },
            replies: statuses.iter().copied().collect(),
            calls: Vec::new(),
        }
    }
}
impl Transport for Fake {
    fn identity(&mut self) -> io::Result<DeviceIdentity> {
        Ok(self.device.clone())
    }
    fn reset(
        &mut self,
        action: &str,
        op: Option<u64>,
        expected_boot: &str,
    ) -> io::Result<ResetReply> {
        self.calls.push((action.into(), op));
        if expected_boot != self.device.boot_id {
            return Ok(ResetReply {
                schema: RESET_SCHEMA,
                boot_id: Some(self.device.boot_id.clone()),
                epoch: None,
                operation: op,
                status: ResetStatus::Rejected,
            });
        }
        match self.replies.pop_front().expect("unexpected reset call") {
            Some(status) => Ok(ResetReply {
                schema: RESET_SCHEMA,
                boot_id: Some("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".into()),
                epoch: Some(99),
                operation: Some(17),
                status,
            }),
            None => Err(io::Error::other("lost reply")),
        }
    }
    fn uninstall(&mut self) -> io::Result<()> {
        self.calls.push(("uninstall".into(), None));
        Ok(())
    }
}
#[test]
fn lost_submission_recovers_by_query_only_and_uninstalls_after_confirmation() {
    let r = tempfile::tempdir().unwrap();
    let lease = Lease::open(&r.path().join("receipt")).unwrap();
    let mut fake = Fake::new(&[
        Some(ResetStatus::Allocated),
        None,
        Some(ResetStatus::Running),
    ]);
    let first = advance(&lease, &mut fake, true).unwrap();
    assert_eq!(first.session.phase, ResetPhase::Waiting);
    assert_eq!(
        fake.calls,
        [
            ("allocate".into(), None),
            ("submit".into(), Some(17)),
            ("query".into(), Some(17))
        ]
    );
    fake.calls.clear();
    fake.replies.push_back(Some(ResetStatus::Confirmed));
    let second = advance(&lease, &mut fake, true).unwrap();
    assert_eq!(second.uninstall, Uninstall::Confirmed);
    assert_eq!(
        fake.calls,
        [("query".into(), Some(17)), ("uninstall".into(), None)]
    );
    fake.calls.clear();
    advance(&lease, &mut fake, true).unwrap();
    assert!(fake.calls.is_empty());
}
#[test]
fn crash_after_intent_never_replays_and_missing_receipt_is_unknown() {
    let r = tempfile::tempdir().unwrap();
    let lease = Lease::open(&r.path().join("receipt")).unwrap();
    let mut fake = Fake::new(&[Some(ResetStatus::NotFound)]);
    let mut session = ResetSession::new(fake.device.clone()).unwrap();
    session
        .allocated(&ResetReply {
            schema: RESET_SCHEMA,
            boot_id: Some("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".into()),
            epoch: Some(99),
            operation: Some(17),
            status: ResetStatus::Allocated,
        })
        .unwrap();
    session.begin_submit(&fake.device).unwrap();
    lease
        .save(&State {
            current_confirmation: false,
            session,
            uninstall: Uninstall::Requested,
        })
        .unwrap();
    let state = advance(&lease, &mut fake, true).unwrap();
    assert_eq!(state.session.phase, ResetPhase::Unknown);
    assert_eq!(fake.calls, [("query".into(), Some(17))]);
}
#[test]
fn known_rejection_or_wrong_boot_never_uninstalls_or_dispatches_again() {
    let r = tempfile::tempdir().unwrap();
    let lease = Lease::open(&r.path().join("receipt")).unwrap();
    let mut fake = Fake::new(&[Some(ResetStatus::Allocated), Some(ResetStatus::Rejected)]);
    assert_eq!(
        advance(&lease, &mut fake, true).unwrap().session.phase,
        ResetPhase::FailedKnown
    );
    fake.calls.clear();
    advance(&lease, &mut fake, true).unwrap();
    assert!(fake.calls.is_empty());
    let mut state = lease.load().unwrap().unwrap();
    state.session.phase = ResetPhase::Allocated;
    lease.save(&state).unwrap();
    fake.device.boot_id = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb".into();
    assert!(advance(&lease, &mut fake, true).is_err());
    assert!(fake.calls.is_empty());
}
#[test]
fn receipt_lock_permissions_and_malformed_removal_state_fail_closed() {
    let r = tempfile::tempdir().unwrap();
    let path = r.path().join("receipt");
    let lease = Lease::open(&path).unwrap();
    assert!(Lease::open(&path).is_err());
    let state = State {
        current_confirmation: false,
        session: ResetSession::new(Fake::new(&[]).device).unwrap(),
        uninstall: Uninstall::NotRequested,
    };
    lease.save(&state).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(lease.load().is_err());
    fs::remove_file(&path).unwrap();
    symlink("elsewhere", &path).unwrap();
    assert!(lease.load().is_err());
    let invalid = State {
        current_confirmation: false,
        session: state.session,
        uninstall: Uninstall::Confirmed,
    };
    assert!(lease.save(&invalid).is_err());
}
#[test]
fn lock_suffix_and_cross_boot_receipts_never_produce_confirmation() {
    let r = tempfile::tempdir().unwrap();
    let path = r.path().join("reset.lock");
    let lease = Lease::open(&path).unwrap();
    assert!(!path.exists());
    assert!(r.path().join("reset.lock.lock").is_file());
    let mut fake = Fake::new(&[
        Some(ResetStatus::Allocated),
        None,
        Some(ResetStatus::Running),
    ]);
    advance(&lease, &mut fake, false).unwrap();
    fake.calls.clear();
    fake.device.boot_id = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb".into();
    let state = advance(&lease, &mut fake, false).unwrap();
    assert_eq!(state.session.phase, ResetPhase::Unknown);
    assert_eq!(fake.calls, [("query".into(), Some(17))]);
}
#[test]
fn cached_confirmation_is_rechecked_before_uninstall_or_current_success() {
    for uninstall in [false, true] {
        let r = tempfile::tempdir().unwrap();
        let lease = Lease::open(&r.path().join("receipt")).unwrap();
        let mut fake = Fake::new(&[Some(ResetStatus::NotFound)]);
        let mut session = ResetSession::new(fake.device.clone()).unwrap();
        session
            .allocated(&ResetReply {
                schema: RESET_SCHEMA,
                boot_id: Some(fake.device.boot_id.clone()),
                epoch: Some(99),
                operation: Some(17),
                status: ResetStatus::Allocated,
            })
            .unwrap();
        session.begin_submit(&fake.device).unwrap();
        session
            .observe(&ResetReply {
                schema: RESET_SCHEMA,
                boot_id: Some(fake.device.boot_id.clone()),
                epoch: Some(99),
                operation: Some(17),
                status: ResetStatus::Confirmed,
            })
            .unwrap();
        lease
            .save(&State {
                session,
                uninstall: if uninstall {
                    Uninstall::Requested
                } else {
                    Uninstall::NotRequested
                },
                current_confirmation: false,
            })
            .unwrap();
        let result = advance(&lease, &mut fake, uninstall).unwrap();
        assert_eq!(result.session.phase, ResetPhase::Unknown);
        assert!(!result.current_confirmation);
        assert_eq!(fake.calls, [("query".into(), Some(17))]);
    }
}
#[test]
fn joined_adb_argv_preserves_su_payload_and_old_core_guard_fails_closed() {
    // Emulate ADB's remote argument joining and the su -c parser using
    // unprivileged host processes. No ADB server or device is contacted.
    let r = tempfile::tempdir().unwrap();
    let core = r.path().join("core");
    let adb = r.path().join("adb");
    let su = r.path().join("su");
    let ready = r.path().join("ready");
    let dispatched = r.path().join("dispatched");
    let boot = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
    let json = serde_json::to_string(&ResetReply {
        schema: RESET_SCHEMA,
        boot_id: Some(boot.into()),
        epoch: Some(99),
        operation: Some(17),
        status: ResetStatus::Accepted,
    })
    .unwrap();
    fs::write(
        &su,
        "#!/bin/sh\n[ \"$#\" -eq 2 ] && [ \"$1\" = -c ] || exit 88\nexec /bin/sh -c \"$2\"\n",
    )
    .unwrap();
    fs::write(&adb,format!("#!/bin/sh\n[ \"$1\" = -s ] && [ \"$2\" = lab-device ] && [ \"$3\" = shell ] || exit 89\nshift 3\nPATH='{}':\"$PATH\"; export PATH\nexec /bin/sh -c \"$*\"\n",r.path().display())).unwrap();
    fs::write(&core,format!("#!/bin/sh\nif [ \"$1:$2\" = probe:maintenance-reset-ready ]; then [ \"$3\" = 1 ] || exit 2; [ -f '{}' ]; exit $?; fi\nprintf '%s\\n' \"$*\" > '{}'\n[ \"$*\" = 'maintenance-reset submit 17 {boot}' ] || exit 90\nprintf '%s\\n' '{json}'\n",ready.display(),dispatched.display())).unwrap();
    for path in [&core, &adb, &su] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let transport = Adb {
        program: adb,
        serial: "lab-device".into(),
    };
    assert!(
        transport
            .reset_at(core.to_str().unwrap(), "submit", Some(17), boot)
            .is_err()
    );
    assert!(
        !dispatched.exists(),
        "old Core must not receive the new verb"
    );
    fs::write(ready, []).unwrap();
    let reply = transport
        .reset_at(core.to_str().unwrap(), "submit", Some(17), boot)
        .unwrap();
    assert_eq!(reply.status, ResetStatus::Accepted);
    assert_eq!(
        fs::read_to_string(dispatched).unwrap(),
        format!("maintenance-reset submit 17 {boot}\n")
    );
}
