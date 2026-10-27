//! Ports phase (core-2.0 §11, step 3): core projects UID policy and applies
//! IPv4/IPv6 firewall rules directly, reusing the activator Ports implementation.
//!
//! Opt-in via [`PORTS_ENV`] (`VPNHIDE_PORTS=1`) — same isolation model as the
//! Native control node: the sandbox core never touches live iptables unless
//! production (or a test) opts in.
//!
//! Partial application is honest: IPv4 and IPv6 are independent restores. The
//! phase outcome is the worst of the two families; [`PortsFamilyReport`] on the
//! operation record carries both. An uncertain apply stays Unknown after restart:
//! chain/jump presence does not prove the intended policy was installed.

use std::path::Path;

use vpnhide_activator::{
    PackageUidMap, PortsApplyResult, PortsFamilyResult, PortsRuleset, apply_ports_rules_families,
    parse_canonical, parse_pm_packages, project_ports_with_resolver, write_ports_core_owner_marker,
};
use vpnhide_core_rpc::{CanonicalConfig, FamilyApplyOutcome, PhaseOutcome, PortsFamilyReport};

use crate::config_file;

fn has_ports_targets(cfg: &CanonicalConfig) -> bool {
    cfg.apps.values().any(|app| app.ports)
}

/// When set to a truthy value (`1`, `true`, `yes`), core includes Ports in the
/// operation plan and applies firewall rules.
pub const PORTS_ENV: &str = "VPNHIDE_PORTS";

/// Optional file of `cmd package list packages -U` output for host tests (skips live PackageManager).
pub const PORTS_PM_DUMP_ENV: &str = "VPNHIDE_PORTS_PM_DUMP";

pub fn ports_enabled_from_env() -> bool {
    match std::env::var(PORTS_ENV) {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}

fn map_family(r: PortsFamilyResult) -> FamilyApplyOutcome {
    match r {
        PortsFamilyResult::Confirmed => FamilyApplyOutcome::Confirmed,
        PortsFamilyResult::FailedKnown => FamilyApplyOutcome::FailedKnown,
        PortsFamilyResult::Unknown => FamilyApplyOutcome::Unknown,
    }
}

fn overall_phase(r: &PortsApplyResult) -> PhaseOutcome {
    match r.overall() {
        PortsFamilyResult::Confirmed => PhaseOutcome::Confirmed,
        PortsFamilyResult::FailedKnown => PhaseOutcome::FailedKnown,
        PortsFamilyResult::Unknown => PhaseOutcome::Unknown,
    }
}

pub fn to_family_report(r: &PortsApplyResult) -> PortsFamilyReport {
    PortsFamilyReport {
        ipv4: map_family(r.ipv4),
        ipv6: map_family(r.ipv6),
        target_count: r.target_count as u32,
    }
}

/// Resolve every Android profile before replacing any live rule. An unavailable
/// PackageManager is an error, never evidence that the target set is empty.
pub fn resolve_uid_map(candidate: &CanonicalConfig) -> Result<PackageUidMap, String> {
    if !has_ports_targets(candidate) {
        return Ok(PackageUidMap::default());
    }
    if let Ok(path) = std::env::var(PORTS_PM_DUMP_ENV) {
        return std::fs::read_to_string(&path)
            .map(|text| parse_pm_packages(&text))
            .map_err(|error| format!("ports UID dump unreadable ({path}): {error}"));
    }
    // Live `cmd package` waits up to a minute for PackageManager. Host/dev sandboxes
    // have no Android PM — skip rather than hang. Production Android has /system/bin/cmd.
    if !Path::new("/system/bin/cmd").is_file() {
        return Err("ports UID resolution unavailable (no Android PackageManager)".into());
    }
    PackageUidMap::from_pm().map_err(|error| format!("ports UID resolution failed: {error}"))
}

pub fn project(
    candidate: &CanonicalConfig,
    resolver: &PackageUidMap,
) -> Result<PortsRuleset, String> {
    let json = config_file::to_json_string(candidate);
    parse_canonical(&json)
        .map(|config| project_ports_with_resolver(&config, resolver))
        .map_err(|error| format!("ports projection failed: {error}"))
}

/// Apply projected rules and claim ownership.
pub fn apply(rules: &PortsRuleset) -> (PhaseOutcome, PortsFamilyReport) {
    let (outcome, report, _) = apply_detailed(rules);
    (outcome, report)
}

/// Like [`apply`], also returning a one-line detail (families plus the apply log tail)
/// for the published Ports status.
pub fn apply_detailed(rules: &PortsRuleset) -> (PhaseOutcome, PortsFamilyReport, String) {
    let result = apply_ports_rules_families(rules);
    // Claim ownership even on partial success so legacy uninstall cannot wipe
    // remnants core is responsible for.
    if result.ipv4 == PortsFamilyResult::Confirmed
        || result.ipv6 == PortsFamilyResult::Confirmed
        || result.overall() == PortsFamilyResult::Unknown
    {
        if let Err(err) = write_ports_core_owner_marker() {
            eprintln!("core: failed to write ports owner marker: {err}");
        }
    }
    let report = to_family_report(&result);
    // The result is published typed (`PortsApply` in the native boot status / receipts);
    // the 1.x `load_status`/`load_log` files are no longer written.
    let detail = format!("ipv4={:?} ipv6={:?}", report.ipv4, report.ipv6);
    let log = result.log.trim();
    let tail: String = {
        let chars: Vec<char> = log.chars().collect();
        chars[chars.len().saturating_sub(400)..].iter().collect()
    };
    let detail = if tail.is_empty() {
        detail
    } else {
        format!("{detail}; {}", tail.replace('\n', " | "))
    };
    (overall_phase(&result), report, detail)
}

/// Full Ports phase: resolve → project → apply.
pub fn run_ports_phase(candidate: &CanonicalConfig) -> (PhaseOutcome, PortsFamilyReport) {
    let (outcome, report, _) = run_ports_phase_detailed(candidate);
    (outcome, report)
}

/// [`run_ports_phase`] plus the published detail line.
pub fn run_ports_phase_detailed(
    candidate: &CanonicalConfig,
) -> (PhaseOutcome, PortsFamilyReport, String) {
    match resolve_uid_map(candidate).and_then(|resolver| project(candidate, &resolver)) {
        Ok(rules) => apply_detailed(&rules),
        Err(error) => (
            PhaseOutcome::FailedKnown,
            PortsFamilyReport {
                ipv4: FamilyApplyOutcome::NotAttempted,
                ipv6: FamilyApplyOutcome::NotAttempted,
                target_count: 0,
            },
            error,
        ),
    }
}

/// netd rebuilds its baseline chains during boot. Wait (at most `max` seconds) until
/// `bw_OUTPUT` exists before the first boot pass; true when it appeared.
pub fn wait_for_netd(max: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + max;
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return false;
        }
        let ready = vpnhide_activator::output_with_timeout(
            std::process::Command::new("iptables").args(["-w", "5", "-L", "bw_OUTPUT", "-n"]),
            left.min(std::time::Duration::from_secs(10)),
        )
        .is_ok_and(|out| out.status.success());
        if ready {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

/// Lock for tests that mutate PATH / VPNHIDE_PORTS* (process-global).
#[cfg(test)]
pub(crate) fn test_env_lock() -> std::sync::MutexGuard<'static, ()> {
    tests::PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex;

    // Host iptables stubs are process-global via PATH; serialize tests that mutate it.
    pub(crate) static PATH_LOCK: Mutex<()> = Mutex::new(());

    fn write_stub_bin(dir: &Path, name: &str, script: &str) {
        let path = dir.join(name);
        fs::write(&path, script).unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
    }

    fn dual_ok_stubs(dir: &Path) {
        // iptables / ip6tables: -N always ok; -C fails then -I ok; -L ok
        let script = r#"#!/bin/sh
case "$*" in
  *"-C OUTPUT"*) exit 1 ;;
  *) exit 0 ;;
esac
"#;
        write_stub_bin(dir, "iptables", script);
        write_stub_bin(dir, "ip6tables", script);
        write_stub_bin(
            dir,
            "iptables-restore",
            "#!/bin/sh\ncat >/dev/null; exit 0\n",
        );
        write_stub_bin(
            dir,
            "ip6tables-restore",
            "#!/bin/sh\ncat >/dev/null; exit 0\n",
        );
    }

    fn ipv6_fail_stubs(dir: &Path) {
        dual_ok_stubs(dir);
        write_stub_bin(
            dir,
            "ip6tables-restore",
            "#!/bin/sh\ncat >/dev/null; exit 1\n",
        );
    }

    fn with_path<R>(dir: &Path, f: impl FnOnce() -> R) -> R {
        let _guard = PATH_LOCK.lock().unwrap();
        // SAFETY: serialized by PATH_LOCK; restored below.
        let old = std::env::var_os("PATH");
        let new_path = format!(
            "{}:{}",
            dir.display(),
            old.as_ref()
                .map(|s| s.to_string_lossy())
                .unwrap_or_default()
        );
        unsafe { std::env::set_var("PATH", &new_path) };
        let out = f();
        match old {
            Some(v) => unsafe { std::env::set_var("PATH", v) },
            None => unsafe { std::env::remove_var("PATH") },
        }
        out
    }

    #[test]
    fn partial_ipv6_failure_reports_families_honestly() {
        let dir = tempfile::tempdir().unwrap();
        ipv6_fail_stubs(dir.path());
        let marker = dir.path().join("ports_owned");

        let (outcome, report) = with_path(dir.path(), || {
            // SAFETY: PATH_LOCK held by with_path.
            unsafe { std::env::set_var("VPNHIDE_PORTS_OWNER_MARKER", &marker) };
            let out = run_ports_phase(&CanonicalConfig::default());
            unsafe { std::env::remove_var("VPNHIDE_PORTS_OWNER_MARKER") };
            out
        });

        assert_eq!(outcome, PhaseOutcome::FailedKnown);
        assert_eq!(report.ipv4, FamilyApplyOutcome::Confirmed);
        assert_eq!(report.ipv6, FamilyApplyOutcome::FailedKnown);
        // Ownership claimed because ipv4 confirmed.
        assert!(marker.is_file());
    }

    #[test]
    fn both_families_ok_confirms() {
        let dir = tempfile::tempdir().unwrap();
        dual_ok_stubs(dir.path());
        let marker = dir.path().join("ports_owned");

        let (outcome, report) = with_path(dir.path(), || {
            unsafe { std::env::set_var("VPNHIDE_PORTS_OWNER_MARKER", &marker) };
            let out = run_ports_phase(&CanonicalConfig::default());
            unsafe { std::env::remove_var("VPNHIDE_PORTS_OWNER_MARKER") };
            out
        });

        assert_eq!(outcome, PhaseOutcome::Confirmed);
        assert_eq!(report.ipv4, FamilyApplyOutcome::Confirmed);
        assert_eq!(report.ipv6, FamilyApplyOutcome::Confirmed);
        assert!(marker.is_file());
    }

    #[test]
    fn failed_uid_scan_never_dispatches_firewall_mutations() {
        let dir = tempfile::tempdir().unwrap();
        let previous = dir.path().join("rules");
        fs::write(&previous, "previous protection").unwrap();
        for binary in [
            "iptables",
            "ip6tables",
            "iptables-restore",
            "ip6tables-restore",
        ] {
            write_stub_bin(
                dir.path(),
                binary,
                "#!/bin/sh\nprintf overwritten >\"$VPNHIDE_TEST_RULES\"\nexit 0\n",
            );
        }
        let mut cfg = CanonicalConfig::default();
        cfg.apps.insert(
            "com.example.app".into(),
            vpnhide_core_rpc::AppConfig {
                ports: true,
                ..Default::default()
            },
        );
        let (outcome, report, detail) = with_path(dir.path(), || {
            // SAFETY: process env is serialized by with_path's PATH_LOCK.
            unsafe {
                std::env::set_var(PORTS_PM_DUMP_ENV, dir.path().join("unreadable-pm"));
                std::env::set_var("VPNHIDE_TEST_RULES", &previous);
            }
            let result = run_ports_phase_detailed(&cfg);
            unsafe {
                std::env::remove_var(PORTS_PM_DUMP_ENV);
                std::env::remove_var("VPNHIDE_TEST_RULES");
            }
            result
        });
        assert_eq!(outcome, PhaseOutcome::FailedKnown);
        assert_eq!(report.ipv4, FamilyApplyOutcome::NotAttempted);
        assert_eq!(report.ipv6, FamilyApplyOutcome::NotAttempted);
        assert!(detail.contains("UID dump unreadable"));
        assert_eq!(fs::read_to_string(previous).unwrap(), "previous protection");
    }

    #[test]
    fn project_resolves_uids_from_pm_dump() {
        let _guard = PATH_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let dump = dir.path().join("pm.txt");
        fs::write(
            &dump,
            "package:com.example.app uid:10123\npackage:com.other uid:10456\n",
        )
        .unwrap();
        unsafe { std::env::set_var(PORTS_PM_DUMP_ENV, &dump) };

        let mut cfg = CanonicalConfig::default();
        cfg.apps.insert(
            "com.example.app".into(),
            vpnhide_core_rpc::AppConfig {
                ports: true,
                ..Default::default()
            },
        );
        let resolver = resolve_uid_map(&cfg).unwrap();
        let rules = project(&cfg, &resolver).unwrap();
        assert_eq!(rules.target_count, 1);
        assert!(rules.ipv4.contains("10123"));
        assert!(!rules.ipv4.contains("10456"));
        unsafe { std::env::remove_var(PORTS_PM_DUMP_ENV) };
    }
}
