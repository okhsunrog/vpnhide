//! Explicit QEMU-only native kernel oracle, never included in Core/APK packages.
//! A setuid child exercises kernel filtering; it is NOT a Zygisk/Android worker
//! acceptance substitute. Android diagnostics uses the real Zygote Service.

use std::io::{self, Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use vpnhide_core_daemon::autonomous::native_fixture::TunFixture;
use vpnhide_diagnostic_model::{
    AddressCheck, BindActor, ControlFailure, FilesystemCheck, NativeFixtureSpec, NativeMeasurement,
    ProcInterfaceCheck, ScopedNativeResult, SysInterfaceCheck, cleanup_confirmed,
};

#[path = "fixture-oracle/raw_control.rs"]
mod raw_control;
#[path = "fixture-oracle/route_control.rs"]
mod route_control;

fn main() {
    if let Err(error) = run() {
        eprintln!("FIXTURE_ORACLE=FAIL {error}");
        std::process::exit(1);
    }
}

fn routes() -> io::Result<Vec<Vec<u8>>> {
    [
        "/proc/net/route",
        "/proc/net/ipv6_route",
        "/proc/net/if_inet6",
    ]
    .into_iter()
    .map(|p| {
        let mut bytes = Vec::new();
        std::fs::File::open(p)?
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 1024 * 1024 {
            return Err(io::Error::other("route control exceeds budget"));
        }
        Ok(bytes)
    })
    .collect()
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let commandline = std::fs::read_to_string("/proc/cmdline")?;
    if !commandline
        .split_whitespace()
        .any(|p| p == "vpnhide_fixture_oracle=1")
    {
        return Err("not an explicitly enabled QEMU oracle boot".into());
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    if let [mode, name] = args.as_slice()
        && mode == "raw-thread-worker"
    {
        return raw_control::worker(name);
    }
    if let [mode, raw_spec] = args.as_slice()
        && mode == "route-worker"
    {
        if unsafe { libc::geteuid() } != 10000 {
            return Err("route worker is not oracle app UID".into());
        }
        let spec: vpnhide_diagnostic_model::RouteFixtureSpec = serde_json::from_str(raw_spec)?;
        if !spec.valid() {
            return Err("invalid Core-issued route fixture".into());
        }
        no_tun_descriptors()?;
        let observed = vpnhide_checks::route::observe();
        println!("{}", serde_json::to_string(&observed.scoped(&spec))?);
        return Ok(());
    }
    if let [mode, name] = args.as_slice()
        && mode == "worker"
    {
        if unsafe { libc::geteuid() } != 10000 || name.len() != 15 || !name.starts_with("tun") {
            return Err("invalid native oracle child".into());
        }
        no_tun_descriptors()?;
        println!(
            "{}",
            serde_json::to_string(&vpnhide_checks::fixture::observe(name))?
        );
        return Ok(());
    }
    if unsafe { libc::geteuid() } != 0 {
        return Err("native oracle parent is not root".into());
    }
    if args.as_slice() == ["route-crash-owner"] {
        let fixture =
            vpnhide_core_daemon::autonomous::route_fixture::RouteFixture::create(0x0123_4567_89b2)?;
        println!("{}", serde_json::to_string(&fixture.cleanup_plan())?);
        io::stdout().flush()?;
        // Actual process-exit cleanup; Rust owners are intentionally not dropped.
        unsafe { libc::_exit(0) };
    }
    if args.as_slice() == ["crash-owner"] {
        let fixture = TunFixture::create(0x0123_4567_89ad)?;
        println!("{}", serde_json::to_string(&fixture.spec)?);
        io::stdout().flush()?;
        // Deliberately bypass Rust Drop: kernel process-exit cleanup is the oracle.
        unsafe {
            libc::_exit(0);
        }
    }
    let protected = match args.as_slice() {
        [mode] if mode == "raw" => false,
        [mode] if mode == "protected" => true,
        _ => return Err("expected raw or protected".into()),
    };
    if protected {
        let control = Path::new("/proc/vpnhide_ctl");
        let status = vpnhide_core_daemon::activation::read_status(control)?;
        let backend = match status.backend {
            value if value == vpnhide_protocol::hook_ids::Backend::Kmod as u32 => {
                vpnhide_core_rpc::NativeBackend::Kmod
            }
            value if value == vpnhide_protocol::hook_ids::Backend::Builtin as u32 => {
                vpnhide_core_rpc::NativeBackend::Builtin
            }
            _ => return Err("oracle requires kernel backend".into()),
        };
        let wire = vpnhide_protocol::format_config(
            false,
            0,
            &[vpnhide_protocol::Target {
                uid: 10000,
                hookmask: vpnhide_protocol::hook_ids::Hook::DevIoctl.bit()
                    | vpnhide_protocol::hook_ids::Hook::RtnlFillIfinfo.bit()
                    | vpnhide_protocol::hook_ids::Hook::InetFillIfaddr.bit()
                    | vpnhide_protocol::hook_ids::Hook::Inet6FillIfaddr.bit()
                    | vpnhide_protocol::hook_ids::Hook::SockIoctl.bit()
                    | vpnhide_protocol::hook_ids::Hook::SocketBindInterface.bit()
                    | vpnhide_protocol::hook_ids::Hook::FilesystemIfacePaths.bit()
                    | vpnhide_protocol::hook_ids::Hook::FibRouteSeqShow.bit()
                    | vpnhide_protocol::hook_ids::Hook::Ipv6RouteSeqShow.bit()
                    | vpnhide_protocol::hook_ids::Hook::FibDumpInfo.bit()
                    | vpnhide_protocol::hook_ids::Hook::Rt6FillNode.bit(),
            }],
        );
        vpnhide_core_daemon::activation::deliver_native(control, backend, &wire)?;
    } else if Path::new("/proc/vpnhide_ctl").exists() {
        return Err("raw oracle cannot have a native backend".into());
    }
    let before = routes()?;
    let fixture = TunFixture::create(0x0123_4567_89ab)?;
    println!("FIXTURE_SPEC={}", serde_json::to_string(&fixture.spec)?);
    println!("FIXTURE_ADDRESS_SETUP={:?}", fixture.address_setup);
    println!(
        "FIXTURE_ROOT_START={}",
        serde_json::to_string(&fixture.root_start)?
    );
    if protected {
        raw_control::verify(&fixture.spec)?;
    }
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(["worker", fixture.spec.interface.name.as_str()])
        .stderr(Stdio::inherit());
    // SAFETY: only an async-signal-safe credential syscall runs before exec.
    unsafe {
        command.pre_exec(|| {
            let inet: libc::gid_t = 3003;
            if libc::setgroups(1, &inet) != 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::setgid(10000) != 0 || libc::setuid(10000) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let output = vpnhide_activator::output_with_timeout(&mut command, Duration::from_secs(5))?;
    if !output.status.success() {
        return Err("native fixture child failed".into());
    }
    let observation: NativeMeasurement = serde_json::from_slice(&output.stdout)?;
    println!("FIXTURE_WORKER={}", serde_json::to_string(&observation)?);
    let done = fixture.finish(&observation);
    println!("FIXTURE_RESULT={}", serde_json::to_string(&done.result)?);
    println!(
        "FIXTURE_BIND_RESULT={}",
        serde_json::to_string(&done.binding)?
    );
    println!(
        "FIXTURE_ADDRESS_RESULT={}",
        serde_json::to_string(&done.addresses)?
    );
    println!(
        "FIXTURE_PROC_RESULT={}",
        serde_json::to_string(&done.proc_interfaces)?
    );
    println!(
        "FIXTURE_SYS_RESULT={}",
        serde_json::to_string(&done.sys_interfaces)?
    );
    let sys_expected = if protected {
        done.sys_interfaces.complete()
    } else {
        done.sys_interfaces.controls.is_empty()
            && done.sys_interfaces.proven.is_empty()
            && SysInterfaceCheck::required()
                .iter()
                .all(|c| done.sys_interfaces.leaked.contains(c))
    };
    println!(
        "FIXTURE_FILESYSTEM_RESULT={}",
        serde_json::to_string(&done.filesystem)?
    );
    let filesystem_expected = if protected {
        done.filesystem.complete()
    } else {
        done.filesystem.controls.is_empty()
            && done.filesystem.proven.is_empty()
            && FilesystemCheck::required()
                .iter()
                .all(|c| done.filesystem.leaked.contains(c))
    };
    // Current kernel backends do not filter these seq files. This independently
    // tests leak detection; it is not a claim that the product hides procfs.
    let proc_expected = done.proc_interfaces.proven.is_empty()
        && done.proc_interfaces.controls.is_empty()
        && ProcInterfaceCheck::required()
            .iter()
            .all(|c| done.proc_interfaces.leaked.contains(c));
    let addresses_expected = if protected {
        done.addresses.complete()
    } else {
        done.addresses.controls.is_empty()
            && AddressCheck::required()
                .iter()
                .all(|v| done.addresses.leaked.contains(v))
    };
    let bind_expected = if protected {
        done.binding.complete()
    } else {
        done.binding.controls.is_empty()
            && BindActor::ALL
                .iter()
                .all(|a| done.binding.leaked.contains(a))
    };
    let expected = if protected {
        matches!(&done.result, ScopedNativeResult::Proven { checks } if checks.len() == 4)
    } else {
        matches!(&done.result, ScopedNativeResult::Leaked { checks, incomplete } if checks.len() == 4 && incomplete.is_empty())
    };
    if !expected
        || !bind_expected
        || !addresses_expected
        || !proc_expected
        || !sys_expected
        || !filesystem_expected
        || routes()? != before
    {
        return Err("native differential/control/cleanup assertion failed".into());
    }
    let duplicated = TunFixture::create(0x0123_4567_89ac)?;
    let spec = duplicated.spec.clone();
    let last_fd = duplicated.duplicate_for_oracle()?;
    let raw = vpnhide_checks::fixture::observe(&spec.interface.name);
    let incomplete = duplicated.finish(&raw);
    if cleanup_confirmed(&spec, &incomplete.cleanup)
        || !matches!(incomplete.result, ScopedNativeResult::Leaked { incomplete, .. } if incomplete.contains(&ControlFailure::CleanupUnconfirmed))
        || !incomplete.addresses.proven.is_empty()
        || !incomplete
            .addresses
            .controls
            .contains(&ControlFailure::CleanupUnconfirmed)
        || !AddressCheck::required()
            .iter()
            .all(|v| incomplete.addresses.leaked.contains(v))
        || !incomplete.proc_interfaces.proven.is_empty()
        || !incomplete
            .proc_interfaces
            .controls
            .contains(&ControlFailure::CleanupUnconfirmed)
        || incomplete.proc_interfaces.leaked.len() != 4
        || incomplete.sys_interfaces.leaked.len() != 6
        || !incomplete.sys_interfaces.proven.is_empty()
        || !incomplete
            .sys_interfaces
            .controls
            .contains(&ControlFailure::CleanupUnconfirmed)
        || incomplete.filesystem.leaked.len() != 18
        || !incomplete.filesystem.proven.is_empty()
        || !incomplete
            .filesystem
            .controls
            .contains(&ControlFailure::CleanupUnconfirmed)
    {
        return Err("last-dup control falsely confirmed cleanup".into());
    }
    drop(last_fd);
    if !cleanup_confirmed(
        &spec,
        &vpnhide_checks::fixture::observe(&spec.interface.name),
    ) {
        return Err("last duplicate did not release fixture".into());
    }
    println!("FIXTURE_DUP_CONTROL=PASS");
    let mut crash = Command::new(std::env::current_exe()?);
    crash.arg("crash-owner");
    let crashed = vpnhide_activator::output_with_timeout(&mut crash, Duration::from_secs(5))?;
    if !crashed.status.success() {
        return Err("crash-owner failed before its witness".into());
    }
    let crash_spec: NativeFixtureSpec = serde_json::from_slice(&crashed.stdout)?;
    if !cleanup_confirmed(
        &crash_spec,
        &vpnhide_checks::fixture::observe(&crash_spec.interface.name),
    ) || routes()? != before
    {
        return Err("process-exit cleanup/control failed".into());
    }
    println!("FIXTURE_CRASH_CONTROL=PASS");
    let failed = TunFixture::failed_setup_for_oracle(0x0123_4567_89ae);
    if !failed.resource_created || !failed.cleanup.is_some_and(|(_, cleaned)| cleaned) {
        return Err("partial-setup cleanup was not positively confirmed".into());
    }
    println!("FIXTURE_SETUP_FAILURE_CONTROL=PASS");
    let partial = TunFixture::partial_address_setup_for_oracle(0x0123_4567_89af)?;
    if partial.address_setup.is_ok() {
        return Err("partial address fault did not fire".into());
    }
    let partial_spec = partial.spec.clone();
    let raw = vpnhide_checks::fixture::observe(&partial_spec.interface.name);
    let completed = partial.finish(&raw);
    if completed.addresses.complete()
        || completed.addresses.leaked.is_empty()
        || !completed.addresses.proven.is_empty()
        || !completed
            .addresses
            .controls
            .contains(&ControlFailure::MissingPositiveControl)
        || !cleanup_confirmed(&partial_spec, &completed.cleanup)
        || completed.proc_interfaces.leaked.len() != 2
        || !completed.proc_interfaces.proven.is_empty()
        || !completed
            .proc_interfaces
            .controls
            .contains(&ControlFailure::MissingPositiveControl)
        || routes()? != before
    {
        return Err("partial address setup leaked resources or falsely proved coverage".into());
    }
    println!("FIXTURE_PARTIAL_ADDRESS_CONTROL=PASS");
    route_control::verify(protected)?;
    if routes()? != before {
        return Err("route prototype did not restore initial proc observations".into());
    }

    println!(
        "FIXTURE_ORACLE=PASS mode={} interface_fixture_down=1 route_fixture_up_pair=1 routes_restored=1",
        if protected { "protected" } else { "raw" }
    );
    Ok(())
}

fn no_tun_descriptors() -> io::Result<()> {
    for entry in std::fs::read_dir("/proc/self/fd")? {
        let metadata = std::fs::metadata(entry?.path())?;
        if metadata.file_type().is_char_device() && metadata.rdev() == 0x0ac8 {
            return Err(io::Error::other("TUN descriptor leaked across worker exec"));
        }
    }
    Ok(())
}
