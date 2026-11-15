#![cfg(target_os = "linux")]

use std::{env, fs, io, path::PathBuf, process::Command};
use vpnhide_core_daemon::autonomous::{native_fixture::TunFixture, route_fixture::RouteFixture};
use vpnhide_diagnostic_model::{
    AddressVector, NativeMeasurement, ProbeError, RouteCheck, RuleRole, cleanup_confirmed,
};

fn assert_isolated() -> io::Result<()> {
    let original = PathBuf::from(
        env::var_os("VPNHIDE_HOST_NETNS").expect("set VPNHIDE_HOST_NETNS before unshare"),
    );
    assert_ne!(fs::read_link("/proc/self/ns/net")?, original);
    Ok(())
}

#[test]
#[ignore = "run inside a fresh user and network namespace with lo up and /dev/net/tun"]
fn production_route_fixture_coexists_with_native_fixture_and_reclaims_both() -> io::Result<()> {
    assert_isolated()?;
    let before = vpnhide_checks::route::observe();
    let session = 0x0123_4567_89b2;
    let routes = RouteFixture::create_detailed(session).map_err(|failure| failure.error)?;
    let mut native = TunFixture::create_detailed(session).map_err(|failure| failure.error)?;
    let issued = routes.spec.clone();
    assert_ne!(native.spec.interface, issued.vpn);
    assert_ne!(native.spec.interface, issued.ordinary);
    let observed = vpnhide_checks::route::observe().scoped(&issued);
    assert!(observed.scoped_for(&issued));
    let mut worker = vpnhide_checks::fixture::observe(&native.spec.interface.name);
    worker.routes = Some(observed.clone());
    let transported = serde_json::to_vec(&worker).expect("serialize combined report");
    assert!(transported.len() < vpnhide_core_rpc::MAX_FRAME_BYTES);
    let worker: NativeMeasurement =
        serde_json::from_slice(&transported).expect("read worker report");
    assert!(worker.bounded_for_routes(Some(&issued)));

    let native_spec = native.spec.clone();
    let completion = native.finish_paired(&worker, Some(&worker));
    assert!(cleanup_confirmed(&native_spec, &completion.cleanup));
    assert_eq!(
        completion
            .addresses
            .leaked
            .iter()
            .filter(|check| check.vector == AddressVector::IfconfSize)
            .count(),
        2
    );

    // The isolated host has no filtering provider, so visible fixture routes
    // qualify as leaks. This never claims Android or RAW grant acceptance.
    let completion = routes.finish(worker.routes.as_ref().unwrap(), Some(&observed), true);
    assert!(completion.interfaces_clean);
    assert_eq!(completion.result.leaked, RouteCheck::required());
    assert!(completion.result.proven.is_empty());
    assert_eq!(vpnhide_checks::route::observe(), before);
    Ok(())
}

#[test]
#[ignore = "run in a fresh user and network namespace mapping uid 10000 with lo up"]
fn policy_rules_share_the_route_report_and_recover_after_descriptor_loss() -> io::Result<()> {
    assert_isolated()?;
    const CHILD_DIR: &str = "VPNHIDE_RULE_TEST_CHILD_DIR";
    let session = 0x0123_4567_89c2;
    if let Some(directory) = env::var_os(CHILD_DIR) {
        let mut fixture =
            RouteFixture::create_detailed(session).map_err(|failure| failure.error)?;
        fixture.enable_rules(&PathBuf::from(directory), 10_000)?;
        // Skip all Rust destructors. The kernel closes the TUN descriptors,
        // which leaves the durable receipt and the six separate rule objects.
        std::process::exit(42);
    }
    let directory = tempfile::Builder::new()
        .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .tempdir()?;
    let before = vpnhide_checks::rule::observe();
    let make_fixture = || -> io::Result<RouteFixture> {
        let mut fixture =
            RouteFixture::create_detailed(session).map_err(|failure| failure.error)?;
        fixture.enable_rules(directory.path(), 10_000)?;
        Ok(fixture)
    };
    let fixture = make_fixture()?;
    let spec = fixture.spec.clone();
    let mut observed = vpnhide_checks::route::observe();
    observed
        .observations
        .extend(vpnhide_checks::rule::observe().observations);
    let observed = observed.scoped(&spec);
    let mut native = NativeMeasurement::unavailable(ProbeError::Unsupported);
    native.routes = Some(observed.clone());
    let serialized = serde_json::to_vec(&native)?;
    let native: NativeMeasurement = serde_json::from_slice(&serialized)?;
    assert!(native.bounded_for_routes(Some(&spec)));
    let mut protected = observed.clone();
    for view in &mut protected.observations {
        view.rules
            .retain(|row| RuleRole::Ordinary.matches(&spec, row, false));
    }
    let mut cleanup = fixture.negative.clone();
    cleanup
        .observations
        .retain(|view| view.check.surface.rule());
    let evaluate = |protected, raw| {
        vpnhide_diagnostic_model::evaluate_paired_route_fixture(
            &spec,
            [
                Some(&fixture.negative),
                Some(&fixture.start),
                Some(protected),
                Some(&fixture.start),
                Some(&cleanup),
            ],
            raw,
            true,
        )
    };
    let proven = evaluate(&protected, Some(&observed));
    assert!(RouteCheck::rules().all(|check| proven.proven.contains(&check)));
    assert!(
        evaluate(&protected, None)
            .proven
            .iter()
            .all(|check| !check.surface.rule())
    );
    // Retain just the UID rule for one protected view. It alone must
    // establish a leak even after the interface rule is hidden.
    let mut partially_protected = protected.clone();
    let view = partially_protected
        .observations
        .iter_mut()
        .find(|v| v.check.surface.rule())
        .unwrap();
    let check = view.check;
    let raw = observed
        .observations
        .iter()
        .find(|v| v.check == check)
        .unwrap();
    view.rules.extend(
        raw.rules
            .iter()
            .filter(|row| RuleRole::WorkerUid.matches(&spec, row, false))
            .cloned(),
    );
    assert!(
        evaluate(&partially_protected, Some(&observed))
            .leaked
            .contains(&check)
    );
    let result = fixture.finish(native.routes.as_ref().unwrap(), Some(&observed), true);
    assert!(result.interfaces_clean && result.rules_clean);
    assert!(RouteCheck::rules().all(|check| result.result.leaked.contains(&check)));
    assert_eq!(vpnhide_checks::rule::observe(), before);

    let child = Command::new(env::current_exe()?)
        .args([
            "--ignored",
            "--exact",
            "policy_rules_share_the_route_report_and_recover_after_descriptor_loss",
        ])
        .env(CHILD_DIR, directory.path())
        .status()?;
    assert_eq!(child.code(), Some(42));
    let receipt = directory.path().join("diagnostic-rules.json");
    assert!(receipt.is_file());
    let after = vpnhide_checks::rule::observe();
    assert!(after.observations.iter().all(|v| {
        RuleRole::ALL
            .into_iter()
            .all(|role| v.rules.iter().any(|row| role.matches(&spec, row, true)))
    }));
    let preference = RuleRole::Vpn.priority(&spec).to_string();
    for (verb, expect_cleanup) in [("add", false), ("del", true)] {
        assert!(
            Command::new("ip")
                .args([
                    "-4",
                    "rule",
                    verb,
                    "pref",
                    &preference,
                    "protocol",
                    "243",
                    "nop"
                ])
                .status()?
                .success()
        );
        let cleanup = RouteFixture::recover_rules(directory.path());
        assert_eq!(cleanup.is_ok(), expect_cleanup, "{cleanup:?}");
        assert_eq!(receipt.is_file(), !expect_cleanup);
    }
    assert_eq!(vpnhide_checks::rule::observe(), before);
    Ok(())
}
