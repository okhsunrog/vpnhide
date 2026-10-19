//! One batched root probe: every piece of system state the dashboard and the Hiding tab
//! derive from, framed into named sections. Section names and value formats are the
//! contract with the app's parsers (`RootSnapshotCache`, `DashboardData`).

use std::path::Path;
use std::time::Duration;

use super::{
    Out, Paths, flag, getenforce, kpatch_bin, non_empty, now_ms, pending_update, read_node,
    read_text, run, trimmed,
};
use crate::diagnostics::prefixes;

const BUDGET: Duration = Duration::from_secs(30);

pub fn collect(paths: &Paths, with_packages: bool) -> Out {
    let mut out = Out::new(prefixes::ROOT_BEGIN, prefixes::ROOT_END, BUDGET);
    phase(&mut out, "module_props", |o| module_props(o, paths));
    phase(&mut out, "target_files", |o| {
        file(o, "canonical_config", &paths.config_file);
        o.section("superkey_saved", flag(non_empty(&paths.superkey_file)));
    });
    phase(&mut out, "legacy_config", |o| {
        for (name, path) in &paths.legacy {
            file(o, name, path);
        }
    });
    phase(&mut out, "kmod_status_files", |o| {
        file(o, "current_boot_id", &paths.boot_id);
        file(o, "kmod_load_status", &paths.kmod_load_status);
        file(o, "kmod_load_dmesg", &paths.kmod_load_dmesg);
        file(o, "zygisk_status", &paths.zygisk_status);
        file(o, "kpm_load_status", &paths.kpm_load_status);
        file(o, "core_boot_status", &paths.core_boot_status);
    });
    phase(&mut out, "runtime_status_files", |o| runtime(o, paths));
    if with_packages {
        phase(&mut out, "pm_packages", |o| {
            super::inventory::emit(o, false)
        });
    }
    phase(&mut out, "shell_probe_identity", |o| {
        o.section("snapshot_shell_uid", identity(&paths.proc_ctl));
    });
    phase(&mut out, "shell_probe_proc_exists", |o| {
        o.section("proc_exists", flag(paths.proc_ctl.exists()));
    });
    phase(&mut out, "shell_probe_ports_chain", |o| {
        let verdict = ports_chain(o);
        o.section("ports_chain", verdict);
    });
    phase(&mut out, "shell_probe_lsposed_framework", |o| {
        o.section("lsposed_framework", lsposed_framework(paths));
    });
    out
}

fn phase(out: &mut Out, name: &str, body: impl FnOnce(&mut Out)) {
    if out.expired() {
        return;
    }
    let start = now_ms();
    body(out);
    let line = format!("{}{name}={}", prefixes::ROOT_TIMING, now_ms() - start);
    out.raw_line(&line);
}

/// `emit_file`: a regular file's content, or an empty section.
fn file(out: &mut Out, name: &str, path: &Path) {
    out.section(name, read_text(path).unwrap_or_default());
}

fn module_props(out: &mut Out, paths: &Paths) {
    const IDS: [(&str, &str); 4] = [
        ("kmod", "vpnhide_kmod"),
        ("zygisk", "vpnhide_zygisk"),
        ("kpm", "vpnhide_kpm"),
        ("core", "vpnhide_core"),
    ];
    for (short, id) in IDS {
        file(
            out,
            &format!("{short}_prop"),
            &paths.module(id).join("module.prop"),
        );
    }
    for (short, id) in IDS {
        out.section(
            &format!("{short}_module_dir"),
            flag(paths.module(id).is_dir()),
        );
    }
    let present = |ok: bool| if ok { "present" } else { "missing" };
    out.section(
        "kmod_artifact_state",
        present(
            paths
                .module("vpnhide_kmod")
                .join("vpnhide_kmod.ko")
                .is_file(),
        ),
    );
    out.section(
        "kpm_artifact_state",
        present(paths.module("vpnhide_kpm").join("vpnhide.kpm").is_file()),
    );
    let zygisk = paths.module("vpnhide_zygisk").join("zygisk");
    out.section(
        "zygisk_artifact_state",
        present(zygisk.join("arm64-v8a.so").is_file() || zygisk.join("armeabi-v7a.so").is_file()),
    );
    for (short, id) in [IDS[0], IDS[2], IDS[1], IDS[3]] {
        out.section(
            &format!("{short}_disabled"),
            flag(paths.module(id).join("disable").is_file()),
        );
    }
    for (short, id) in [IDS[0], IDS[2], IDS[1], IDS[3]] {
        out.section(
            &format!("{short}_pending_update"),
            flag(pending_update(paths, id)),
        );
    }
}

fn runtime(out: &mut Out, paths: &Paths) {
    let release = vpnhide_activator::sys::uname()
        .map(|u| u.release)
        .unwrap_or_default();
    out.section("kernel_release", release);
    let ctl = if paths.proc_ctl.exists() {
        read_node(&paths.proc_ctl).unwrap_or_default()
    } else {
        String::new()
    };
    out.section("kmod_state", ctl);
    let exe = paths.core_exe.display().to_string();
    let state = run(
        &exe,
        &["observe", "kpm-state"],
        out.remaining(Duration::from_secs(10)),
    );
    out.section("kpm_state", state.stdout);
    let list = run(
        &exe,
        &["observe", "kpm-list"],
        out.remaining(Duration::from_secs(10)),
    );
    out.section("kpm_runtime_modules", list.stdout);
    file(out, "lsposed_state", &paths.lsposed_state);
    out.section("getenforce", getenforce());
    // A *live* KPM runtime: APatch's /data/adb/ap, or KPatch-Next's CLI whose `hello`
    // succeeds only on a patched kernel. kpatchRuntimeAvailable() reads this.
    let mut body = format!("apatch_dir={}\n", flag(paths.apatch_dir.is_dir()));
    match kpatch_bin() {
        Some(bin) => {
            let shown = bin.display().to_string();
            let hello = run(&shown, &["hello"], out.remaining(Duration::from_secs(5)));
            body.push_str(&format!("kpatch_bin={shown}\nhello_exit={}\n", hello.exit));
        }
        None => body.push_str("kpatch_bin=\n"),
    }
    out.section("kpatch_runtime", body);
}

/// Who is this probe, really? Root-only reads (0600 ctl, CAP_NET_ADMIN) give false
/// negatives when not uid 0, so the detectors gate runtime checks on this.
fn identity(ctl: &Path) -> String {
    let context = std::fs::read("/proc/self/attr/current")
        .map(|raw| String::from_utf8_lossy(&raw).replace('\0', ""))
        .unwrap_or_default();
    let errno = match std::fs::symlink_metadata(ctl) {
        Ok(_) => "ok".to_owned(),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => "eacces".into(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "enoent".into(),
        Err(e) => format!("other:{e}"),
    };
    format!(
        "uid={}\nid={}\ncontext={}\nerrno_ctl={errno}\n",
        vpnhide_activator::sys::uid(),
        vpnhide_activator::sys::id_line(),
        trimmed(&context),
    )
}

/// "1" when both chains exist and OUTPUT jumps to them, "0" when a chain or jump is
/// absent (iptables exits 1/2), "error=<rc>" otherwise (4: xtables lock held, 3: no
/// CAP_NET_ADMIN, 127: no iptables) — rendered as "not verified", never "inactive".
fn ports_chain(out: &Out) -> String {
    let checks: [(&str, &[&str]); 4] = [
        ("iptables", &["-w", "2", "-L", "vpnhide_out", "-n"]),
        (
            "iptables",
            &["-w", "2", "-C", "OUTPUT", "-j", "vpnhide_out"],
        ),
        ("ip6tables", &["-w", "2", "-L", "vpnhide_out6", "-n"]),
        (
            "ip6tables",
            &["-w", "2", "-C", "OUTPUT", "-j", "vpnhide_out6"],
        ),
    ];
    for (program, args) in checks {
        match run(program, args, out.remaining(Duration::from_secs(5))).exit {
            0 => {}
            1 | 2 => return "0".into(),
            rc => return format!("error={rc}"),
        }
    }
    "1".into()
}

fn lsposed_framework(paths: &Paths) -> String {
    for id in ["zygisk_vector", "zygisk_lsposed", "lsposed"] {
        for base in [&paths.modules, &paths.modules_update] {
            let dir = base.join(id);
            if dir.join("module.prop").is_file() {
                return format!(
                    "installed=1\ndisabled={}\nprobe_ok=1\n",
                    flag(dir.join("disable").is_file())
                );
            }
        }
    }
    "installed=0\nprobe_ok=1\n".into()
}
