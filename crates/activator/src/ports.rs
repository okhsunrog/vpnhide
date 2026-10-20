use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::process::{Command, Output, Stdio};

use crate::{
    PORTS_CHAIN4, PORTS_CHAIN6, PortProtocol, PortRule, PortUidPolicy, PortsActivationReport,
    PortsRuleset, Result, write_atomic,
};

// Wait (bounded) for the xtables lock instead of failing instantly. netd, an
// OEM firewall (e.g. MIUI), or our own back-to-back iptables/ip6tables calls
// can hold it, so an unguarded restore aborts with "Another app is currently
// holding the xtables lock" (exit 4) and the whole ports apply is marked
// failed. Every iptables-family invocation passes these first; the timeout
// keeps a stuck lock from hanging boot.
/// Wait for the xtables lock instead of failing at once: netd on a network
/// change and the VPN client hold it too, and a check or teardown that gives up
/// immediately reads "absent" where the rules are merely locked.
pub(crate) const XTABLES_WAIT: [&str; 2] = ["-w", "5"];

pub(crate) fn build_ports_ruleset(
    chain: &str,
    loopback: &str,
    udp_reject: &str,
    targets: &BTreeMap<u32, PortUidPolicy>,
) -> String {
    let mut out = String::new();
    out.push_str("*filter\n");
    out.push_str(&format!(":{chain} - [0:0]\n"));
    for (uid, policy) in targets {
        if policy.all_ports {
            append_full_ports_rules(&mut out, chain, loopback, udp_reject, *uid);
        } else {
            for rule in &policy.rules {
                append_port_rule(&mut out, chain, loopback, udp_reject, *uid, *rule);
            }
        }
    }
    out.push_str(&format!("-A {chain} -j RETURN\n"));
    out.push_str("COMMIT\n");
    out
}

fn append_full_ports_rules(
    out: &mut String,
    chain: &str,
    loopback: &str,
    udp_reject: &str,
    uid: u32,
) {
    out.push_str(&format!(
        "-A {chain} -m owner --uid-owner {uid} -d {loopback} -p tcp -j REJECT --reject-with tcp-reset\n",
    ));
    out.push_str(&format!(
        "-A {chain} -m owner --uid-owner {uid} -d {loopback} -p udp -j REJECT --reject-with {udp_reject}\n",
    ));
}

fn append_port_rule(
    out: &mut String,
    chain: &str,
    loopback: &str,
    udp_reject: &str,
    uid: u32,
    rule: PortRule,
) {
    match rule.protocol {
        PortProtocol::Both => {
            append_protocol_port_rule(out, chain, loopback, "tcp", "tcp-reset", uid, rule);
            append_protocol_port_rule(out, chain, loopback, "udp", udp_reject, uid, rule);
        }
        PortProtocol::Tcp => {
            append_protocol_port_rule(out, chain, loopback, "tcp", "tcp-reset", uid, rule);
        }
        PortProtocol::Udp => {
            append_protocol_port_rule(out, chain, loopback, "udp", udp_reject, uid, rule);
        }
    }
}

fn append_protocol_port_rule(
    out: &mut String,
    chain: &str,
    loopback: &str,
    proto: &str,
    reject: &str,
    uid: u32,
    rule: PortRule,
) {
    out.push_str(&format!(
        "-A {chain} -m owner --uid-owner {uid} -d {loopback} -p {proto} --dport {} -j REJECT --reject-with {reject}\n",
        port_match(rule),
    ));
}

fn port_match(rule: PortRule) -> String {
    let end = rule.end_port();
    if rule.start == end {
        rule.start.to_string()
    } else {
        format!("{}:{end}", rule.start)
    }
}

/// Per-address-family outcome of a Ports apply. IPv4 and IPv6 are independent
/// restores — never one atomic transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortsFamilyResult {
    Confirmed,
    FailedKnown,
    /// Spawn/IO failure after we may have mutated tables — do not invent absence.
    Unknown,
}

/// Presence of the Ports chains / OUTPUT jumps (restart inspection).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PortsChainPresence {
    pub ipv4_chain: bool,
    pub ipv6_chain: bool,
    pub ipv4_jump: bool,
    pub ipv6_jump: bool,
}

impl PortsChainPresence {
    /// True when any remnant of a prior apply is still visible.
    pub fn any_present(self) -> bool {
        self.ipv4_chain || self.ipv6_chain || self.ipv4_jump || self.ipv6_jump
    }
}

/// Honest dual-family apply report for core (and for legacy activator aggregation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortsApplyResult {
    pub ipv4: PortsFamilyResult,
    pub ipv6: PortsFamilyResult,
    pub target_count: usize,
    pub log: String,
}

impl PortsApplyResult {
    /// Worst of the two families for a single phase outcome: Unknown > FailedKnown > Confirmed.
    pub fn overall(&self) -> PortsFamilyResult {
        use PortsFamilyResult::*;
        match (self.ipv4, self.ipv6) {
            (Unknown, _) | (_, Unknown) => Unknown,
            (FailedKnown, _) | (_, FailedKnown) => FailedKnown,
            (Confirmed, Confirmed) => Confirmed,
        }
    }
}

/// Env override for the core-ownership marker path (host tests).
pub const PORTS_OWNER_MARKER_ENV: &str = "VPNHIDE_PORTS_OWNER_MARKER";

pub fn ports_owner_marker_path() -> std::path::PathBuf {
    std::env::var_os(PORTS_OWNER_MARKER_ENV)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(crate::PORTS_CORE_OWNER_MARKER))
}

pub fn ports_owned_by_core() -> bool {
    ports_owner_marker_path().is_file()
}

pub fn write_ports_core_owner_marker() -> Result<()> {
    let path = ports_owner_marker_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    write_atomic(&path, b"owned-by-core\n", 0o644)?;
    Ok(())
}

/// Inspect surviving Ports chains/jumps. Used on restart instead of assuming
/// daemon death cleared netfilter state.
pub fn inspect_ports_chains() -> PortsChainPresence {
    PortsChainPresence {
        ipv4_chain: chain_exists("iptables", PORTS_CHAIN4),
        ipv6_chain: chain_exists("ip6tables", PORTS_CHAIN6),
        ipv4_jump: jump_exists("iptables", PORTS_CHAIN4),
        ipv6_jump: jump_exists("ip6tables", PORTS_CHAIN6),
    }
}

fn chain_exists(program: &str, chain: &str) -> bool {
    Command::new(program)
        .args(XTABLES_WAIT)
        .args(["-L", chain, "-n"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn jump_exists(program: &str, chain: &str) -> bool {
    Command::new(program)
        .args(XTABLES_WAIT)
        .args(["-C", "OUTPUT", "-j", chain])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Apply Ports rules with independent IPv4/IPv6 outcomes.
pub fn apply_ports_rules_families(rules: &PortsRuleset) -> PortsApplyResult {
    let mut log = String::new();

    if let Ok(out) = Command::new("iptables")
        .args(XTABLES_WAIT)
        .args(["-N", PORTS_CHAIN4])
        .output()
    {
        append_command_output(&mut log, "iptables -N vpnhide_out", &out);
    }
    if let Ok(out) = Command::new("ip6tables")
        .args(XTABLES_WAIT)
        .args(["-N", PORTS_CHAIN6])
        .output()
    {
        append_command_output(&mut log, "ip6tables -N vpnhide_out6", &out);
    }

    let ipv4 = apply_one_family(
        "iptables",
        "iptables-restore",
        PORTS_CHAIN4,
        &rules.ipv4,
        &mut log,
    );
    let ipv6 = apply_one_family(
        "ip6tables",
        "ip6tables-restore",
        PORTS_CHAIN6,
        &rules.ipv6,
        &mut log,
    );

    PortsApplyResult {
        ipv4,
        ipv6,
        target_count: rules.target_count,
        log,
    }
}

fn apply_one_family(
    tables: &str,
    restore: &str,
    chain: &str,
    ruleset: &str,
    log: &mut String,
) -> PortsFamilyResult {
    let rc = match run_with_stdin(restore, &["--noflush"], ruleset) {
        Ok(out) => {
            append_command_output(log, &format!("{restore} --noflush"), &out);
            out
        }
        Err(err) => {
            log.push_str(&format!("$ {restore} --noflush\nerror={err}\n"));
            // Restore may have partially applied; treat spawn failure as Unknown.
            return PortsFamilyResult::Unknown;
        }
    };
    if !rc.status.success() {
        return PortsFamilyResult::FailedKnown;
    }
    match ensure_output_jump(tables, chain, log) {
        Ok(()) => PortsFamilyResult::Confirmed,
        Err(err) => {
            log.push_str(&format!("jump error: {err}\n"));
            // Jump insert failed after restore succeeded — tables changed; Unknown
            // would over-claim uncertainty for a clear insert failure. FailedKnown.
            PortsFamilyResult::FailedKnown
        }
    }
}

/// Legacy activator entry: both families must succeed.
pub(crate) fn apply_ports_rules(rules: &PortsRuleset) -> Result<PortsActivationReport> {
    let result = apply_ports_rules_families(rules);
    match result.overall() {
        PortsFamilyResult::Confirmed => Ok(PortsActivationReport {
            target_count: result.target_count,
            log: result.log,
        }),
        PortsFamilyResult::FailedKnown | PortsFamilyResult::Unknown => Err(format!(
            "ports apply failed: ipv4={:?} ipv6={:?}\n{}",
            result.ipv4,
            result.ipv6,
            result.log.trim_end()
        )
        .into()),
    }
}

fn run_with_stdin(program: &str, args: &[&str], stdin: &str) -> Result<Output> {
    let mut child = Command::new(program)
        .args(XTABLES_WAIT)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    {
        let pipe = child.stdin.as_mut().ok_or("stdin pipe unavailable")?;
        pipe.write_all(stdin.as_bytes())?;
    }
    Ok(child.wait_with_output()?)
}

fn ensure_output_jump(program: &str, chain: &str, log: &mut String) -> Result<()> {
    let check = Command::new(program)
        .args(XTABLES_WAIT)
        .args(["-C", "OUTPUT", "-j", chain])
        .output()?;
    append_command_output(log, &format!("{program} -C OUTPUT -j {chain}"), &check);
    if check.status.success() {
        return Ok(());
    }
    let insert = Command::new(program)
        .args(XTABLES_WAIT)
        .args(["-I", "OUTPUT", "-j", chain])
        .output()?;
    append_command_output(log, &format!("{program} -I OUTPUT -j {chain}"), &insert);
    if insert.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{program} failed to insert OUTPUT jump to {chain}: {}\n{}",
            insert.status,
            log.trim_end()
        )
        .into())
    }
}

fn append_command_output(log: &mut String, label: &str, output: &Output) {
    log.push_str("$ ");
    log.push_str(label);
    log.push('\n');
    log.push_str("status=");
    log.push_str(&output.status.to_string());
    log.push('\n');
    append_output_stream(log, "stdout", &output.stdout);
    append_output_stream(log, "stderr", &output.stderr);
}

fn append_output_stream(log: &mut String, label: &str, bytes: &[u8]) {
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    log.push_str(label);
    log.push_str(":\n");
    log.push_str(trimmed);
    log.push('\n');
}

#[cfg(test)]
mod ownership_tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn owner_marker_roundtrip() {
        let _g = ENV_LOCK.lock().unwrap();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("vpnhide-ports-owner-{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("owned");
        // SAFETY: ENV_LOCK held.
        unsafe { std::env::set_var(PORTS_OWNER_MARKER_ENV, &marker) };
        assert!(!ports_owned_by_core());
        write_ports_core_owner_marker().unwrap();
        assert!(ports_owned_by_core());
        unsafe { std::env::remove_var(PORTS_OWNER_MARKER_ENV) };
        let _ = fs::remove_dir_all(&dir);
    }
}
