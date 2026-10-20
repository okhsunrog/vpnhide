//! Everything a bug report needs from the device, framed into named sections: module
//! state, backend status, kernel/network forensics and the per-user app scan. Also the
//! cheap hook-counter probe. Read by humans except where `DebugShellSnapshot` and
//! `StatisticsData` parse a section (`kmod_state`, `kpm_state`, `lsposed_state`,
//! `pm_packages`), whose formats match the root snapshot.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{
    Out, Paths, flag, getenforce, kpatch_bin, ls_dir, ls_line, non_empty, read_node, read_text,
    run, sha256_line, trimmed,
};
use crate::diagnostics::prefixes;

const BUDGET: Duration = Duration::from_secs(120);
const TOOL: Duration = Duration::from_secs(15);

/// `emit_file` (debug flavour): content, or `(missing: path)`.
fn file(out: &mut Out, name: &str, path: &Path) {
    let body = match read_text(path) {
        Some(text) => text,
        None => format!("(missing: {})", path.display()),
    };
    out.section(name, body);
}

/// A procfs node: content, or `(missing: path)`.
fn node(out: &mut Out, name: &str, path: &Path) {
    let body = if path.exists() {
        read_node(path).unwrap_or_else(|e| format!("cat: {}: {e}", path.display()))
    } else {
        format!("(missing: {})", path.display())
    };
    out.section(name, body);
}

/// A tool's combined output (`cmd 2>&1`).
fn tool(out: &mut Out, name: &str, program: &str, args: &[&str]) {
    let ran = run(program, args, out.remaining(TOOL));
    out.section(name, ran.combined());
}

fn kpm_state(out: &mut Out, paths: &Paths) {
    let ran = run(
        &paths.core_exe.display().to_string(),
        &["observe", "kpm-state"],
        out.remaining(Duration::from_secs(10)),
    );
    let mut body = ran.combined();
    if !body.is_empty() && !body.ends_with('\n') {
        body.push('\n');
    }
    body.push_str(&format!("[exit={}]", ran.exit));
    out.section("kpm_state", body);
}

pub fn hook_counters(paths: &Paths) -> Out {
    let mut out = Out::new(
        prefixes::DEBUG_BEGIN,
        prefixes::DEBUG_END,
        Duration::from_secs(8),
    );
    node(&mut out, "current_boot_id", &paths.boot_id);
    node(&mut out, "kmod_state", &paths.proc_ctl);
    kpm_state(&mut out, paths);
    file(&mut out, "lsposed_state", &paths.lsposed_state);
    file(&mut out, "zygisk_status", &paths.zygisk_status);
    out
}

const PROPS: &[&str] = &[
    "ro.product.manufacturer",
    "ro.product.brand",
    "ro.product.model",
    "ro.product.device",
    "ro.product.name",
    "ro.product.board",
    "ro.product.cpu.abilist",
    "ro.build.fingerprint",
    "ro.build.version.release",
    "ro.build.version.sdk",
    "ro.build.version.security_patch",
    "ro.vendor.build.security_patch",
    "ro.bootimage.build.fingerprint",
    "ro.vendor.build.fingerprint",
    "ro.odm.build.fingerprint",
    "ro.boot.slot_suffix",
    "ro.boot.bootloader",
    "ro.boot.hardware",
    "ro.boot.hardware.sku",
    "ro.boot.verifiedbootstate",
    "ro.boot.vbmeta.device_state",
    "ro.boot.flash.locked",
    "ro.boot.dynamic_partitions",
    "ro.kernel.qemu",
    "sys.boot_completed",
];

/// `getprop` lines (`name=value`) for the set props.
pub fn props(names: &[&str]) -> String {
    names
        .iter()
        .filter_map(|p| vpnhide_activator::sys::property(p).map(|v| format!("{p}={v}\n")))
        .collect()
}

/// Serial number and MAC addresses never leave the device.
pub fn redact_cmdline(cmdline: &str) -> String {
    cmdline
        .split(' ')
        .map(|word| {
            for key in [
                "androidboot.serialno=",
                "androidboot.wifi_macaddr=",
                "androidboot.btmacaddr=",
            ] {
                if word.starts_with(key) {
                    let tail = if word.ends_with('\n') { "\n" } else { "" };
                    return format!("{key}<redacted>{tail}");
                }
            }
            word.to_owned()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn grep_lines(text: &str, keep: impl Fn(&str) -> bool) -> String {
    text.lines()
        .filter(|l| keep(l))
        .map(|l| format!("{l}\n"))
        .collect()
}

fn contains_any_ci(line: &str, needles: &[&str]) -> bool {
    let lower = line.to_ascii_lowercase();
    needles.iter().any(|n| lower.contains(n))
}

/// `[ -d dir ]` flags plus the dir's own `ls -ldZ` line.
fn file_flags(dir: &Path) -> String {
    let mut s = format!("path={}\n", dir.display());
    if dir.is_dir() {
        s.push_str("installed=1\n");
        for (key, marker) in [
            ("disabled", "disable"),
            ("remove", "remove"),
            ("update", "update"),
        ] {
            s.push_str(&format!("{key}={}\n", flag(dir.join(marker).is_file())));
        }
        s.push_str(&ls_line(dir));
        s.push('\n');
    } else {
        s.push_str("installed=0\n");
    }
    s
}

fn hash_file(path: &Path) -> String {
    if path.is_file() {
        format!("{}\n{}\n", ls_line(path), sha256_line(path))
    } else {
        format!("(missing: {})\n", path.display())
    }
}

fn staged_state(paths: &Paths, id: &str) -> String {
    let staged = paths.modules_update.join(id);
    if !staged.is_dir() {
        "staged=0\n".into()
    } else if staged.join("module.prop").is_file() {
        "staged=1 staged_prop=1\n".into()
    } else {
        "staged=1 staged_prop=0\n".into()
    }
}

fn module_state(paths: &Paths, id: &str, artifacts: &[&str]) -> String {
    let dir = paths.module(id);
    let mut s = file_flags(&dir);
    for a in artifacts {
        s.push_str(&hash_file(&dir.join(a)));
    }
    s.push_str(&staged_state(paths, id));
    s
}

fn module_prop_lines(dir: &Path) -> String {
    read_text(&dir.join("module.prop"))
        .map(|t| t.lines().map(|l| format!("module.prop: {l}\n")).collect())
        .unwrap_or_default()
}

fn tail(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..]
        .iter()
        .map(|l| format!("{l}\n"))
        .collect()
}

fn head(text: &str, n: usize) -> String {
    text.lines().take(n).map(|l| format!("{l}\n")).collect()
}

pub fn collect(paths: &Paths) -> Out {
    let mut out = Out::new(prefixes::DEBUG_BEGIN, prefixes::DEBUG_END, BUDGET);
    let o = &mut out;
    node(o, "current_boot_id", &paths.boot_id);
    o.section(
        "uname",
        vpnhide_activator::sys::uname()
            .map(|u| u.all())
            .unwrap_or_else(|e| format!("uname: {e}")),
    );
    file(o, "proc_version", Path::new("/proc/version"));
    o.section(
        "proc_cmdline",
        redact_cmdline(&read_node(Path::new("/proc/cmdline")).unwrap_or_default()),
    );
    o.section("getprop_selected", props(PROPS));
    o.section("root_manager", root_manager(paths, o));
    o.section("selinux", {
        let mut s = format!("{}\n", getenforce());
        if let Ok(v) = std::fs::read_to_string("/sys/fs/selinux/enforce") {
            super::push_lines(&mut s, &v);
        }
        for p in [
            "/data",
            "/data/adb",
            "/data/system",
            "/proc/net",
            "/sys/class/net",
        ] {
            s.push_str(&ls_line(Path::new(p)));
            s.push('\n');
        }
        s
    });
    o.section("kernel_config", kernel_config());
    o.section(
        "proc_modules",
        grep_lines(
            &read_node(Path::new("/proc/modules")).unwrap_or_default(),
            |l| contains_any_ci(l, &["vpnhide", "kpatch", "ksu", "magisk", "zygisk"]),
        ),
    );
    o.section("kprobes", kprobes());
    o.section("kernel_symbols", kernel_symbols());
    o.section("module_inventory", module_inventory(paths));

    let core = paths.module("vpnhide_core");
    file(o, "core_prop", &core.join("module.prop"));
    o.section(
        "core_module_state",
        module_state(paths, "vpnhide_core", &["vpnhide-core"]),
    );
    file(o, "core_boot_status", &paths.core_boot_status);
    // Core's persisted once-per-boot native decision (the app reads it over RPC).
    file(o, "core_native_boot", &paths.core_native_boot);
    o.section(
        "core_log",
        match read_text(&paths.core_log) {
            Some(t) => tail(&t, 200),
            None => format!(
                "tail: {}: No such file or directory",
                paths.core_log.display()
            ),
        },
    );
    file(
        o,
        "kmod_prop",
        &paths.module("vpnhide_kmod").join("module.prop"),
    );
    o.section(
        "kmod_module_state",
        module_state(paths, "vpnhide_kmod", &["vpnhide_kmod.ko"]),
    );
    node(o, "kmod_state", &paths.proc_ctl);
    // Diagnostic-only companion to the control node; never parsed.
    node(o, "kmod_diag", &paths.proc_diag);

    file(
        o,
        "kpm_prop",
        &paths.module("vpnhide_kpm").join("module.prop"),
    );
    o.section(
        "kpm_module_state",
        module_state(paths, "vpnhide_kpm", &["vpnhide.kpm"]),
    );
    kpm_state(o, paths);
    o.section("kpatch_runtime", kpatch_runtime(paths, o));

    file(
        o,
        "zygisk_prop",
        &paths.module("vpnhide_zygisk").join("module.prop"),
    );
    o.section(
        "zygisk_module_state",
        module_state(
            paths,
            "vpnhide_zygisk",
            &["zygisk/arm64-v8a.so", "zygisk/armeabi-v7a.so"],
        ),
    );
    file(o, "zygisk_status", &paths.zygisk_status);
    o.section("zygisk_runtime", zygisk_runtime(paths));
    o.section("ports_state", ports_state(o));

    file(o, "lsposed_state", &paths.lsposed_state);
    o.section("lsposed_framework", {
        let mut s = String::new();
        for base in [&paths.modules, &paths.modules_update] {
            for id in ["zygisk_vector", "zygisk_lsposed", "lsposed"] {
                let dir = base.join(id);
                if dir.join("module.prop").is_file() {
                    s.push_str(&format!("### {}\n", dir.display()));
                    s.push_str(&file_flags(&dir));
                    s.push_str(&module_prop_lines(&dir));
                }
            }
        }
        s
    });
    o.section("lsposed_files", {
        let config = Path::new("/data/adb/lspd/config");
        let mut s = ls_dir(config, 200);
        s.push('\n');
        for suffix in ["", "-wal", "-shm"] {
            let p = config.join(format!("modules_config.db{suffix}"));
            if p.exists() {
                s.push_str(&ls_line(&p));
                s.push('\n');
            }
        }
        s
    });

    // Network state before the package scan, which can eat most of the budget.
    tool(o, "network_addr", "ip", &["-d", "addr"]);
    o.section("network_operstate", {
        let mut names: Vec<PathBuf> = std::fs::read_dir("/sys/class/net")
            .map(|it| it.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        names.sort();
        names
            .iter()
            .map(|p| {
                let state = std::fs::read_to_string(p.join("operstate")).unwrap_or_default();
                format!(
                    "{}: {}\n",
                    p.file_name().unwrap_or_default().to_string_lossy(),
                    state.trim()
                )
            })
            .collect::<String>()
    });
    tool(
        o,
        "network_routes",
        "ip",
        &["route", "show", "table", "all"],
    );
    tool(o, "network_rules", "ip", &["rule"]);
    let ss = run("ss", &["-H", "-ltnup"], o.remaining(TOOL));
    o.section(
        "network_sockets",
        head(
            &grep_lines(&ss.stdout, |l| {
                l.contains("127.") || l.contains("::1") || l.contains("LISTEN") || l.contains("udp")
            }),
            300,
        ),
    );
    let dumpsys = run(
        "dumpsys",
        &["connectivity"],
        o.remaining(Duration::from_secs(30)),
    );
    o.section(
        "connectivity_dump",
        head(
            &grep_lines(&dumpsys.stdout, |l| {
                contains_any_ci(
                    l,
                    &[
                        "vpn",
                        "tun",
                        "networkagentinfo",
                        "networkcapabilities",
                        "linkproperties",
                        "rmnet",
                        "wlan",
                        "dummy",
                    ],
                )
            }),
            400,
        ),
    );
    for name in [
        "route",
        "ipv6_route",
        "if_inet6",
        "tcp",
        "tcp6",
        "udp",
        "udp6",
        "dev",
    ] {
        let p = PathBuf::from(format!("/proc/net/{name}"));
        file(o, &format!("proc_net_{name}"), &p);
    }
    o.section(
        "proc_net_fib_trie",
        head(
            &read_node(Path::new("/proc/net/fib_trie")).unwrap_or_default(),
            1200,
        ),
    );

    file(o, "canonical_config", &paths.config_file);
    for (name, path) in &paths.legacy {
        file(o, name, path);
    }
    super::inventory::emit(o, true);
    let summary = app_scan_diagnostics(o);
    o.section("app_scan_diagnostics", summary);
    out
}

fn root_manager(paths: &Paths, out: &Out) -> String {
    let version = |program: &str, arg: &str| {
        let r = run(program, &[arg], out.remaining(Duration::from_secs(5)));
        if r.exit == 0 {
            trimmed(&r.stdout).to_owned()
        } else {
            String::new()
        }
    };
    let mut s = format!("id={}\n", vpnhide_activator::sys::id_line());
    s.push_str(&format!(
        "su={}\n",
        super::which("su")
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    ));
    s.push_str(&format!(
        "magisk_version_name={}\n",
        version("magisk", "-v")
    ));
    s.push_str(&format!(
        "magisk_version_code={}\n",
        version("magisk", "-V")
    ));
    s.push_str(&format!(
        "ksu_version={}\n",
        trimmed(&read_text(Path::new("/data/adb/ksu/version")).unwrap_or_default())
    ));
    s.push_str(&format!("ksud_version={}\n", version("ksud", "--version")));
    s.push_str(&format!("apatch_dir={}\n", flag(paths.apatch_dir.is_dir())));
    s.push_str(&format!(
        "apd={}\n",
        flag(super::is_executable(Path::new("/data/adb/apd")))
    ));
    s.push_str(&format!(
        "magisk_dir={}\n",
        flag(Path::new("/data/adb/magisk").is_dir())
    ));
    s.push_str(&format!(
        "ksu_dir={}\n",
        flag(Path::new("/data/adb/ksu").is_dir())
    ));
    for p in [
        Path::new("/data/adb"),
        &paths.modules,
        &paths.modules_update,
    ] {
        s.push_str(&ls_line(p));
        s.push('\n');
    }
    s
}

fn kernel_config() -> String {
    let path = Path::new("/proc/config.gz");
    match crate::native_boot::read_gzip_text(path) {
        Some(config) => grep_lines(&config, |l| {
            [
                "KPROBES",
                "KRETPROBES",
                "MODULES",
                "MODULE_SIG",
                "SECURITY_SELINUX",
                "NETFILTER",
                "IP_NF",
                "IP6_NF",
                "CGROUP_BPF",
            ]
            .iter()
            .any(|opt| {
                l.strip_prefix("CONFIG_")
                    .and_then(|rest| rest.strip_prefix(opt))
                    .is_some_and(|rest| rest.starts_with('=') || rest.starts_with('_'))
            })
        }),
        None => "(missing: /proc/config.gz)".into(),
    }
}

fn kprobes() -> String {
    grep_lines(
        &read_node(Path::new("/sys/kernel/debug/kprobes/list")).unwrap_or_default(),
        |l| {
            let lower = l.to_ascii_lowercase();
            ["vpnhide", "dev_ioctl", "sock_ioctl", "rtnl", "route"]
                .iter()
                .any(|n| lower.contains(n))
                || (lower.contains("inet") && lower.contains("fill"))
        },
    )
}

const SYMBOLS: &[&str] = &[
    "dev_ioctl",
    "dev_ifconf",
    "sock_ioctl",
    "rtnl_fill_ifinfo",
    "inet6_fill_ifaddr",
    "inet_fill_ifaddr",
    "fib_route_seq_show",
    "ipv6_route_seq_show",
    "fib_dump_info",
    "rt6_fill_node",
    "fib_nl_fill_rule",
];

/// `grep -w SYM /proc/kallsyms | head -3` per symbol, in one pass over kallsyms.
fn kernel_symbols() -> String {
    let kallsyms = read_node(Path::new("/proc/kallsyms")).unwrap_or_default();
    let mut hits: Vec<Vec<&str>> = vec![Vec::new(); SYMBOLS.len()];
    for line in kallsyms.lines() {
        let Some(name) = line.split_whitespace().nth(2) else {
            continue;
        };
        if let Some(i) = SYMBOLS.iter().position(|s| *s == name)
            && hits[i].len() < 3
        {
            hits[i].push(line);
        }
    }
    SYMBOLS
        .iter()
        .zip(hits)
        .map(|(sym, lines)| format!("{sym}: {}\n", lines.join("\n")))
        .collect()
}

const INVENTORY_IDS: &[&str] = &[
    "vpnhide_core",
    "vpnhide_kmod",
    "vpnhide_kpm",
    "vpnhide_zygisk",
    "vpnhide_builtin",
    "vpnhide_ports",
    "KPatch-Next",
    "kpatch-next",
    "zygisk_vector",
    "zygisk_lsposed",
    "lsposed",
    "zygisk_next",
    "zygisknext",
    "zygisksu",
    "NeoZygisk",
];

fn module_inventory(paths: &Paths) -> String {
    let mut s = String::new();
    for base in [&paths.modules, &paths.modules_update] {
        if !base.is_dir() {
            continue;
        }
        for id in INVENTORY_IDS {
            let dir = base.join(id);
            if !dir.exists() {
                continue;
            }
            s.push_str(&format!("### {}\n", dir.display()));
            s.push_str(&file_flags(&dir));
            s.push_str(&module_prop_lines(&dir));
            s.push_str(&ls_dir(&dir, 80));
            s.push_str("\n\n");
        }
    }
    s
}

fn kpatch_runtime(paths: &Paths, out: &Out) -> String {
    let mut s = format!("apatch_dir={}\n", flag(paths.apatch_dir.is_dir()));
    s.push_str(&format!(
        "apd={}\n",
        flag(super::is_executable(Path::new("/data/adb/apd")))
    ));
    s.push_str(&format!(
        "superkey_saved={}\n",
        flag(non_empty(&paths.superkey_file))
    ));
    let autoload = Path::new("/data/adb/fp/kpms/kpm_autoload_config.json");
    match read_text(autoload) {
        Some(text) => {
            s.push_str("folkpatch_kpm_autoload=1\n");
            s.push_str(&head(&text, 80));
        }
        None => s.push_str("folkpatch_kpm_autoload=0\n"),
    }
    match kpatch_bin() {
        Some(bin) => {
            let shown = bin.display().to_string();
            s.push_str(&format!("kpatch_bin={shown}\n"));
            let hello = run(&shown, &["hello"], out.remaining(Duration::from_secs(5)));
            super::push_lines(&mut s, &hello.combined());
            s.push_str(&format!("hello_exit={}\n", hello.exit));
            let list = run(
                &shown,
                &["kpm", "list"],
                out.remaining(Duration::from_secs(5)),
            );
            super::push_lines(&mut s, &list.combined());
            s.push_str(&format!("list_exit={}\n", list.exit));
        }
        None => s.push_str("kpatch_bin=(not found)\n"),
    }
    s
}

fn zygisk_runtime(paths: &Paths) -> String {
    let mut s = String::new();
    for base in [&paths.modules, &paths.modules_update] {
        let Ok(entries) = std::fs::read_dir(base) else {
            continue;
        };
        let mut dirs: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.path())
            .filter(|p| {
                let name = p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_ascii_lowercase();
                ["zygisk", "lsposed", "vector"]
                    .iter()
                    .any(|n| name.contains(n))
            })
            .collect();
        dirs.sort();
        for d in dirs {
            s.push_str(&format!("{}\n", d.display()));
        }
    }
    s
}

fn ports_state(out: &Out) -> String {
    let mut s = String::new();
    for (program, chain) in [("iptables", "vpnhide_out"), ("ip6tables", "vpnhide_out6")] {
        let t = out.remaining(Duration::from_secs(5));
        let output = run(program, &["-w", "2", "-S", "OUTPUT"], t);
        s.push_str(&grep_lines(&output.combined(), |l| l.contains("vpnhide")));
        super::push_lines(
            &mut s,
            &run(program, &["-w", "2", "-S", chain], t).combined(),
        );
        super::push_lines(
            &mut s,
            &run(
                program,
                &["-w", "2", "-L", chain, "-n", "-v", "--line-numbers"],
                t,
            )
            .combined(),
        );
    }
    s
}

/// Privacy-safe summary of the per-user app scan: names redacted, counts only — never
/// package names or paths. Diagnoses "couldn't read all profiles".
fn app_scan_diagnostics(out: &Out) -> String {
    let plain =
        super::inventory::pm_within(&["list", "users"], out.remaining(Duration::from_secs(20)))
            .stdout;
    let mut s = String::from("--- pm list users (names redacted) ---\n");
    for line in trimmed(&plain).lines() {
        s.push_str(&redact_user_name(line));
        s.push('\n');
    }
    let ids = super::inventory::user_ids(&plain, "");
    s.push_str("--- per-user pm list packages -U -f (counts only, no names) ---\n");
    for user in ids {
        let running = plain.lines().any(|l| {
            l.contains(&format!("UserInfo{{{user}:"))
                && l.split_whitespace().any(|w| w == "running")
        });
        let args = super::inventory::packages_args(user);
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let listed = super::inventory::pm_within(&argv, out.remaining(Duration::from_secs(20)));
        let lines: Vec<&str> = listed
            .stdout
            .lines()
            .filter(|l| l.starts_with("package:"))
            .collect();
        let with_uid = lines.iter().filter(|l| l.contains(" uid:")).count();
        let with_path = lines
            .iter()
            .filter(|l| {
                l["package:".len()..]
                    .split(' ')
                    .next()
                    .is_some_and(|w| w.contains('='))
            })
            .count();
        let err: Vec<&str> = listed.stderr.lines().take(2).collect();
        s.push_str(&format!(
            "user={user} running={} exit={} package_lines={} with_uid={with_uid} with_path={with_path} stderr=[{} ]\n",
            flag(running),
            listed.exit,
            lines.len(),
            err.join(" "),
        ));
    }
    s.push_str("inprocess_backstop=app also unions getInstalledApplications(0) into user 0\n");
    s
}

/// `UserInfo{10:Work profile:1030}` → `UserInfo{10:<name>:1030}`.
pub fn redact_user_name(line: &str) -> String {
    let Some(start) = line.find("UserInfo{") else {
        return line.to_owned();
    };
    let body_start = start + "UserInfo{".len();
    let rest = &line[body_start..];
    let Some(close) = rest.find('}') else {
        return line.to_owned();
    };
    let inner = &rest[..close];
    let parts: Vec<&str> = inner.split(':').collect();
    if parts.len() < 3 || !parts[0].bytes().all(|b| b.is_ascii_digit()) {
        return line.to_owned();
    }
    let flags = parts[parts.len() - 1];
    format!(
        "{}{}:<name>:{flags}{}",
        &line[..body_start],
        parts[0],
        &rest[close..]
    )
}
