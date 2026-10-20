//! Ports phase (core-2.0 §11, step 3): core projects UID policy and applies
//! IPv4/IPv6 firewall rules directly, reusing the activator Ports implementation.
//!
//! Opt-in via [`PORTS_ENV`] (`VPNHIDE_PORTS=1`) — same isolation model as the
//! Native control node: the sandbox core never touches live iptables unless
//! production (or a test) opts in.
//!
//! Partial application is honest: IPv4 and IPv6 are independent restores. The
//! phase outcome is the worst of the two families; [`PortsFamilyReport`] on the
//! operation record carries both. On restart, [`inspect_surviving_rules`] reads
//! chain/jump presence rather than assuming daemon death cleared netfilter.

use std::path::{Path, PathBuf};

use vpnhide_activator::{
    PackageUidMap, PortsApplyResult, PortsChainPresence, PortsFamilyResult, PortsRuleset,
    apply_ports_rules_families, inspect_ports_chains, parse_canonical, parse_pm_packages,
    project_ports_with_resolver, write_ports_core_owner_marker,
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

/// Resolve UIDs: prefer a test dump, else live PackageManager, else empty map
/// (still applies a RETURN-only chain — cleanup / no targets).
pub fn resolve_uid_map(candidate: &CanonicalConfig) -> PackageUidMap {
    if let Ok(path) = std::env::var(PORTS_PM_DUMP_ENV) {
        match std::fs::read_to_string(&path) {
            Ok(text) => return parse_pm_packages(&text),
            Err(err) => eprintln!("core: ports pm dump unreadable ({path}): {err}"),
        }
    }
    if !has_ports_targets(candidate) {
        return PackageUidMap::default();
    }
    // Live `cmd package` waits up to a minute for PackageManager. Host/dev sandboxes
    // have no Android PM — skip rather than hang. Production Android has /system/bin/cmd.
    if !Path::new("/system/bin/cmd").is_file() {
        eprintln!("core: ports UID resolve skipped (no Android pm on this host)");
        return PackageUidMap::default();
    }
    match PackageUidMap::from_pm() {
        Ok(map) => map,
        Err(err) => {
            eprintln!("core: ports UID resolve via pm failed: {err}");
            PackageUidMap::default()
        }
    }
}

pub fn project(candidate: &CanonicalConfig, resolver: &PackageUidMap) -> PortsRuleset {
    let json = config_file::to_json_string(candidate);
    match parse_canonical(&json) {
        Ok(cfg) => project_ports_with_resolver(&cfg, resolver),
        Err(err) => {
            eprintln!("core: ports projection parse failed: {err}");
            PortsRuleset {
                ipv4: String::new(),
                ipv6: String::new(),
                target_count: 0,
            }
        }
    }
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
    let resolver = resolve_uid_map(candidate);
    let rules = project(candidate, &resolver);
    apply_detailed(&rules)
}

/// netd rebuilds its baseline chains during boot. Wait (at most `max` seconds) until
/// `bw_OUTPUT` exists before the first boot pass; true when it appeared.
pub fn wait_for_netd(max: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + max;
    loop {
        let ready = std::process::Command::new("iptables")
            .args(["-w", "-L", "bw_OUTPUT", "-n"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if ready {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

/// Restart inspection: surviving chain/jump presence (does not invent absence).
pub fn inspect_surviving_rules() -> PortsChainPresence {
    inspect_ports_chains()
}

/// Bounded Ports Unknown reconciliation: promote to Confirmed only when both
/// families still show chain+jump; to FailedKnown only when both are fully
/// absent. Mixed or unreadable remnants stay Unknown — no auto-repair.
pub fn reconcile_ports_unknown(presence: PortsChainPresence) -> PhaseOutcome {
    let v4_ok = presence.ipv4_chain && presence.ipv4_jump;
    let v6_ok = presence.ipv6_chain && presence.ipv6_jump;
    let v4_gone = !presence.ipv4_chain && !presence.ipv4_jump;
    let v6_gone = !presence.ipv6_chain && !presence.ipv6_jump;
    if v4_ok && v6_ok {
        PhaseOutcome::Confirmed
    } else if v4_gone && v6_gone {
        PhaseOutcome::FailedKnown
    } else {
        PhaseOutcome::Unknown
    }
}

/// Marker path helper for tests.
pub fn owner_marker_path() -> PathBuf {
    vpnhide_activator::ports_owner_marker_path()
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

    fn inspect_partial_stubs(dir: &Path) {
        // IPv4 present, IPv6 absent
        write_stub_bin(
            dir,
            "iptables",
            r#"#!/bin/sh
exit 0
"#,
        );
        write_stub_bin(
            dir,
            "ip6tables",
            r#"#!/bin/sh
exit 1
"#,
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
    fn restart_inspection_mixed_presence_stays_unknown() {
        let dir = tempfile::tempdir().unwrap();
        inspect_partial_stubs(dir.path());
        let presence = with_path(dir.path(), inspect_surviving_rules);
        assert!(presence.ipv4_chain && presence.ipv4_jump);
        assert!(!presence.ipv6_chain && !presence.ipv6_jump);
        assert_eq!(
            reconcile_ports_unknown(presence),
            PhaseOutcome::Unknown,
            "mixed remnants must not invent Confirmed or FailedKnown"
        );
    }

    #[test]
    fn restart_inspection_full_absence_is_failed_known() {
        let dir = tempfile::tempdir().unwrap();
        write_stub_bin(dir.path(), "iptables", "#!/bin/sh\nexit 1\n");
        write_stub_bin(dir.path(), "ip6tables", "#!/bin/sh\nexit 1\n");
        let presence = with_path(dir.path(), inspect_surviving_rules);
        assert!(!presence.any_present());
        assert_eq!(reconcile_ports_unknown(presence), PhaseOutcome::FailedKnown);
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
        let resolver = resolve_uid_map(&cfg);
        let rules = project(&cfg, &resolver);
        assert_eq!(rules.target_count, 1);
        assert!(rules.ipv4.contains("10123"));
        assert!(!rules.ipv4.contains("10456"));
        unsafe { std::env::remove_var(PORTS_PM_DUMP_ENV) };
    }
}
