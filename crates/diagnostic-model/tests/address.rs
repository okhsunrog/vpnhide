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
fn entry(fixture: bool, family: usize, prefix: bool) -> ObservedAddress {
    let address = match (fixture, family) {
        (true, 4) => vec![198, 18, 0, 1],
        (false, 4) => vec![127, 0, 0, 1],
        (true, _) => vec![
            0xfd, 0x76, 0x70, 0x6e, 0x68, 0x69, 0x64, 0x65, 0, 0, 0, 0, 0, 1, 0, 1,
        ],
        _ => vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
    };
    ObservedAddress {
        name: Some(if fixture { "tun000000000001" } else { "lo" }.into()),
        index: Some(if fixture { 42 } else { 1 }),
        address,
        prefix: prefix.then_some(if family == 16 {
            128
        } else if fixture {
            32
        } else {
            8
        }),
    }
}
fn measurement(visible: bool) -> AddressMeasurement {
    let mut addresses = vec![entry(false, 4, true), entry(false, 16, true)];
    if visible {
        addresses.extend([entry(true, 4, true), entry(true, 16, true)]);
    }
    AddressMeasurement {
        ifaddrs: Sample::Value(addresses.clone()),
        actors: NativeReadActor::ALL
            .into_iter()
            .map(|actor| {
                let mut entries = vec![entry(false, 4, false)];
                if visible {
                    entries.push(entry(true, 4, false));
                }
                let count = entries.len() as u32 * 40;
                AddressActorMeasurement {
                    actor,
                    netlink: Sample::Value(addresses.clone()),
                    ioctls: AddressIoctl::ALL
                        .into_iter()
                        .map(|kind| {
                            let fixture = if !visible {
                                Sample::Error(ProbeError::Errno(19))
                            } else {
                                Sample::Value(match kind {
                                    AddressIoctl::Address | AddressIoctl::Destination => {
                                        vec![198, 18, 0, 1]
                                    }
                                    AddressIoctl::Netmask => vec![255; 4],
                                    AddressIoctl::Broadcast => vec![0; 4],
                                })
                            };
                            AddressIoctlObservation {
                                kind,
                                fixture,
                                ordinary: Sample::Value(vec![127, 0, 0, 1]),
                            }
                        })
                        .collect(),
                    ifconf: Sample::Value(IfconfObservation {
                        entries,
                        record_bytes: 40,
                        used_bytes: count,
                        size_before: count,
                        size_after: count,
                    }),
                }
            })
            .collect(),
    }
}
fn check(actor: NativeReadActor, vector: AddressVector) -> AddressCheck {
    AddressCheck { actor, vector }
}

#[test]
fn address_assignment_contract_is_independently_pinned() {
    assert_eq!(
        fixture_addresses(0x1234_5678_9abc),
        (
            [198, 18, 154, 188],
            [
                0xfd, 0x76, 0x70, 0x6e, 0x68, 0x69, 0x64, 0x65, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc,
                0, 1
            ]
        )
    );
    assert_eq!(fixture_addresses(0x1234_5679_9abc).0, [198, 19, 154, 188]);
}

#[test]
fn raw_and_protected_observations_require_both_syscall_actors_and_address_families() {
    let (n, p) = (measurement(false), measurement(true));
    let raw = evaluate_address_fixture(&spec(), &n, &p, &p, &p, &n);
    assert_eq!(raw.leaked.len(), 18);
    assert!(raw.proven.is_empty() && raw.controls.is_empty());
    let hidden = evaluate_address_fixture(&spec(), &n, &p, &n, &p, &n);
    assert_eq!(hidden.proven.len(), 18);
    assert!(hidden.complete());
    let mut libc_only = n.clone();
    libc_only.actors[1] = p.actors[1].clone();
    let partial = evaluate_address_fixture(&spec(), &n, &p, &libc_only, &p, &n);
    assert_eq!(partial.proven.len(), 10);
    assert_eq!(partial.leaked.len(), 8);
    assert!(
        partial
            .leaked
            .iter()
            .all(|c| c.actor == NativeReadActor::Syscall)
    );
    assert!(!partial.complete());
}

#[test]
fn literal_addresses_remain_leaks_after_both_identity_fields_are_scrubbed() {
    let (negative, positive) = (measurement(false), measurement(true));
    let mut worker = positive.clone();
    let scrub = |entries: &mut Vec<ObservedAddress>| {
        for entry in entries {
            if entry.index == Some(42) {
                entry.name = None;
                entry.index = None;
                entry.prefix = None;
            }
        }
    };
    if let Sample::Value(entries) = &mut worker.ifaddrs {
        scrub(entries);
    }
    for actor in &mut worker.actors {
        if let Sample::Value(entries) = &mut actor.netlink {
            scrub(entries);
        }
        if let Sample::Value(ifconf) = &mut actor.ifconf {
            scrub(&mut ifconf.entries);
        }
    }
    assert!(worker.bounded());
    let result =
        evaluate_address_fixture(&spec(), &negative, &positive, &worker, &positive, &negative);
    assert_eq!(result.leaked.len(), 18);
    assert!(result.proven.is_empty());
    assert!(result.controls.is_empty());
}

#[test]
fn missing_raw_positive_is_not_a_leak_or_an_empty_enumeration_pass() {
    let n = measurement(false);
    let empty = evaluate_address_fixture(&spec(), &n, &n, &n, &n, &n);
    assert!(empty.proven.is_empty() && empty.leaked.is_empty());
    assert!(
        empty
            .controls
            .contains(&ControlFailure::MissingPositiveControl)
    );
    let p = measurement(true);
    let missing = evaluate_address_fixture(&spec(), &n, &n, &p, &n, &n);
    assert!(missing.leaked.is_empty());
    assert!(!missing.complete());
}

#[test]
fn null_ifconf_size_is_independent_of_filtered_data_and_uses_the_worker_abi() {
    let (n, p) = (measurement(false), measurement(true));
    let mut worker = n.clone();
    for actor in &mut worker.actors {
        if let Sample::Value(v) = &mut actor.ifconf {
            v.record_bytes = 32;
            v.used_bytes = 32;
            v.size_before = 64;
            v.size_after = 64;
        }
    }
    let result = evaluate_address_fixture(&spec(), &n, &p, &worker, &p, &n);
    assert_eq!(
        result.leaked,
        vec![
            check(NativeReadActor::Libc, AddressVector::IfconfSize),
            check(NativeReadActor::Syscall, AddressVector::IfconfSize)
        ]
    );
    assert_eq!(result.proven.len(), 16);
    for actor in &mut worker.actors {
        if let Sample::Value(v) = &mut actor.ifconf {
            v.size_before = 32;
            v.size_after = 32;
        }
    }
    assert!(evaluate_address_fixture(&spec(), &n, &p, &worker, &p, &n).complete());
    if let Sample::Value(v) = &mut worker.actors[0].ifconf {
        v.size_after = 64;
    }
    let changed = evaluate_address_fixture(&spec(), &n, &p, &worker, &p, &n);
    assert!(
        !changed
            .proven
            .contains(&check(NativeReadActor::Libc, AddressVector::IfconfSize))
    );
    assert!(
        !changed
            .leaked
            .contains(&check(NativeReadActor::Libc, AddressVector::IfconfSize))
    );
}

#[test]
fn cleanup_failure_and_lost_ordinary_objects_preserve_observed_leaks() {
    let (n, p) = (measurement(false), measurement(true));
    let failed = evaluate_address_fixture(&spec(), &n, &p, &p, &p, &p);
    assert_eq!(failed.leaked.len(), 18);
    assert!(
        failed
            .controls
            .contains(&ControlFailure::CleanupUnconfirmed)
    );
    let mut broken = n.clone();
    broken.ifaddrs = Sample::Value(vec![]);
    let changed = evaluate_address_fixture(&spec(), &n, &p, &broken, &p, &n);
    assert!(
        changed
            .controls
            .contains(&ControlFailure::OrdinaryObjectsChanged)
    );
    assert!(
        !changed
            .proven
            .contains(&check(NativeReadActor::Libc, AddressVector::GetifaddrsIpv6))
    );
}

#[test]
fn permission_failure_duplicate_actors_and_malformed_records_never_pass() {
    let (n, p) = (measurement(false), measurement(true));
    let mut denied = n.clone();
    denied.actors[0].ioctls[0].fixture = Sample::Error(ProbeError::Errno(13));
    let result = evaluate_address_fixture(&spec(), &n, &p, &denied, &p, &n);
    assert!(result.controls.contains(&ControlFailure::ProbeUnavailable));
    assert!(
        !result
            .proven
            .contains(&check(NativeReadActor::Libc, AddressVector::IoctlAddress))
    );
    denied.actors.push(denied.actors[0].clone());
    assert!(!denied.bounded());
    assert_eq!(
        evaluate_address_fixture(&spec(), &n, &p, &denied, &p, &n).controls,
        vec![ControlFailure::InvalidSpec]
    );
    let mut invalid = entry(false, 4, true);
    invalid.prefix = Some(33);
    assert!(!invalid.valid());
    invalid.prefix = Some(8);
    invalid.index = Some(0);
    assert!(!invalid.valid());
}
