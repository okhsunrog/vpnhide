//! Framework VPN classification and per-app membership for the continuous observer.
//! Kernel name matches alone never establish TRANSPORT_VPN. The explicit probe and
//! continuous source share one parser/session identity, including unmanaged tunnels.

use crate::vpn_networks::{
    FrameworkNetworks, parse_current_networks, session_key, unmanaged_vpn_interfaces,
};
use std::collections::BTreeSet;
use std::time::Duration;
use vpnhide_checks::self_routed_for_interfaces;
use vpnhide_core_rpc::{
    FrameworkVpnEvidence, FrameworkVpnStatus, UidRoutingObservation, UidVpnRouteState,
};

pub const FRAMEWORK_IFACES_ENV: &str = "VPNHIDE_FRAMEWORK_VPN_IFACES";
pub const FRAMEWORK_MODE_ENV: &str = "VPNHIDE_FRAMEWORK_VPN";

pub fn classify() -> FrameworkVpnEvidence {
    classify_networks().0
}

/// One framework read supplies both VPN classification and the main app's membership.
/// UID session identity changes when Android replaces a VPN even if it reuses tun0.
pub fn sample(uid: Option<u32>) -> (FrameworkVpnEvidence, Option<UidRoutingObservation>) {
    let (framework, networks) = classify_networks();
    let member = uid.map(|uid| membership(uid, &framework, networks.as_ref()));
    (framework, member)
}

fn classify_networks() -> (FrameworkVpnEvidence, Option<FrameworkNetworks>) {
    if let Ok(mode) = std::env::var(FRAMEWORK_MODE_ENV) {
        let status = match mode.trim().to_ascii_lowercase().as_str() {
            "unavailable" => Some(FrameworkVpnStatus::Unavailable),
            "failed" => Some(FrameworkVpnStatus::Failed),
            _ => None,
        };
        if let Some(status) = status {
            return (
                FrameworkVpnEvidence {
                    status,
                    interfaces: Vec::new(),
                    detail: "env_override".into(),
                },
                None,
            );
        }
    }
    if let Ok(raw) = std::env::var(FRAMEWORK_IFACES_ENV) {
        let interfaces: BTreeSet<String> = raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        return (
            FrameworkVpnEvidence {
                status: FrameworkVpnStatus::Classified,
                detail: session_key(&BTreeSet::new(), &interfaces, false),
                interfaces: interfaces.into_iter().collect(),
            },
            None,
        );
    }
    // Reuse core's bounded ELF command runner; a stalled framework cannot occupy the
    // observer indefinitely or grow an unbounded command-output allocation.
    let output = crate::device_probes::run("dumpsys", &["connectivity"], Duration::from_secs(3));
    if output.exit != 0 || output.stdout.len() >= 16 << 20 {
        return (
            FrameworkVpnEvidence {
                status: FrameworkVpnStatus::Unavailable,
                interfaces: Vec::new(),
                detail: format!("dumpsys_status:{}", output.exit),
            },
            None,
        );
    }
    match parse_current_networks(&output.stdout) {
        Some(networks) => (
            FrameworkVpnEvidence {
                status: FrameworkVpnStatus::Classified,
                detail: session_key(&networks.vpn_network_ids, &networks.vpn_interfaces, false),
                interfaces: networks.vpn_interfaces.iter().cloned().collect(),
            },
            Some(networks),
        ),
        None => (
            FrameworkVpnEvidence {
                status: FrameworkVpnStatus::Unavailable,
                interfaces: Vec::new(),
                detail: "dumpsys_unparsed".into(),
            },
            None,
        ),
    }
}

fn membership(
    uid: u32,
    framework: &FrameworkVpnEvidence,
    networks: Option<&FrameworkNetworks>,
) -> UidRoutingObservation {
    let mut observation = UidRoutingObservation {
        uid,
        state: UidVpnRouteState::Unknown,
        detail: framework.detail.clone(),
        session: None,
        interfaces: Vec::new(),
    };
    if framework.status != FrameworkVpnStatus::Classified {
        return observation;
    }
    let mut interfaces = framework
        .interfaces
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut unmanaged = false;
    if interfaces.is_empty() {
        if let Some(networks) = networks {
            match unmanaged_vpn_interfaces(&networks.non_vpn_interfaces) {
                Ok((found, is_unmanaged)) => {
                    interfaces = found;
                    unmanaged = is_unmanaged;
                }
                Err(()) => {
                    observation.detail = "interfaces_unavailable".into();
                    return observation;
                }
            }
        }
    }
    if interfaces.is_empty() {
        observation.state = UidVpnRouteState::VpnOff;
        observation.detail = "no_vpn".into();
        return observation;
    }
    observation.session = Some(if unmanaged {
        session_key(&BTreeSet::new(), &interfaces, true)
    } else {
        framework.detail.clone()
    });
    observation.interfaces = interfaces.into_iter().collect();
    let (routed, detail) = self_routed_for_interfaces(uid, Some(&observation.interfaces));
    observation.state = match routed {
        Some(true) => UidVpnRouteState::Routed,
        Some(false) => UidVpnRouteState::Excluded,
        None => UidVpnRouteState::Unknown,
    };
    observation.detail = detail;
    observation
}

pub fn parse_vpn_interfaces(raw: &str) -> Option<BTreeSet<String>> {
    parse_current_networks(raw).map(|networks| networks.vpn_interfaces)
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
    fn replacing_a_vpn_on_the_same_interface_changes_session_identity() {
        let first = "Current Networks:\n  NetworkAgentInfo{ network{100} InterfaceName: tun0 Transports: VPN CONNECTED }\n";
        let next = first.replace("network{100}", "network{101}");
        let a = parse_current_networks(first).unwrap();
        let b = parse_current_networks(&next).unwrap();
        assert_eq!(a.vpn_interfaces, b.vpn_interfaces);
        assert_ne!(
            session_key(&a.vpn_network_ids, &a.vpn_interfaces, false),
            session_key(&b.vpn_network_ids, &b.vpn_interfaces, false),
        );
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
