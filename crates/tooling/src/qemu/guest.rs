use super::model::{self, Backend, CountMode, Mode, Plan, Report, ResultRow, Verdict};
use clap::ValueEnum;
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read, Write},
    os::unix::process::CommandExt,
    path::Path,
    process::Command,
    time::Duration,
};
use vpnhide_artifact_model::{
    archive::{FileIdentity, read_regular},
    elf,
};
use vpnhide_bounded_process::{CapturedCommand, capture_command};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Observation {
    IpAddr,
    IfconfigAll,
    IfconfigVpn,
    IfconfigOrdinary,
    ProcRoute4,
    ProcRoute6,
    Route4,
    Route6,
    Rules,
    SysfsStat,
    SysfsOpen,
    SysfsList,
    ProcSysStat,
    ProcSysList,
}
fn execute(program: &str, args: &[&str], as_app: bool) -> io::Result<CapturedCommand> {
    let program = if program == "ip" && Path::new("/vpnhide-ip").is_file() {
        "/vpnhide-ip"
    } else {
        program
    };
    let mut c = Command::new(program);
    c.args(args);
    if as_app {
        // Linux test actor only. This does not impersonate an Android Zygote
        // worker and supplies no Android/SELinux acceptance evidence.
        unsafe {
            c.pre_exec(|| {
                let groups = [model::INET_GID as libc::gid_t];
                if libc::setgroups(1, groups.as_ptr()) != 0
                    || libc::setgid(model::INET_GID) != 0
                    || libc::setuid(model::TARGET_UID) != 0
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    capture_command(
        &mut c,
        None,
        Duration::from_secs(match program {
            "apk" => 120,
            "/native-fixture-oracle" => 90,
            _ => 20,
        }),
        256 * 1024,
    )
}
fn success(c: &CapturedCommand) -> bool {
    !c.timed_out && !c.truncated && c.output.status.success()
}
fn checked(program: &str, args: &[&str]) -> io::Result<Vec<u8>> {
    let c = execute(program, args, false)?;
    if !success(&c) {
        return Err(io::Error::other(format!(
            "guest setup command failed: {program} {args:?}"
        )));
    }
    Ok(c.output.stdout)
}
fn read_pseudo(path: &str, limit: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(io::Error::other("kernel observation exceeds budget"));
    }
    Ok(bytes)
}
fn set_target(uid: u32) -> io::Result<()> {
    use vpnhide_protocol::{
        Target,
        hook_ids::{KERNEL_HOOK_MASK, KMOD_HOOK_MASK},
    };
    let wire = vpnhide_protocol::format_config(
        true,
        0,
        &[Target {
            uid,
            hookmask: KERNEL_HOOK_MASK | KMOD_HOOK_MASK,
        }],
    );
    fs::write("/proc/vpnhide_ctl", wire)
}
fn field_output(program: &str, args: &[&str], as_app: bool) -> Option<BTreeMap<String, i64>> {
    let c = execute(program, args, as_app).ok()?;
    if !success(&c) {
        return None;
    }
    model::fields(&c.output.stdout).ok()
}
type NativeFields = Option<BTreeMap<String, i64>>;
fn pair_fields(
    program: &str,
    args: &[&str],
    as_app: bool,
) -> io::Result<(NativeFields, NativeFields)> {
    set_target(model::BASELINE_UID)?;
    let raw = field_output(program, args, as_app);
    set_target(model::TARGET_UID)?;
    let protected = field_output(program, args, as_app);
    Ok((raw, protected))
}
fn value(fields: &Option<BTreeMap<String, i64>>, key: &str) -> Option<i64> {
    fields.as_ref()?.get(key).copied()
}
fn bind_value(fields: &Option<BTreeMap<String, i64>>, key: &str) -> Option<model::Binding> {
    model::binding(fields.as_ref()?, key)
}
fn counts(kind: Observation, needle: &str, starts_with: bool) -> Option<i64> {
    let child = execute(
        "/vpnhide-tooling",
        &["qemu", "observe", kind.to_possible_value()?.get_name()],
        true,
    )
    .ok()?;
    if !success(&child) {
        eprintln!(
            "OBSERVATION_ERROR kind={kind:?} stderr={}",
            String::from_utf8_lossy(&child.output.stderr)
        );
        return None;
    }
    let text = std::str::from_utf8(&child.output.stdout).ok()?;
    Some(
        text.lines()
            .filter(|line| {
                if starts_with {
                    line.starts_with(needle)
                } else {
                    line.contains(needle)
                }
            })
            .count() as i64,
    )
}
fn check_counts(
    report: &mut Report,
    name: &str,
    kind: Observation,
    needle: &str,
    starts_with: bool,
    mode: CountMode,
) -> io::Result<()> {
    set_target(model::BASELINE_UID)?;
    let raw = counts(kind, needle, starts_with);
    set_target(model::TARGET_UID)?;
    let target = counts(kind, needle, starts_with);
    report.rows.push(model::count(name, mode, raw, target));
    Ok(())
}
fn path_observation(path: &str, kind: Observation) -> io::Result<String> {
    match kind {
        Observation::SysfsStat | Observation::ProcSysStat => match fs::metadata(path) {
            Ok(_) => Ok("vpn0\n".to_string()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(e),
        },
        Observation::SysfsOpen => match fs::read(path) {
            Ok(bytes) if !bytes.is_empty() => Ok("vpn0\n".into()),
            Ok(_) => Err(io::Error::other("empty interface property")),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(e),
        },
        Observation::SysfsList | Observation::ProcSysList => {
            let mut names = Vec::new();
            for e in fs::read_dir(path)? {
                let e = e?;
                let name = e
                    .file_name()
                    .into_string()
                    .map_err(|_| io::Error::other("non-UTF8 interface name"))?;
                names.push(name);
            }
            names.sort();
            Ok(names.join("\n") + "\n")
        }
        _ => Err(io::Error::other("invalid path observation")),
    }
}
fn expected_ifconfig_denial(
    kind: Observation,
    code: Option<i32>,
    stdout: &[u8],
    stderr: &[u8],
) -> bool {
    if code != Some(1)
        || !matches!(kind, Observation::IfconfigVpn | Observation::IfconfigAll)
        || (matches!(kind, Observation::IfconfigVpn) && !stdout.is_empty())
    {
        return false;
    }
    let Ok(error) = std::str::from_utf8(stderr) else {
        return false;
    };
    let lines: Vec<_> = error.lines().collect();
    !lines.is_empty()
        && lines.iter().all(|line| {
            matches!(
                *line,
                "ifconfig: vpn0: error fetching interface information: Device not found"
                    | "ifconfig: vpn0: error fetching interface information: No such device"
            )
        })
}
/// Executed as the actual unprivileged Linux actor. Paths are an enum, never
/// caller-chosen shell text. Missing objects and failed utilities stay distinct.
pub fn observe(kind: Observation) -> io::Result<()> {
    let path = match kind {
        Observation::SysfsStat => Some("/sys/class/net/vpn0"),
        Observation::SysfsOpen => Some("/sys/class/net/vpn0/mtu"),
        Observation::SysfsList => Some("/sys/class/net"),
        Observation::ProcSysStat => Some("/proc/sys/net/ipv4/conf/vpn0"),
        Observation::ProcSysList => Some("/proc/sys/net/ipv4/conf"),
        _ => None,
    };
    if let Some(path) = path {
        print!("{}", path_observation(path, kind)?);
        return Ok(());
    }
    if matches!(kind, Observation::ProcRoute4 | Observation::ProcRoute6) {
        print!(
            "{}",
            fs::read_to_string(if matches!(kind, Observation::ProcRoute4) {
                "/proc/net/route"
            } else {
                "/proc/net/ipv6_route"
            })?
        );
        return Ok(());
    }
    let (program, args): (&str, &[&str]) = match kind {
        Observation::IpAddr => ("ip", &["addr", "show"]),
        Observation::IfconfigAll => ("ifconfig", &["-a"]),
        Observation::IfconfigVpn => ("ifconfig", &["vpn0"]),
        Observation::IfconfigOrdinary => ("ifconfig", &["eth0"]),
        Observation::Route4 => ("ip", &["route", "show", "table", "all"]),
        Observation::Route6 => ("ip", &["-6", "route", "show", "table", "all"]),
        Observation::Rules => ("ip", &["rule", "show"]),
        _ => return Err(io::Error::other("invalid utility observation")),
    };
    let c = execute(program, args, false)?;
    if !success(&c) {
        let error = String::from_utf8_lossy(&c.output.stderr);
        if !c.timed_out
            && !c.truncated
            && expected_ifconfig_denial(
                kind,
                c.output.status.code(),
                &c.output.stdout,
                &c.output.stderr,
            )
        {
            return io::stdout().write_all(&c.output.stdout);
        }
        return Err(io::Error::other(format!(
            "observation utility failed: {program} {args:?}: {error}"
        )));
    }
    io::stdout().write_all(&c.output.stdout)
}
fn setup() -> io::Result<()> {
    // Preserve the original iproute2 utility coverage. Host can provide an
    // already prepared rootfs; a failed package setup is never an empty PASS.
    checked("ip", &["link", "set", "eth0", "up"])?;
    checked("ip", &["addr", "add", "10.0.2.15/24", "dev", "eth0"])?;
    checked("ip", &["route", "add", "default", "via", "10.0.2.2"])?;
    fs::write("/etc/resolv.conf", "nameserver 10.0.2.3\n")?;
    let version = execute("ip", &["-Version"], false)?;
    if !success(&version) || !String::from_utf8_lossy(&version.output.stdout).contains("iproute2") {
        fs::write(
            "/etc/apk/repositories",
            "https://dl-cdn.alpinelinux.org/alpine/v3.21/main\n",
        )?;
        checked("apk", &["add", "--no-cache", "iproute2"])?;
    }
    println!("IPROUTE2=ok");
    for args in [
        vec!["link", "add", "vpn0", "type", "dummy"],
        vec!["link", "set", "vpn0", "up"],
        vec!["addr", "add", "10.9.0.1/24", "dev", "vpn0"],
        vec!["route", "add", "10.9.9.0/24", "dev", "vpn0"],
        vec!["-6", "addr", "add", "fd00:9::1/64", "dev", "vpn0"],
        vec!["-6", "route", "add", "fd00:99::/64", "dev", "vpn0"],
        vec!["rule", "add", "uidrange", "10000-10000", "table", "199"],
        vec!["route", "add", "1.2.3.4/32", "dev", "eth0"],
        vec![
            "-6",
            "route",
            "add",
            "2001:4860:4860::8888/128",
            "dev",
            "eth0",
        ],
    ] {
        checked("ip", &args)?;
    }
    Ok(())
}
fn preflight(plan: &Plan, report: &mut Report) -> io::Result<()> {
    let bytes = read_regular(
        Path::new("/vpnhide_kmod.ko"),
        vpnhide_artifact_model::MAX_PAYLOAD_BYTES,
    )?;
    if Some(FileIdentity::of(&bytes).sha256) != plan.payload_sha256 {
        return Err(io::Error::other("guest preflight payload identity changed"));
    }
    let caps = elf::capabilities(&bytes)?;
    if caps.preflight_protocol != Some(1) || caps.init_contract != Some(1) {
        return Err(io::Error::other(
            "unverified preflight/closed-init protocol",
        ));
    }
    if !Path::new("/sys/kernel/debug/kprobes/list").exists() {
        checked("mount", &["-t", "debugfs", "debugfs", "/sys/kernel/debug"])?;
    }
    let absent = || {
        [
            "/proc/vpnhide_ctl",
            "/proc/vpnhide_diag",
            "/sys/module/vpnhide_kmod",
        ]
        .iter()
        .all(|p| !Path::new(p).exists())
    };
    if !absent() {
        return Err(io::Error::other("preflight preconditions are not empty"));
    }
    let before = read_regular(Path::new("/sys/kernel/debug/kprobes/list"), 1024 * 1024)?;
    let c = execute("/preflight-probe", &["/vpnhide_kmod.ko"], false)?;
    let after = read_regular(Path::new("/sys/kernel/debug/kprobes/list"), 1024 * 1024)?;
    if !success(&c) || !absent() || before != after {
        return Err(io::Error::other(
            "preflight sentinel/postconditions not confirmed; no real-load dispatch",
        ));
    }
    io::stdout().write_all(&c.output.stdout)?;
    report.rows.push(ResultRow::new(
        "loader_preflight",
        Verdict::Pass,
        "hash-bound exact sentinel and unchanged module/proc/kprobes",
    ));
    Ok(())
}
fn load(plan: &Plan, report: &mut Report) -> io::Result<()> {
    if plan.backend == Backend::Builtin {
        report.loaded = Path::new("/proc/vpnhide_ctl").exists();
        report.registered = u32::from(report.loaded);
    } else if plan.backend == Backend::Kmod {
        let bytes = read_regular(
            Path::new("/vpnhide_kmod.ko"),
            vpnhide_artifact_model::MAX_PAYLOAD_BYTES,
        )?;
        if Some(FileIdentity::of(&bytes).sha256) != plan.payload_sha256 {
            return Err(io::Error::other("real-load payload identity changed"));
        }
        let caps = elf::capabilities(&bytes)?;
        if caps.init_contract != Some(1) {
            return Err(io::Error::other("real-load closed-init capability missing"));
        }
        if plan.probes.preflight.present {
            preflight(plan, report)?;
        }
        checked("insmod", &["/vpnhide_kmod.ko", "filesystem_hiding=1"])?;
        let text = String::from_utf8(checked("dmesg", &[])?).map_err(io::Error::other)?;
        report.registered = text
            .lines()
            .filter(|l| l.contains("vpnhide:") && l.contains("registered"))
            .count() as u32;
        report.loaded = Path::new("/proc/vpnhide_ctl").exists();
    } else {
        report.loaded =
            String::from_utf8_lossy(&checked("dmesg", &[])?).contains("KPM hooks installed");
    }
    println!(
        "INSMOD={} REGISTERED={}",
        if report.loaded { "ok" } else { "FAIL" },
        report.registered
    );
    if !report.loaded {
        return Err(io::Error::other("backend did not load"));
    }
    Ok(())
}

pub fn count_cases() -> Vec<(&'static str, Observation, &'static str, bool, CountMode)> {
    let mut cases: Vec<_> = [
        ("getifaddrs", Observation::IpAddr, "vpn0", false),
        ("siocgifconf", Observation::IfconfigAll, "vpn0", false),
        ("dev_ioctl", Observation::IfconfigVpn, "vpn0", false),
        ("proc_route_v4", Observation::ProcRoute4, "vpn0", false),
        ("proc_route_v6", Observation::ProcRoute6, "vpn0", false),
        ("netlink_route4", Observation::Route4, "vpn0", false),
        ("hostroute4", Observation::Route4, "1.2.3.4", false),
        ("netlink_route6", Observation::Route6, "vpn0", false),
        ("hostroute6", Observation::Route6, "2001:4860", false),
        ("policy_rule", Observation::Rules, "199", false),
        ("sysfs_stat", Observation::SysfsStat, "vpn0", false),
        ("sysfs_readdir", Observation::SysfsList, "vpn0", false),
        ("sysfs_open", Observation::SysfsOpen, "vpn0", false),
        ("proc_sys_stat", Observation::ProcSysStat, "vpn0", false),
        ("proc_sys_readdir", Observation::ProcSysList, "vpn0", false),
    ]
    .into_iter()
    .map(|(n, k, v, s)| (n, k, v, s, CountMode::Hide))
    .collect();
    cases.extend(
        [
            ("keep_proc_route_v4", Observation::ProcRoute4, "eth0", true),
            ("keep_getifaddrs", Observation::IpAddr, ": eth0:", false),
            ("keep_siocgifconf", Observation::IfconfigAll, "eth0", true),
            (
                "keep_dev_ioctl",
                Observation::IfconfigOrdinary,
                "eth0",
                true,
            ),
            (
                "keep_netlink_route4",
                Observation::Route4,
                "dev eth0",
                false,
            ),
            ("keep_policy_rule", Observation::Rules, "lookup main", false),
            ("keep_sysfs_readdir", Observation::SysfsList, "eth0", false),
            (
                "keep_proc_sys_readdir",
                Observation::ProcSysList,
                "eth0",
                false,
            ),
        ]
        .into_iter()
        .map(|(n, k, v, s)| {
            (
                n,
                k,
                v,
                s,
                if n == "keep_netlink_route4" {
                    CountMode::KeepPositive
                } else {
                    CountMode::KeepExact
                },
            )
        }),
    );
    cases
}
fn vectors(plan: &Plan, report: &mut Report) -> io::Result<()> {
    for (name, kind, needle, start, mode) in count_cases() {
        check_counts(report, name, kind, needle, start, mode)?;
    }
    if plan.probes.gai.present {
        let (r, t) = pair_fields("/gai", &[], true)?;
        report.rows.push(model::count(
            "gai_getifaddrs",
            CountMode::Hide,
            value(&r, "GAI_VPN0"),
            value(&t, "GAI_VPN0"),
        ));
        report.rows.push(model::count(
            "keep_gai_getifaddrs",
            CountMode::KeepPositive,
            value(&r, "GAI_OTHER"),
            value(&t, "GAI_OTHER"),
        ));
    }
    if plan.probes.ifconf.present {
        let (r, t) = pair_fields("/ifconf", &[], true)?;
        report.rows.push(model::ifconf_pair(
            "ifconf_probe",
            r.as_ref().and_then(model::ifconf),
            t.as_ref().and_then(model::ifconf),
        ));
    }
    set_target(model::BASELINE_UID)?;
    let index = read_pseudo("/sys/class/net/vpn0/ifindex", 64)?;
    let index = std::str::from_utf8(&index)
        .map_err(io::Error::other)?
        .trim()
        .to_owned();
    if index.parse::<u32>().ok().is_none_or(|v| v == 0) {
        return Err(io::Error::other("fixture ifindex missing"));
    }
    if plan.probes.bind.present {
        // Root launches this C binary: it creates independent actor/observer
        // processes and reads the same socket after the actor's mutation.
        let (r, t) = pair_fields("/bind-probe", &["vpn0", &index], false)?;
        let absent = bind_value(&r, "BIND_ABSENT");
        for (name, prefix) in [
            ("bind_device_raw", "BIND_NAME_RAW"),
            ("bind_device_nul", "BIND_NAME_NUL"),
            ("bind_ifindex", "BIND_INDEX"),
        ] {
            report.rows.push(model::hidden_bind(
                name,
                bind_value(&r, prefix),
                bind_value(&t, prefix),
                absent,
            ));
        }
        report.rows.push(model::bad_bind(
            "bind_bad_pointer",
            bind_value(&r, "BIND_BADPTR"),
            bind_value(&t, "BIND_BADPTR"),
            false,
        ));
        report.rows.push(model::bad_bind(
            "bind_bad_length",
            bind_value(&r, "BIND_BADLEN"),
            bind_value(&t, "BIND_BADLEN"),
            true,
        ));
        report.rows.push(model::preserved_bind(
            bind_value(&r, "BIND_KEEP"),
            bind_value(&t, "BIND_KEEP"),
        ));
        report.rows.push(model::absent_bind(absent));
        if plan.backend != Backend::Kpm && plan.probes.compat.present {
            let (r32, t32) = pair_fields("/bind-probe32", &["vpn0", &index], false)?;
            report.rows.push(model::hidden_bind(
                "bind_device_compat",
                bind_value(&r32, "BIND_NAME_RAW"),
                bind_value(&t32, "BIND_NAME_RAW"),
                absent,
            ));
        }
    }
    if plan.probes.ioctl.present {
        let (r, t) = pair_fields("/iface-ioctl", &["vpn0"], false)?;
        for (name, key) in [
            ("ioctl_hwaddr", "HWADDR_ERRNO"),
            ("ioctl_addr", "ADDR_ERRNO"),
        ] {
            report
                .rows
                .push(model::ioctl(name, value(&r, key), value(&t, key)));
        }
    }
    if plan.backend != Backend::Kpm {
        let bytes = read_regular(Path::new("/proc/vpnhide_ctl"), 1024 * 1024)?;
        let stats = vpnhide_protocol::parse_stats(&bytes);
        let hits = stats.as_ref().is_some_and(|s| {
            s.iter().any(|v| {
                v.uid == model::TARGET_UID
                    && v.hook_id == vpnhide_protocol::hook_ids::Hook::FilesystemIfacePaths as u32
                    && v.count > 0
            })
        });
        report.rows.push(ResultRow::new(
            "filesystem_stats",
            if hits { Verdict::Pass } else { Verdict::Fail },
            "filesystem hook telemetry for the test UID",
        ));
        let permanent = if plan.backend == Backend::Builtin {
            Path::new("/proc/vpnhide_ctl").exists()
        } else {
            let c = execute("rmmod", &["vpnhide_kmod"], false)?;
            !c.timed_out
                && !c.truncated
                && !c.output.status.success()
                && Path::new("/sys/module/vpnhide_kmod").is_dir()
                && Path::new("/proc/vpnhide_ctl").exists()
        };
        report.rows.push(ResultRow::new(
            "permanent_module",
            if permanent {
                Verdict::Pass
            } else {
                Verdict::Fail
            },
            "backend remains resident with a live control plane",
        ));
    }
    // Emit every planned optional absence, including compat when bind is absent.
    let expected = plan.expected()?;
    for (name, skip) in expected {
        if skip && !report.rows.iter().any(|r| r.vector == name) {
            report.rows.push(ResultRow::new(
                &name,
                Verdict::Skip,
                "optional probe absent in host plan",
            ));
        }
    }
    Ok(())
}
pub(super) fn check_environment(plan: &Plan) -> io::Result<()> {
    plan.validate()?;
    if !cfg!(target_arch = "aarch64")
        || unsafe { libc::getpid() } != 1
        || unsafe { libc::geteuid() } != 0
    {
        return Err(io::Error::other(
            "guest executor requires isolated AArch64 QEMU PID1",
        ));
    }
    let cmdline = read_regular(Path::new("/proc/cmdline"), 16 * 1024)?;
    let text = std::str::from_utf8(&cmdline).map_err(io::Error::other)?;
    if !text
        .split_ascii_whitespace()
        .any(|v| v == format!("vpnhide_harness={}", plan.session))
    {
        return Err(io::Error::other(
            "isolated harness boot marker does not match the plan",
        ));
    }
    Ok(())
}
pub fn run() -> io::Result<()> {
    let plan: Plan =
        serde_json::from_slice(&read_regular(Path::new("/vpnhide-plan.json"), 64 * 1024)?)
            .map_err(io::Error::other)?;
    check_environment(&plan)?;
    if plan.backend == Backend::Kpm {
        return run_kpm(&plan);
    }
    println!("##### VPNHIDE-QEMU-TEST START #####");
    let mut report = Report::new(&plan);
    let outcome = (|| -> io::Result<()> {
        if plan.mode == Mode::Preflight {
            preflight(&plan, &mut report)?;
            report.setup_ok = true;
            return Ok(());
        }
        if matches!(plan.mode, Mode::FixtureRaw | Mode::FixtureProtected) {
            // Isolated guest reference objects, never production fixture effects.
            checked("ip", &["link", "set", "lo", "up"])?;
            checked("ip", &["addr", "replace", "127.0.0.1/8", "dev", "lo"])?;
            if plan.mode == Mode::FixtureProtected {
                load(&plan, &mut report)?;
            }
            let c = execute(
                "/native-fixture-oracle",
                &[if plan.mode == Mode::FixtureRaw {
                    "raw"
                } else {
                    "protected"
                }],
                false,
            )?;
            io::stdout().write_all(&c.output.stdout)?;
            report.rows.push(ResultRow::new(
                "native_fixture",
                if success(&c) {
                    Verdict::Pass
                } else {
                    Verdict::Fail
                },
                "external Core fixture oracle",
            ));
            report.setup_ok = true;
            return Ok(());
        }
        setup()?;
        report.setup_ok = true;
        load(&plan, &mut report)?;
        vectors(&plan, &mut report)
    })();
    if let Err(error) = outcome {
        eprintln!("HARNESS_ERROR {error}");
        report.setup_ok = false;
    }
    for row in &report.rows {
        println!("RESULT {}={:?} ({})", row.vector, row.verdict, row.detail);
    }
    let pass = report
        .rows
        .iter()
        .filter(|r| r.verdict == Verdict::Pass)
        .count();
    let fail = report
        .rows
        .iter()
        .filter(|r| matches!(r.verdict, Verdict::Fail | Verdict::Unknown))
        .count();
    println!(
        "SUMMARY pass={pass} fail={fail} panic=0 registered={}",
        report.registered
    );
    println!(
        "VPNHIDE_HARNESS_REPORT {}",
        serde_json::to_string(&report).map_err(io::Error::other)?
    );
    println!("VPNHIDE_HARNESS_END {}", plan.session);
    println!("##### VPNHIDE-QEMU-TEST END #####");
    io::stdout().flush()?;
    io::stderr().flush()?;
    // This point is reachable only after PID1 + unique kernel-boot marker proof.
    unsafe {
        libc::sync();
        libc::reboot(libc::RB_POWER_OFF);
    }
    Err(io::Error::other("QEMU poweroff returned"))
}
fn run_kpm(plan: &Plan) -> io::Result<()> {
    if plan.kpm_phase.is_none() {
        return Err(io::Error::other("KPM guest phase was not declared"));
    }
    let payload = read_regular(
        Path::new("/vpnhide.kpm"),
        vpnhide_artifact_model::MAX_PAYLOAD_BYTES,
    )?;
    if Some(FileIdentity::of(&payload).sha256) != plan.payload_sha256 {
        return Err(io::Error::other("KPM guest payload identity changed"));
    }
    let mut phase = model::PhaseReport {
        plan: plan.clone(),
        boot: fs::read_to_string("/proc/sys/kernel/random/boot_id")?
            .trim()
            .into(),
        setup_ok: false,
        loaded: false,
        counts: Vec::new(),
        fields: Vec::new(),
    };
    let outcome = (|| -> io::Result<()> {
        setup()?;
        phase.setup_ok = true;
        phase.loaded =
            String::from_utf8_lossy(&checked("dmesg", &[])?).contains("KPM hooks installed");
        for (n, k, v, s, _) in count_cases() {
            phase.counts.push((n.into(), counts(k, v, s)));
        }
        let index = fs::read_to_string("/sys/class/net/vpn0/ifindex")?;
        let index = index.trim();
        if index.parse::<u32>().ok().is_none_or(|v| v == 0) {
            return Err(io::Error::other("missing KPM fixture index"));
        }
        for (n, p, program, args, app) in [
            ("gai", plan.probes.gai, "/gai", Vec::new(), true),
            ("ifconf", plan.probes.ifconf, "/ifconf", Vec::new(), true),
            (
                "bind",
                plan.probes.bind,
                "/bind-probe",
                vec!["vpn0", index],
                false,
            ),
            (
                "ioctl",
                plan.probes.ioctl,
                "/iface-ioctl",
                vec!["vpn0"],
                false,
            ),
        ] {
            if p.present {
                phase.fields.push((
                    n.into(),
                    field_output(program, &args, app).map(|f| f.into_iter().collect()),
                ));
            }
        }
        Ok(())
    })();
    if let Err(e) = outcome {
        eprintln!("HARNESS_ERROR {e}");
        phase.setup_ok = false;
    }
    println!(
        "VPNHIDE_KPM_PHASE {}",
        serde_json::to_string(&phase).map_err(io::Error::other)?
    );
    println!("VPNHIDE_HARNESS_END {}", plan.session);
    io::stdout().flush()?;
    io::stderr().flush()?;
    unsafe {
        libc::sync();
        libc::reboot(libc::RB_POWER_OFF);
    }
    Err(io::Error::other("QEMU poweroff returned"))
}

#[cfg(test)]
mod utility_tests {
    use super::*;
    #[test]
    fn only_the_exact_expected_device_denial_preserves_partial_enumeration() {
        let error = b"ifconfig: vpn0: error fetching interface information: Device not found\n";
        assert!(expected_ifconfig_denial(
            Observation::IfconfigAll,
            Some(1),
            b"eth0 Link encap:Ethernet\n",
            error
        ));
        assert!(expected_ifconfig_denial(
            Observation::IfconfigVpn,
            Some(1),
            b"",
            error
        ));
        assert!(!expected_ifconfig_denial(
            Observation::IfconfigOrdinary,
            Some(1),
            b"",
            error
        ));
        assert!(!expected_ifconfig_denial(
            Observation::IfconfigAll,
            Some(2),
            b"",
            error
        ));
        assert!(!expected_ifconfig_denial(
            Observation::IfconfigAll,
            Some(1),
            b"",
            b"ifconfig: socket: Permission denied\n"
        ));
        assert!(!expected_ifconfig_denial(
            Observation::IfconfigAll,
            Some(1),
            b"",
            b""
        ));
    }
}
#[cfg(test)]
mod environment_tests {
    use super::*;
    #[test]
    fn guest_refuses_the_host_before_loading_or_powering_off() {
        let plan = Plan {
            schema: model::REPORT_SCHEMA,
            session: "a".repeat(64),
            backend: Backend::Kmod,
            mode: Mode::Full,
            payload_sha256: Some("b".repeat(64)),
            probes: Default::default(),
            kpm_phase: None,
        };
        assert!(check_environment(&plan).is_err());
    }
}
