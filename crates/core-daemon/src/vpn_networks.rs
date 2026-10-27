//! Shared framework VPN parsing and session identity for continuous core observations
//! and explicit diagnostic probes. Name-matched unmanaged tunnels remain separately
//! identified; they never become framework TRANSPORT_VPN evidence.

use std::collections::BTreeSet;
use std::fs;
use vpnhide_checks::is_vpn_interface_name;

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct FrameworkNetworks {
    pub vpn_interfaces: BTreeSet<String>,
    pub non_vpn_interfaces: BTreeSet<String>,
    pub vpn_network_ids: BTreeSet<String>,
}

pub(crate) fn parse_current_networks(raw: &str) -> Option<FrameworkNetworks> {
    let mut current = false;
    let mut found = false;
    let mut result = FrameworkNetworks::default();
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
            result.vpn_interfaces.insert(interface);
            if let Some(id) = network_id(trimmed) {
                result.vpn_network_ids.insert(id);
            }
        } else if trimmed.contains("NOT_VPN") && !transports.is_empty() {
            result.non_vpn_interfaces.insert(interface);
        }
    }
    found.then_some(result)
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

fn network_id(raw: &str) -> Option<String> {
    let value = raw.split_once("network{")?.1;
    let digits = value
        .bytes()
        .take_while(u8::is_ascii_digit)
        .collect::<Vec<_>>();
    (!digits.is_empty()).then(|| String::from_utf8(digits).expect("digits are UTF-8"))
}

pub(crate) fn unmanaged_vpn_interfaces(
    framework_non_vpn: &BTreeSet<String>,
) -> Result<(BTreeSet<String>, bool), ()> {
    let candidates = fs::read_dir("/sys/class/net")
        .map_err(|_| ())?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| is_vpn_interface_name(name) && !framework_non_vpn.contains(name))
        .filter(|name| {
            fs::read_to_string(format!("/sys/class/net/{name}/operstate"))
                .is_ok_and(|state| matches!(state.trim(), "up" | "unknown"))
        })
        .collect::<BTreeSet<_>>();
    Ok((candidates, true))
}

pub(crate) fn session_key(
    network_ids: &BTreeSet<String>,
    interfaces: &BTreeSet<String>,
    unmanaged: bool,
) -> String {
    format!(
        "{}:{}:{}",
        if unmanaged { "unmanaged" } else { "framework" },
        network_ids.iter().cloned().collect::<Vec<_>>().join(","),
        interfaces.iter().cloned().collect::<Vec<_>>().join(",")
    )
}
