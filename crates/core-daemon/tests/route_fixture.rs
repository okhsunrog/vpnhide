#![cfg(target_os = "linux")]

use std::{env, fs, io, path::PathBuf};
use vpnhide_core_daemon::autonomous::{native_fixture::TunFixture, route_fixture::RouteFixture};
use vpnhide_diagnostic_model::{NativeMeasurement, ProbeError, RouteCheck, cleanup_confirmed};

#[test]
#[ignore = "run inside a fresh user and network namespace with lo up and /dev/net/tun"]
fn production_route_fixture_coexists_with_native_fixture_and_reclaims_both() -> io::Result<()> {
    let original = PathBuf::from(
        env::var_os("VPNHIDE_HOST_NETNS").expect("set VPNHIDE_HOST_NETNS before unshare"),
    );
    assert_ne!(fs::read_link("/proc/self/ns/net")?, original);
    let before = vpnhide_checks::route::observe();
    let session = 0x0123_4567_89b2;
    let native = TunFixture::create_detailed(session).map_err(|failure| failure.error)?;
    let routes = RouteFixture::create_detailed(session).map_err(|failure| failure.error)?;
    let issued = routes.spec.clone();
    assert_ne!(native.spec.interface, issued.vpn);
    assert_ne!(native.spec.interface, issued.ordinary);
    let observed = vpnhide_checks::route::observe().scoped(&issued);
    assert!(observed.scoped_for(&issued));
    let mut worker = NativeMeasurement::unavailable(ProbeError::Unsupported);
    worker.routes = Some(observed.clone());
    let transported = serde_json::to_vec(&worker).expect("serialize combined report");
    assert!(transported.len() < vpnhide_core_rpc::MAX_FRAME_BYTES);
    let worker: NativeMeasurement =
        serde_json::from_slice(&transported).expect("read worker report");
    assert!(worker.bounded_for_routes(Some(&issued)));

    // The isolated host has no filtering provider, so visible fixture routes
    // qualify as leaks. This never claims Android or RAW grant acceptance.
    let completion = routes.finish(worker.routes.as_ref().unwrap(), Some(&observed), true);
    assert!(completion.interfaces_clean);
    assert_eq!(completion.result.leaked, RouteCheck::required());
    assert!(completion.result.proven.is_empty());
    let native_spec = native.spec.clone();
    let completion = native.finish_paired(&worker, None);
    assert!(cleanup_confirmed(&native_spec, &completion.cleanup));
    assert_eq!(vpnhide_checks::route::observe(), before);
    Ok(())
}
