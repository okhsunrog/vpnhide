//! Framework VPN classification (§8): netlink cannot mark an interface as VPN.
//!
//! Sources (first match wins):
//! 1. `VPNHIDE_FRAMEWORK_VPN_IFACES` — comma list (host tests / forced classify)
//! 2. `VPNHIDE_FRAMEWORK_VPN=unavailable|failed` — force non-current evidence
//! 3. `dumpsys connectivity` parse (device); missing/unparseable → Unavailable
//!
//! Name-match netlink candidates alone must not set freshness to Current.

use std::collections::BTreeSet;
use std::process::Command;

use vpnhide_core_rpc::{FrameworkVpnEvidence, FrameworkVpnStatus};

/// Env: comma-separated framework VPN iface names (test stub / override).
pub const FRAMEWORK_IFACES_ENV: &str = "VPNHIDE_FRAMEWORK_VPN_IFACES";
/// Env: `unavailable` | `failed` | `auto` (default).
pub const FRAMEWORK_MODE_ENV: &str = "VPNHIDE_FRAMEWORK_VPN";

pub fn classify() -> FrameworkVpnEvidence {
    if let Ok(v) = std::env::var(FRAMEWORK_MODE_ENV) {
        let v = v.trim().to_ascii_lowercase();
        if v == "unavailable" {
            return FrameworkVpnEvidence {
                status: FrameworkVpnStatus::Unavailable,
                interfaces: Vec::new(),
                detail: "VPNHIDE_FRAMEWORK_VPN=unavailable".into(),
            };
        }
        if v == "failed" {
            return FrameworkVpnEvidence {
                status: FrameworkVpnStatus::Failed,
                interfaces: Vec::new(),
                detail: "VPNHIDE_FRAMEWORK_VPN=failed".into(),
            };
        }
    }
    if let Ok(raw) = std::env::var(FRAMEWORK_IFACES_ENV) {
        let interfaces = raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        return FrameworkVpnEvidence {
            status: FrameworkVpnStatus::Classified,
            detail: "env_stub".into(),
            interfaces,
        };
    }
    match dumpsys_connectivity() {
        Ok(text) => match parse_vpn_interfaces(&text) {
            Some(interfaces) => FrameworkVpnEvidence {
                status: FrameworkVpnStatus::Classified,
                interfaces: interfaces.into_iter().collect(),
                detail: "dumpsys_connectivity".into(),
            },
            None => FrameworkVpnEvidence {
                status: FrameworkVpnStatus::Unavailable,
                interfaces: Vec::new(),
                detail: "dumpsys_unparsed".into(),
            },
        },
        Err(detail) => FrameworkVpnEvidence {
            status: FrameworkVpnStatus::Unavailable,
            interfaces: Vec::new(),
            detail,
        },
    }
}

fn dumpsys_connectivity() -> Result<String, String> {
    let output = Command::new("dumpsys")
        .args(["connectivity"])
        .output()
        .map_err(|e| format!("dumpsys_spawn:{e}"))?;
    if !output.status.success() {
        return Err(format!("dumpsys_status:{}", output.status));
    }
    String::from_utf8(output.stdout).map_err(|_| "dumpsys_utf8".into())
}

/// Parse `dumpsys connectivity` "Current Networks:" VPN transports (shared shape with vhhelper).
pub fn parse_vpn_interfaces(raw: &str) -> Option<BTreeSet<String>> {
    let mut current = false;
    let mut found = false;
    let mut vpn = BTreeSet::new();
    for line in raw.lines() {
        if line.trim() == "Current Networks:" {
            current = true;
            found = true;
            continue;
        }
        if !current {
            continue;
        }
        if !line.is_empty() && !line.as_bytes()[0].is_ascii_whitespace() {
            break;
        }
        let trimmed = line.trim_start();
        if !trimmed.starts_with("NetworkAgentInfo")
            || trimmed.contains("DISCONNECTED")
            || trimmed.contains("CONNECTING")
        {
            continue;
        }
        let Some(interface) = interface_after(trimmed, "InterfaceName: ") else {
            continue;
        };
        let transports = field_after(trimmed, "Transports: ").unwrap_or_default();
        if transports.split('|').any(|value| value == "VPN") {
            vpn.insert(interface);
        }
    }
    found.then_some(vpn)
}

fn field_after<'a>(raw: &'a str, marker: &str) -> Option<&'a str> {
    let value = raw.split_once(marker)?.1;
    value
        .split(|character: char| character.is_ascii_whitespace() || matches!(character, '}' | ']'))
        .next()
}

fn interface_after(raw: &str, marker: &str) -> Option<String> {
    let token = field_after(raw, marker)?;
    valid_interface(token).then(|| token.to_owned())
}

fn valid_interface(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vpn_transport_from_dumpsys_fixture() {
        let raw = r#"
Current Networks:
  NetworkAgentInfo{ network{100}  InterfaceName: tun0  Transports: VPN|CELLULAR  CONNECTED }
  NetworkAgentInfo{ network{101}  InterfaceName: wlan0  Transports: WIFI  NOT_VPN  CONNECTED }
Other section:
"#;
        let set = parse_vpn_interfaces(raw).unwrap();
        assert_eq!(set.iter().cloned().collect::<Vec<_>>(), vec!["tun0"]);
    }

    #[test]
    fn missing_current_networks_is_none() {
        assert!(parse_vpn_interfaces("no section here").is_none());
    }

    #[test]
    fn env_stub_classifies() {
        let _g = crate::ports::test_env_lock();
        unsafe {
            std::env::set_var(FRAMEWORK_IFACES_ENV, "tun0,wg0");
            std::env::remove_var(FRAMEWORK_MODE_ENV);
        }
        let ev = classify();
        assert_eq!(ev.status, FrameworkVpnStatus::Classified);
        assert_eq!(ev.interfaces, vec!["tun0", "wg0"]);
        unsafe { std::env::remove_var(FRAMEWORK_IFACES_ENV) };
    }
}
