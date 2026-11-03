//! Linux engineering route prototype only. No Android/Zygote/compat proof.
use std::{
    io,
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    time::Duration,
};
use vpnhide_core_daemon::autonomous::{native_fixture::TunFixture, route_fixture::RouteFixture};
use vpnhide_diagnostic_model::{RouteCheck, RouteMeasurement};
pub fn verify(protected: bool) -> Result<(), Box<dyn std::error::Error>> {
    // Keep a distinct DOWN witness for the existing thread-RAW handshake; the
    // two UP route owners never masquerade as the old DOWN fixture spec.
    let witness = TunFixture::create(0x0123_4567_89b0)?;
    let fixture = RouteFixture::create(0x0123_4567_89b0)?;
    println!("ROUTE_SPEC={}", serde_json::to_string(&fixture.spec)?);
    println!(
        "ROUTE_NEGATIVE={}",
        serde_json::to_string(&fixture.negative)?
    );
    println!(
        "ROUTE_ROOT_START={}",
        serde_json::to_string(&fixture.start)?
    );
    let raw = if protected {
        Some(super::raw_control::verify(&witness.spec)?)
    } else {
        None
    };
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(["route-worker", &serde_json::to_string(&fixture.spec)?])
        .stderr(Stdio::inherit());
    // SAFETY: isolated Linux oracle child only; this is not Android admission.
    unsafe {
        command.pre_exec(|| {
            let inet: libc::gid_t = 3003;
            if libc::setgroups(1, &inet) != 0
                || libc::setgid(10000) != 0
                || libc::setuid(10000) != 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let output = vpnhide_activator::output_with_timeout(&mut command, Duration::from_secs(5))?;
    if !output.status.success() {
        return Err("route worker failed".into());
    }
    if output.stdout.len() > vpnhide_diagnostic_model::SCOPED_ROUTE_JSON_BUDGET {
        return Err("scoped route worker exceeded its transport reservation".into());
    }
    let worker: RouteMeasurement = serde_json::from_slice(&output.stdout)?;
    println!("ROUTE_WORKER={}", serde_json::to_string(&worker)?);
    if let Some(raw) = &raw {
        println!("ROUTE_APP_RAW={}", serde_json::to_string(&raw.raw)?);
        println!(
            "ROUTE_SAME_THREAD_PROTECTED={}",
            serde_json::to_string(&raw.protected)?
        );
        println!(
            "ROUTE_OTHER_THREAD_PROTECTED={}",
            serde_json::to_string(&raw.other_thread)?
        );
    }
    let spec = fixture.spec.clone();
    let negative = fixture.negative.clone();
    let start = fixture.start.clone();
    let completion = fixture.finish(&worker, raw.as_ref().map(|v| &v.raw), protected);
    println!("ROUTE_ROOT_END={}", serde_json::to_string(&completion.end)?);
    println!(
        "ROUTE_CLEANUP={}",
        serde_json::to_string(&completion.cleanup)?
    );
    println!(
        "ROUTE_RESULT={}",
        serde_json::to_string(&completion.result)?
    );
    if let Some(raw) = &raw {
        // All worker variants use the actual Root end and post-close cleanup.
        for measurement in [&raw.protected, &raw.other_thread] {
            let result = vpnhide_diagnostic_model::evaluate_paired_route_fixture(
                &spec,
                [
                    Some(&negative),
                    Some(&start),
                    Some(measurement),
                    Some(&completion.end),
                    Some(&completion.cleanup),
                ],
                Some(&raw.raw),
                completion.interfaces_clean,
            );
            if !result.complete() {
                return Err(
                    format!("route RAW thread isolation/cleanup failed: {result:?}").into(),
                );
            }
        }
    }
    if !completion.interfaces_clean
        || if protected {
            !completion.result.complete()
        } else {
            !completion.result.controls.is_empty()
                || completion.result.leaked.len() != RouteCheck::required().len()
                || !completion.result.proven.is_empty()
        }
    {
        return Err("route stimulus/differential/cleanup assertion failed".into());
    }
    drop(witness);
    lifecycle_controls()?;
    println!(
        "ROUTE_ORACLE=PASS mode={} owned_up_pair=1 ordinary_preserved=1 cleanup=1",
        if protected { "protected" } else { "raw" }
    );
    Ok(())
}

fn lifecycle_controls() -> Result<(), Box<dyn std::error::Error>> {
    use vpnhide_core_daemon::autonomous::route_fixture::observe_cleanup;
    let fixture = RouteFixture::create(0x0123_4567_89b1)?;
    let spec = fixture.spec.clone();
    let last = fixture.duplicate_for_oracle(false)?;
    let closed = fixture.close_for_oracle();
    if closed.interfaces_clean || spec.negative(&closed.cleanup) {
        return Err("duplicated route FD falsely reports cleanup".into());
    }
    drop(last);
    let (cleanup, clean) = observe_cleanup(&spec);
    if !clean || !spec.negative(&cleanup) {
        return Err("last route FD failed to remove owned objects".into());
    }
    println!("ROUTE_DUPLICATE_CONTROL=PASS");
    let mut command = Command::new(std::env::current_exe()?);
    command.arg("route-crash-owner");
    let output = vpnhide_activator::output_with_timeout(&mut command, Duration::from_secs(8))?;
    if !output.status.success() {
        return Err("crash route owner did not reach _exit".into());
    }
    let spec: vpnhide_diagnostic_model::RouteFixtureSpec = serde_json::from_slice(&output.stdout)?;
    let (cleanup, clean) = observe_cleanup(&spec);
    if !spec.valid() || !clean || !spec.negative(&cleanup) {
        return Err("process-exit route cleanup failed".into());
    }
    println!("ROUTE_CRASH_CONTROL=PASS");
    let spec = RouteFixture::failed_setup_for_oracle(0x0123_4567_89b3)?;
    let (cleanup, clean) = observe_cleanup(&spec);
    if !spec.valid() || !clean || !spec.negative(&cleanup) {
        return Err("partial route setup cleanup failed".into());
    }
    println!("ROUTE_SETUP_FAILURE_CONTROL=PASS");
    Ok(())
}
