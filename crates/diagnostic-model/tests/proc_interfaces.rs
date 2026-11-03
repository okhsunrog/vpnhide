use vpnhide_diagnostic_model::*;
fn spec() -> NativeFixtureSpec {
    NativeFixtureSpec {
        session: 1,
        interface: InterfaceId {
            name: "tun000000000001".into(),
            index: 42,
        },
        flags: 16,
        mtu: 1543,
        ordinary: OrdinaryControl {
            interface: InterfaceId {
                name: "lo".into(),
                index: 1,
            },
            flags: 73,
            mtu: 65536,
        },
    }
}
fn measured(fixture: bool) -> ProcInterfaceMeasurement {
    let ordinary = ObservedAddress {
        name: Some("lo".into()),
        index: Some(1),
        address: [vec![0; 15], vec![1]].concat(),
        prefix: Some(128),
    };
    let target = ObservedAddress {
        name: Some("tun000000000001".into()),
        index: Some(42),
        address: vec![
            0xfd, 0x76, 0x70, 0x6e, 0x68, 0x69, 0x64, 0x65, 0, 0, 0, 0, 0, 1, 0, 1,
        ],
        prefix: Some(128),
    };
    ProcInterfaceMeasurement {
        actors: NativeReadActor::ALL
            .into_iter()
            .map(|actor| ProcInterfaceActorMeasurement {
                actor,
                devices: Sample::Value(if fixture {
                    vec!["lo".into(), "tun000000000001".into()]
                } else {
                    vec!["lo".into()]
                }),
                ipv6: Sample::Value(if fixture {
                    vec![ordinary.clone(), target.clone()]
                } else {
                    vec![ordinary.clone()]
                }),
            })
            .collect(),
    }
}
#[test]
fn raw_positive_and_all_actual_actors_are_required_for_scoped_proof() {
    let (n, p) = (measured(false), measured(true));
    let raw = evaluate_proc_interface_fixture(&spec(), [&n, &p, &p, &p, &n]);
    assert_eq!(raw.leaked.len(), 4);
    assert!(raw.proven.is_empty() && raw.controls.is_empty());
    let protected = evaluate_proc_interface_fixture(&spec(), [&n, &p, &n, &p, &n]);
    assert_eq!(protected.proven.len(), 4);
    assert!(protected.complete());
    let mut libc_only = n.clone();
    libc_only.actors.pop();
    let incomplete = evaluate_proc_interface_fixture(&spec(), [&n, &p, &libc_only, &p, &n]);
    assert_eq!(incomplete.proven.len(), 2);
    assert!(!incomplete.complete());
    assert!(
        incomplete
            .controls
            .contains(&ControlFailure::ProbeUnavailable)
    );
    let absent = evaluate_proc_interface_fixture(&spec(), [&n, &n, &p, &n, &n]);
    assert!(absent.leaked.is_empty() && absent.proven.is_empty());
    assert!(
        absent
            .controls
            .contains(&ControlFailure::MissingPositiveControl)
    );
}
#[test]
fn permission_and_empty_enumeration_are_unverified_instead_of_hidden() {
    let (n, p) = (measured(false), measured(true));
    for denied in [false, true] {
        let mut worker = n.clone();
        for actor in &mut worker.actors {
            actor.devices = if denied {
                Sample::Error(ProbeError::Errno(13))
            } else {
                Sample::Value(vec![])
            };
            actor.ipv6 = if denied {
                Sample::Error(ProbeError::Errno(1))
            } else {
                Sample::Value(vec![])
            };
        }
        let result = evaluate_proc_interface_fixture(&spec(), [&n, &p, &worker, &p, &n]);
        assert!(result.proven.is_empty() && result.leaked.is_empty() && !result.complete());
        assert!(result.controls.contains(&if denied {
            ControlFailure::ProbeUnavailable
        } else {
            ControlFailure::OrdinaryObjectsChanged
        }));
    }
}
#[test]
fn leaks_survive_scrubbed_identity_later_fixture_change_and_cleanup_failure() {
    let (n, p) = (measured(false), measured(true));
    let mut worker = p.clone();
    for actor in &mut worker.actors {
        if let Sample::Value(v) = &mut actor.ipv6 {
            v[1].name = None;
            v[1].index = None;
            v[1].prefix = None;
        }
    }
    let result = evaluate_proc_interface_fixture(&spec(), [&n, &p, &worker, &n, &p]);
    assert_eq!(result.leaked.len(), 4);
    assert!(result.proven.is_empty());
    assert!(result.controls.contains(&ControlFailure::FixtureChanged));
    assert!(
        result
            .controls
            .contains(&ControlFailure::CleanupUnconfirmed)
    );
}
#[test]
fn unavailable_end_or_cleanup_retains_qualified_worker_leaks_and_blocks_proof() {
    let (n, p) = (measured(false), measured(true));
    for (lost_end, lost_cleanup) in [(true, false), (false, true), (true, true)] {
        let mut end = p.clone();
        let mut cleanup = n.clone();
        for (lost, m, error) in [
            (lost_end, &mut end, ProbeError::Timeout),
            (lost_cleanup, &mut cleanup, ProbeError::Errno(13)),
        ] {
            if lost {
                for a in &mut m.actors {
                    a.devices = Sample::Error(error.clone());
                    a.ipv6 = Sample::Error(error.clone());
                }
            }
        }
        let observed = evaluate_proc_interface_fixture(&spec(), [&n, &p, &p, &end, &cleanup]);
        assert_eq!(observed.leaked.len(), 4);
        assert!(observed.proven.is_empty());
        assert!(
            observed
                .controls
                .contains(&ControlFailure::ProbeUnavailable)
        );
        if lost_end {
            assert!(observed.controls.contains(&ControlFailure::FixtureChanged));
        }
        if lost_cleanup {
            assert!(
                observed
                    .controls
                    .contains(&ControlFailure::CleanupUnconfirmed)
            );
        }
        let hidden = evaluate_proc_interface_fixture(&spec(), [&n, &p, &n, &end, &cleanup]);
        assert!(hidden.proven.is_empty() && hidden.leaked.is_empty());
    }
}
#[test]
fn missing_later_actor_records_cannot_erase_prior_proc_leaks() {
    let (n, p) = (measured(false), measured(true));
    let unavailable = ProcInterfaceMeasurement { actors: vec![] };
    let result = evaluate_proc_interface_fixture(&spec(), [&n, &p, &p, &unavailable, &unavailable]);
    assert_eq!(result.leaked.len(), 4);
    assert!(result.proven.is_empty());
    assert!(
        result
            .controls
            .contains(&ControlFailure::CleanupUnconfirmed)
    );
}

#[test]
fn missing_later_component_preserves_qualified_proc_evidence_in_core_projection() {
    let mut n = NativeMeasurement::unavailable(ProbeError::Unsupported);
    let mut p = n.clone();
    n.proc_interfaces = Some(measured(false));
    p.proc_interfaces = Some(measured(true));
    let missing = NativeMeasurement::unavailable(ProbeError::Timeout);
    let result = evaluate_native_proc_interface_fixture(&spec(), [&n, &p, &p, &missing, &missing]);
    assert_eq!(result.leaked.len(), 4);
    assert!(result.proven.is_empty());
    assert!(
        result
            .controls
            .contains(&ControlFailure::CleanupUnconfirmed)
    );
}

#[test]
fn duplicate_actors_and_invalid_records_never_make_complete_coverage() {
    let (n, p) = (measured(false), measured(true));
    let mut bad = n.clone();
    bad.actors[1] = bad.actors[0].clone();
    assert!(!bad.bounded());
    assert!(
        evaluate_proc_interface_fixture(&spec(), [&n, &p, &bad, &p, &n])
            .controls
            .contains(&ControlFailure::InvalidSpec)
    );
    let mut malformed = n.clone();
    if let Sample::Value(v) = &mut malformed.actors[0].ipv6 {
        v[0].address = vec![127, 0, 0, 1];
    }
    assert!(!malformed.bounded());
}
