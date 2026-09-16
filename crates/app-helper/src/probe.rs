// Ground-truth probe: runs the exact same native checks as the in-process JNI
// path, but exec'd as root (uid 0 is not a hook target) so its view is the
// unfiltered truth. The app diffs this against its own in-process run.
//
// `probe routing --uid <n>`: the self-in-tunnel gate — report whether uid <n>
// is routed through the VPN (a policy rule steers it into a tun table). Emits
// a versioned routing observation instead of the checks payload.
//
use vpnhide_checks::{run_all_json, self_routed_for_interfaces_json};

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
    use super::{checks, parse_interfaces, routing};

    #[test]
    fn probe_modes_reject_cross_mode_flags_before_running_a_probe() {
        assert_eq!(checks(&["--uid".to_owned(), "10042".to_owned()]), 2);
        assert_eq!(routing(&[]), 2);
        assert_eq!(routing(&["--uid".to_owned(), "x".to_owned()]), 2);
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
}
