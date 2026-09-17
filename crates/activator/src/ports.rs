use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    PORTS_CHAIN4, PORTS_CHAIN6, PORTS_LOAD_LOG, PORTS_LOAD_STATUS, PORTS_STATUS_DIR, PortProtocol,
    PortRule, PortUidPolicy, PortsActivationReport, PortsRuleset, Result, write_atomic,
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

pub(crate) fn apply_ports_rules(rules: &PortsRuleset) -> Result<PortsActivationReport> {
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

    let rc4 = run_with_stdin("iptables-restore", &["--noflush"], &rules.ipv4)?;
    append_command_output(&mut log, "iptables-restore --noflush", &rc4);
    let rc6 = run_with_stdin("ip6tables-restore", &["--noflush"], &rules.ipv6)?;
    append_command_output(&mut log, "ip6tables-restore --noflush", &rc6);

    ensure_output_jump("iptables", PORTS_CHAIN4, &mut log)?;
    ensure_output_jump("ip6tables", PORTS_CHAIN6, &mut log)?;

    if !rc4.status.success() || !rc6.status.success() {
        return Err(format!(
            "ports apply failed: rc4={} rc6={}\n{}",
            rc4.status,
            rc6.status,
            log.trim_end()
        )
        .into());
    }
    Ok(PortsActivationReport {
        target_count: rules.target_count,
        log,
    })
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

pub(crate) fn write_ports_load_status(
    source: &str,
    loaded: bool,
    target_count: Option<usize>,
    detail: &str,
    log: &str,
) {
    if let Err(err) = write_ports_load_status_inner(source, loaded, target_count, detail, log) {
        eprintln!("vpnhide ports activator failed to write load_status: {err}");
    }
}

fn write_ports_load_status_inner(
    source: &str,
    loaded: bool,
    target_count: Option<usize>,
    detail: &str,
    log: &str,
) -> Result<()> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| String::new());
    let boot_id = read_trimmed("/proc/sys/kernel/random/boot_id");
    let uname_r = command_stdout_trimmed("uname", &["-r"]);
    let status = format!(
        "timestamp={timestamp}\nboot_id={boot_id}\nuname_r={uname_r}\nruntime=ports\nsource={source}\nloaded={}\ntarget_count={}\ndetail={}\n",
        if loaded { 1 } else { 0 },
        target_count
            .map(|count| count.to_string())
            .unwrap_or_else(|| "unknown".to_owned()),
        sanitize_status_value(detail),
    );
    write_atomic(Path::new(PORTS_LOAD_STATUS), status.as_bytes(), 0o644)?;
    let load_log = if log.trim().is_empty() {
        "(empty)\n".to_owned()
    } else {
        format!("{}\n", log.trim_end())
    };
    write_atomic(Path::new(PORTS_LOAD_LOG), load_log.as_bytes(), 0o644)?;
    let _ = fs::set_permissions(PORTS_STATUS_DIR, fs::Permissions::from_mode(0o755));
    Ok(())
}

fn read_trimmed(path: &str) -> String {
    fs::read_to_string(path)
        .map(|s| s.trim().to_owned())
        .unwrap_or_default()
}

fn command_stdout_trimmed(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .output()
        .ok()
        .and_then(|out| out.status.success().then_some(out.stdout))
        .map(|stdout| String::from_utf8_lossy(&stdout).trim().to_owned())
        .unwrap_or_default()
}

fn sanitize_status_value(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            '\n' | '\r' | '\t' => ' ',
            _ => ch,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(240)
        .collect()
}
