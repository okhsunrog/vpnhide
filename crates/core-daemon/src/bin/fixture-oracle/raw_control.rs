//! Isolated Linux oracle only. Production admits genuine Zygote workers through
//! the coordinator; this setuid child is not that Android authentication path.
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use vpnhide_core_daemon::autonomous::native_raw::RawLease;
use vpnhide_diagnostic_model::{NativeFixtureSpec, NativeMeasurement, ProbeError, Sample};

fn line(reader: &mut impl BufRead) -> io::Result<String> {
    let mut text = String::new();
    reader.take(2 * 1024 * 1024 + 1).read_line(&mut text)?;
    if text.len() > 2 * 1024 * 1024 || !text.ends_with('\n') {
        return Err(io::Error::other("oracle IPC frame unavailable/oversize"));
    }
    Ok(text.trim_end_matches('\n').to_owned())
}
fn say(writer: &mut impl Write, text: &str) -> io::Result<()> {
    writeln!(writer, "{text}")?;
    writer.flush()
}
fn require(condition: bool, message: &str) -> io::Result<()> {
    if condition {
        Ok(())
    } else {
        Err(io::Error::other(message))
    }
}
fn directories(
    value: &vpnhide_diagnostic_model::SysInterfaceMeasurement,
    name: &str,
    present: bool,
) -> bool {
    value.bounded()
        && vpnhide_diagnostic_model::SysInterfaceCheck::required()
            .iter()
            .all(|check| {
                value
                    .entries
                    .iter()
                    .find(|e| e.check == *check)
                    .and_then(|e| e.names.value())
                    .is_some_and(|names| {
                        names.iter().any(|n| n == name) == present
                            && names.iter().any(|n| n == "lo")
                    })
            })
}
fn lookups(value: &vpnhide_diagnostic_model::FilesystemMeasurement, present: bool) -> bool {
    value.bounded()
        && vpnhide_diagnostic_model::FilesystemCheck::required()
            .iter()
            .all(|check| {
                value
                    .entries
                    .iter()
                    .find(|e| e.check == *check)
                    .is_some_and(|e| {
                        e.fixture == Sample::Value(present) && e.ordinary == Sample::Value(true)
                    })
            })
}
pub fn worker(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    require(unsafe { libc::geteuid() } == 10000, "not oracle app UID")?;
    require(name.len() == 15 && name.starts_with("tun"), "bad fixture")?;
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    say(&mut writer, &unsafe { libc::gettid() }.to_string())?;
    let nonce: u64 = line(&mut reader)?.parse()?;
    // Unprivileged callers cannot even open the private control channel.
    require(
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(vpnhide_protocol::diagnostic_raw::NODE)
            .is_err(),
        "root control exposed",
    )?;
    vpnhide_checks::diagnostic_raw::register(nonce)?;
    say(&mut writer, "registered")?;
    require(line(&mut reader)? == "raw", "expected RAW command")?;
    let name = name.to_owned();
    let ordinary_thread = std::thread::spawn({
        let name = name.clone();
        move || {
            (
                vpnhide_checks::fixture::interface_ioctl(&name, true),
                vpnhide_checks::sys_interfaces::observe(),
                vpnhide_checks::filesystem::observe(&name),
            )
        }
    })
    .join()
    .map_err(|_| "ordinary thread failed")?;
    require(
        ordinary_thread.0 == Sample::Error(ProbeError::Errno(libc::ENODEV))
            && directories(&ordinary_thread.1, &name, false)
            && lookups(&ordinary_thread.2, false),
        "RAW escaped to another thread of the same UID",
    )?;
    say(
        &mut writer,
        &serde_json::to_string(&vpnhide_checks::fixture::observe(&name))?,
    )?;
    require(
        line(&mut reader)? == "protected",
        "expected confirmed revoke",
    )?;
    say(
        &mut writer,
        &serde_json::to_string(&vpnhide_checks::fixture::observe(&name))?,
    )?;
    for phase in ["closed", "expired"] {
        let next_nonce: u64 = line(&mut reader)?.parse()?;
        vpnhide_checks::diagnostic_raw::register(next_nonce)?;
        say(&mut writer, "registered")?;
        require(line(&mut reader)? == phase, "expected cleanup proof")?;
        say(
            &mut writer,
            &serde_json::to_string(&vpnhide_checks::fixture::interface_ioctl(&name, true))?,
        )?;
    }
    Ok(())
}
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
pub fn verify(spec: &NativeFixtureSpec) -> Result<(), Box<dyn std::error::Error>> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(["raw-thread-worker", spec.interface.name.as_str()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
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
    let mut child = ChildGuard(command.spawn()?);
    let mut writer = child.0.stdin.take().ok_or("child stdin")?;
    let mut reader = BufReader::new(child.0.stdout.take().ok_or("child stdout")?);
    let tid: u32 = line(&mut reader)?.parse()?;
    let nonce = 0x0123456789abcdef;
    let status =
        vpnhide_core_daemon::activation::read_status(std::path::Path::new("/proc/vpnhide_ctl"))?;
    let mut lease = RawLease::arm(status.backend, tid, 10000, nonce)?;
    say(&mut writer, &nonce.to_string())?;
    require(line(&mut reader)? == "registered", "registration ACK")?;
    lease.activate()?;
    say(&mut writer, "raw")?;
    let raw: NativeMeasurement = serde_json::from_str(&line(&mut reader)?)?;
    require(lease.finish()?, "RAW expired before measurement completed")?;
    say(&mut writer, "protected")?;
    let protected: NativeMeasurement = serde_json::from_str(&line(&mut reader)?)?;
    require(
        raw.filesystem.as_ref().is_some_and(|v| lookups(v, true))
            && protected
                .filesystem
                .as_ref()
                .is_some_and(|v| lookups(v, false)),
        "lookup RAW/protected/ordinary controls failed",
    )?;
    println!(
        "RAW_THREAD_FILESYSTEM_OBSERVATIONS={}",
        serde_json::json!({"raw":raw.filesystem,"protected":protected.filesystem})
    );
    require(
        raw.sys_interfaces
            .as_ref()
            .is_some_and(|v| directories(v, &spec.interface.name, true))
            && protected
                .sys_interfaces
                .as_ref()
                .is_some_and(|v| directories(v, &spec.interface.name, false)),
        "directory RAW/protected/ordinary controls failed",
    )?;
    println!(
        "RAW_THREAD_SYS_OBSERVATIONS={}",
        serde_json::json!({"raw":raw.sys_interfaces,"protected":protected.sys_interfaces})
    );
    require(
        raw.flags == Sample::Value(spec.flags) && raw.mtu == Sample::Value(spec.mtu),
        "app RAW fixture stimulus missing",
    )?;
    require(
        protected.flags == Sample::Error(ProbeError::Errno(libc::ENODEV))
            && protected.mtu == Sample::Error(ProbeError::Errno(libc::ENODEV)),
        "protection not restored",
    )?;
    require(
        raw.ordinary_flags == protected.ordinary_flags
            && raw.ordinary_mtu == protected.ordinary_mtu
            && matches!(raw.ordinary_mtu, Sample::Value(_)),
        "ordinary objects changed",
    )?;
    // Last owning FD closes while RAW is ACTIVE, simulating Core destruction.
    // Then leave a separate active lease open through the fixed kernel TTL.
    require(
        RawLease::arm(status.backend, tid, 10000, nonce).is_err(),
        "nonce resurrected",
    )?;
    for (phase, token) in [("closed", nonce + 1), ("expired", nonce + 2)] {
        let mut owner = RawLease::arm(status.backend, tid, 10000, token)?;
        say(&mut writer, &token.to_string())?;
        require(
            line(&mut reader)? == "registered",
            "cleanup registration ACK",
        )?;
        owner.activate()?;
        if phase == "closed" {
            owner.close_without_revoke_for_oracle();
            say(&mut writer, phase)?;
            let restored: Sample<u32> = serde_json::from_str(&line(&mut reader)?)?;
            require(
                restored == Sample::Error(ProbeError::Errno(libc::ENODEV)),
                "active FD close did not revoke",
            )?;
        } else {
            std::thread::sleep(std::time::Duration::from_millis(
                u64::from(vpnhide_protocol::diagnostic_raw::MAX_LEASE_MS) + 25,
            ));
            // Observe BEFORE any root readback, revoke or FD close. The hot
            // path must reject the expired identity on its own.
            say(&mut writer, phase)?;
            let restored: Sample<u32> = serde_json::from_str(&line(&mut reader)?)?;
            require(
                restored == Sample::Error(ProbeError::Errno(libc::ENODEV)),
                "expired hot-path RAW remained active",
            )?;
            require(
                !owner.finish()?,
                "expired lease falsely claims completed RAW proof",
            )?;
        }
    }
    require(child.0.wait()?.success(), "RAW collector failed")?;
    println!(
        "RAW_THREAD_ORACLE=PASS same_thread_raw=1 same_uid_other_thread_protected=1 revoke=1 ordinary_preserved=1 active_close=1 ttl_expiry=1 no_nonce_replay=1"
    );
    Ok(())
}
