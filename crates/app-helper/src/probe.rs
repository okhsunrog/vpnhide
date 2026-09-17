// Ground-truth probe: runs the exact same native checks as the in-process JNI
// path, but exec'd as root (uid 0 is not a hook target) so its view is the
// unfiltered truth. The app diffs this against its own in-process run.
//
// `probe routing --uid <n>`: the self-in-tunnel gate — report whether uid <n>
// is routed through the VPN (a policy rule steers it into a tun table). Emits
// a versioned routing observation instead of the checks payload.
//
use std::collections::BTreeSet;
use std::fs;
use std::process::Command;

use serde::Serialize;
use vpnhide_checks::{
    is_vpn_interface_name, observation, run_all_json, self_routed_for_interfaces,
    self_routed_for_interfaces_json,
};

pub fn checks(args: &[String]) -> i32 {
    if !args.is_empty() {
        eprintln!("usage: vhhelper probe checks");
        return 2;
    }
    println!("{}", run_all_json());
    0
}

pub fn routing(args: &[String]) -> i32 {
    let mut uid = None;
    let mut interfaces = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--uid" if uid.is_none() => {
                let Some(value) = args.get(index + 1) else {
                    return usage_routing();
                };
                let Some(parsed) = value.parse::<u32>().ok() else {
                    return usage_routing();
                };
                uid = Some(parsed);
                index += 2;
            }
            "--vpn-ifaces" if interfaces.is_none() => {
                let Some(value) = args.get(index + 1) else {
                    return usage_routing();
                };
                let Some(parsed) = parse_interfaces(value) else {
                    return usage_routing();
                };
                interfaces = Some(parsed);
                index += 2;
            }
            _ => return usage_routing(),
        }
    }
    let Some(uid) = uid else {
        return usage_routing();
    };
    println!(
        "{}",
        self_routed_for_interfaces_json(uid, interfaces.as_deref())
    );
    0
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum AppVpnState {
    VpnOff,
    Excluded,
    Routed,
    Unknown,
}

#[derive(Serialize)]
struct AppVpnStateObservation {
    uid: u32,
    state: AppVpnState,
    session: Option<String>,
    interfaces: Vec<String>,
    method: &'static str,
    detail: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct FrameworkNetworks {
    vpn_interfaces: BTreeSet<String>,
    non_vpn_interfaces: BTreeSet<String>,
    vpn_network_ids: BTreeSet<String>,
}

/// One privileged observation of both facts the app needs: whether a VPN exists
/// and whether the supplied UID is routed through it. Unlike the legacy polling
/// path, this does not publish or compare global route/rule dumps.
pub fn app_vpn_state(args: &[String]) -> i32 {
    let Some(uid) = parse_uid_only(args) else {
        return usage_app_vpn_state();
    };
    let observation = observe_app_vpn_state(uid);
    println!("{}", observation::app_vpn_state(&observation));
    0
}

fn parse_uid_only(args: &[String]) -> Option<u32> {
    match args {
        [flag, value] if flag == "--uid" => value.parse().ok(),
        _ => None,
    }
}

fn usage_app_vpn_state() -> i32 {
    eprintln!("usage: vhhelper observe app-vpn-state --uid <uid>");
    2
}

fn observe_app_vpn_state(uid: u32) -> AppVpnStateObservation {
    let framework_raw = match command_output("dumpsys", &["connectivity"]) {
        Ok(output) => output,
        Err(()) => return unknown_observation(uid, "framework_unavailable"),
    };
    let framework = match parse_current_networks(&framework_raw) {
        Some(value) => value,
        None => return unknown_observation(uid, "framework_unavailable"),
    };

    let (interfaces, unmanaged) = if framework.vpn_interfaces.is_empty() {
        match unmanaged_vpn_interfaces(&framework.non_vpn_interfaces) {
            Ok(value) => value,
            Err(()) => return unknown_observation(uid, "interfaces_unavailable"),
        }
    } else {
        (framework.vpn_interfaces.clone(), false)
    };
    if interfaces.is_empty() {
        return AppVpnStateObservation {
            uid,
            state: AppVpnState::VpnOff,
            session: None,
            interfaces: Vec::new(),
            method: "none",
            detail: "no_vpn".to_owned(),
        };
    }

    let interface_list = interfaces.iter().cloned().collect::<Vec<_>>();
    let session = Some(session_key(
        &framework.vpn_network_ids,
        &interfaces,
        unmanaged,
    ));
    let (routed, routing_detail) = self_routed_for_interfaces(uid, Some(&interface_list));
    match routed {
        Some(true) => AppVpnStateObservation {
            uid,
            state: AppVpnState::Routed,
            session,
            interfaces: interface_list,
            method: "uid_rule",
            detail: "uid_in_vpn_table".to_owned(),
        },
        None => unknown_with_context(uid, session, interface_list, routing_detail),
        Some(false) => AppVpnStateObservation {
            uid,
            state: AppVpnState::Excluded,
            session,
            interfaces: interface_list,
            method: "uid_rule",
            detail: "uid_not_in_vpn_table".to_owned(),
        },
    }
}

fn unknown_observation(uid: u32, detail: &'static str) -> AppVpnStateObservation {
    unknown_with_context(uid, None, Vec::new(), detail.to_owned())
}

fn unknown_with_context(
    uid: u32,
    session: Option<String>,
    interfaces: Vec<String>,
    detail: String,
) -> AppVpnStateObservation {
    AppVpnStateObservation {
        uid,
        state: AppVpnState::Unknown,
        session,
        interfaces,
        method: "none",
        detail,
    }
}

fn command_output(program: &str, args: &[&str]) -> Result<String, ()> {
    let output = Command::new(program).args(args).output().map_err(|_| ())?;
    if !output.status.success() {
        return Err(());
    }
    String::from_utf8(output.stdout).map_err(|_| ())
}

fn parse_current_networks(raw: &str) -> Option<FrameworkNetworks> {
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

fn unmanaged_vpn_interfaces(
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

fn session_key(
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

fn usage_routing() -> i32 {
    eprintln!("usage: vhhelper probe routing --uid <uid> [--vpn-ifaces <comma-separated names>]");
    2
}

fn parse_interfaces(raw: &str) -> Option<Vec<String>> {
    if raw.is_empty() {
        return Some(Vec::new());
    }
    let interfaces = raw.split(',').map(str::to_owned).collect::<Vec<_>>();
    interfaces
        .iter()
        .all(|name| {
            !name.is_empty()
                && name.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'-')
                })
        })
        .then_some(interfaces)
}

#[cfg(test)]
mod tests {
    use super::{
        AppVpnState, AppVpnStateObservation, BTreeSet, app_vpn_state, checks,
        parse_current_networks, parse_interfaces, routing,
    };
    use serde_json::Value;
    use vpnhide_checks::observation;

    #[test]
    fn probe_modes_reject_cross_mode_flags_before_running_a_probe() {
        assert_eq!(checks(&["--uid".to_owned(), "10042".to_owned()]), 2);
        assert_eq!(routing(&[]), 2);
        assert_eq!(routing(&["--uid".to_owned(), "x".to_owned()]), 2);
        assert_eq!(app_vpn_state(&[]), 2);
    }

    #[test]
    fn routing_arguments_are_strict_but_empty_interface_set_is_valid() {
        assert_eq!(
            parse_interfaces("tun0,vpn.1"),
            Some(vec!["tun0".into(), "vpn.1".into()])
        );
        assert_eq!(parse_interfaces(""), Some(Vec::new()));
        assert_eq!(parse_interfaces("tun0;rm"), None);
        assert_eq!(parse_interfaces("tun0,"), None);
    }

    #[test]
    fn current_network_parser_separates_real_vpn_from_ims() {
        let raw = "header\nCurrent Networks:\n  NetworkAgentInfo{network{101} ni{MOBILE CONNECTED} lp{InterfaceName: ipsec1} nc{[ Transports: CELLULAR Capabilities: IMS&NOT_VPN ]}}\n  NetworkAgentInfo{network{143} ni{VPN CONNECTED} lp{InterfaceName: wg-client} nc{[ Transports: WIFI|VPN Capabilities: INTERNET ]}}\nRequests:\n";
        let parsed = parse_current_networks(raw).unwrap();
        assert_eq!(
            parsed.vpn_interfaces,
            BTreeSet::from(["wg-client".to_owned()])
        );
        assert_eq!(
            parsed.non_vpn_interfaces,
            BTreeSet::from(["ipsec1".to_owned()])
        );
        assert_eq!(parsed.vpn_network_ids, BTreeSet::from(["143".to_owned()]));
    }

    #[test]
    fn app_vpn_state_payload_matches_shared_fixture() {
        let actual: Value =
            serde_json::from_str(&observation::app_vpn_state(&AppVpnStateObservation {
                uid: 10042,
                state: AppVpnState::Routed,
                session: Some("framework:143:tun0".to_owned()),
                interfaces: vec!["tun0".to_owned()],
                method: "uid_rule",
                detail: "uid_in_vpn_table".to_owned(),
            }))
            .unwrap();
        let expected: Value = serde_json::from_str(include_str!(
            "../../../fixtures/app-helper/app-vpn-state-routed.json"
        ))
        .unwrap();
        assert_eq!(actual, expected);
    }
}
