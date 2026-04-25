// AUTO-GENERATED from data/interfaces.toml — do not edit by hand. Regenerate with: python3 scripts/codegen-interfaces.py

#![allow(dead_code)]

fn starts_with_ci(name: &[u8], prefix: &[u8]) -> bool {
    if name.len() < prefix.len() {
        return false;
    }
    for (i, &p) in prefix.iter().enumerate() {
        if name[i].to_ascii_lowercase() != p {
            return false;
        }
    }
    true
}

fn starts_with_then_digits_ci(name: &[u8], prefix: &[u8]) -> bool {
    if !starts_with_ci(name, prefix) {
        return false;
    }
    let rest = &name[prefix.len()..];
    !rest.is_empty() && rest.iter().all(|b| b.is_ascii_digit())
}

fn starts_with_then_digits_optional_ci(name: &[u8], prefix: &[u8]) -> bool {
    if !starts_with_ci(name, prefix) {
        return false;
    }
    name[prefix.len()..].iter().all(|b| b.is_ascii_digit())
}

fn starts_with_then_any_ci(name: &[u8], prefix: &[u8]) -> bool {
    starts_with_ci(name, prefix) && name.len() > prefix.len()
}

fn equals_ci(name: &[u8], other: &[u8]) -> bool {
    if name.len() != other.len() {
        return false;
    }
    name.iter()
        .zip(other.iter())
        .all(|(a, b)| a.to_ascii_lowercase() == *b)
}

fn contains_ci(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }
    for start in 0..=haystack.len() - needle.len() {
        let window = &haystack[start..start + needle.len()];
        if window
            .iter()
            .zip(needle.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
        {
            return true;
        }
    }
    false
}

/// True if the name matches any VPN-iface rule from data/interfaces.toml.
pub fn matches_vpn(name: &[u8]) -> bool {
    if name.is_empty() {
        return false;
    }
    // OpenVPN, WireGuard userspace, Tailscale, generic tunneling
    if starts_with_ci(name, b"tun") {
        return true;
    }
    // OpenVPN bridged
    if starts_with_ci(name, b"tap") {
        return true;
    }
    // WireGuard kernel
    if starts_with_ci(name, b"wg") {
        return true;
    }
    // PPTP / L2TP PPP tunnels
    if starts_with_ci(name, b"ppp") {
        return true;
    }
    // Android built-in IPsec VPN
    if starts_with_ci(name, b"ipsec") {
        return true;
    }
    // kernel IPsec XFRM framework
    if starts_with_ci(name, b"xfrm") {
        return true;
    }
    // Apple-style, rare on Android
    if starts_with_ci(name, b"utun") {
        return true;
    }
    // L2TP
    if starts_with_ci(name, b"l2tp") {
        return true;
    }
    // GRE tunnels
    if starts_with_ci(name, b"gre") {
        return true;
    }
    // catch-all for renamed clients (myvpn0, vpn-client, xvpn1, ...)
    if contains_ci(name, b"vpn") {
        return true;
    }
    false
}

#[cfg(test)]
#[rustfmt::skip]
mod tests {
    use super::*;

    #[test]
    fn generated_vectors() {
        assert_eq!(matches_vpn(b"tun0"), true, "matches_vpn('tun0')");
        assert_eq!(matches_vpn(b"tun"), true, "matches_vpn('tun')");
        assert_eq!(matches_vpn(b"tun1234"), true, "matches_vpn('tun1234')");
        assert_eq!(matches_vpn(b"tap0"), true, "matches_vpn('tap0')");
        assert_eq!(matches_vpn(b"wg0"), true, "matches_vpn('wg0')");
        assert_eq!(matches_vpn(b"wg-client"), true, "matches_vpn('wg-client')");
        assert_eq!(matches_vpn(b"ppp0"), true, "matches_vpn('ppp0')");
        assert_eq!(matches_vpn(b"ipsec0"), true, "matches_vpn('ipsec0')");
        assert_eq!(matches_vpn(b"xfrm0"), true, "matches_vpn('xfrm0')");
        assert_eq!(matches_vpn(b"utun3"), true, "matches_vpn('utun3')");
        assert_eq!(matches_vpn(b"l2tp0"), true, "matches_vpn('l2tp0')");
        assert_eq!(matches_vpn(b"gre0"), true, "matches_vpn('gre0')");
        assert_eq!(matches_vpn(b"TUN0"), true, "matches_vpn('TUN0')");
        assert_eq!(matches_vpn(b"Wg99"), true, "matches_vpn('Wg99')");
        assert_eq!(matches_vpn(b"MyVPN"), true, "matches_vpn('MyVPN')");
        assert_eq!(matches_vpn(b"custom_VPN_42"), true, "matches_vpn('custom_VPN_42')");
        assert_eq!(matches_vpn(b"myvpn0"), true, "matches_vpn('myvpn0')");
        assert_eq!(matches_vpn(b"vpn"), true, "matches_vpn('vpn')");
        assert_eq!(matches_vpn(b"xvpn1"), true, "matches_vpn('xvpn1')");
        assert_eq!(matches_vpn(b"lo"), false, "matches_vpn('lo')");
        assert_eq!(matches_vpn(b"wlan0"), false, "matches_vpn('wlan0')");
        assert_eq!(matches_vpn(b"wlan"), false, "matches_vpn('wlan')");
        assert_eq!(matches_vpn(b"rmnet0"), false, "matches_vpn('rmnet0')");
        assert_eq!(matches_vpn(b"rmnet_data0"), false, "matches_vpn('rmnet_data0')");
        assert_eq!(matches_vpn(b"rmnet_ipa0"), false, "matches_vpn('rmnet_ipa0')");
        assert_eq!(matches_vpn(b"eth0"), false, "matches_vpn('eth0')");
        assert_eq!(matches_vpn(b"ccmni0"), false, "matches_vpn('ccmni0')");
        assert_eq!(matches_vpn(b"seth_lte8"), false, "matches_vpn('seth_lte8')");
        assert_eq!(matches_vpn(b"dummy0"), false, "matches_vpn('dummy0')");
        assert_eq!(matches_vpn(b"bnep0"), false, "matches_vpn('bnep0')");
        assert_eq!(matches_vpn(b"rndis0"), false, "matches_vpn('rndis0')");
        assert_eq!(matches_vpn(b"if33"), false, "matches_vpn('if33')");
        assert_eq!(matches_vpn(b""), false, "matches_vpn('')");
        assert_eq!(matches_vpn(b"tunl"), true, "matches_vpn('tunl')");
        assert_eq!(matches_vpn(b"atun0"), false, "matches_vpn('atun0')");
        assert_eq!(matches_vpn(b"VPN"), true, "matches_vpn('VPN')");
    }

    #[test]
    fn helper_starts_with_then_digits_optional() {
        assert!(starts_with_then_digits_optional_ci(b"foo", b"foo"));
        assert!(starts_with_then_digits_optional_ci(b"foo0", b"foo"));
        assert!(starts_with_then_digits_optional_ci(b"foo123", b"foo"));
        assert!(!starts_with_then_digits_optional_ci(b"foox", b"foo"));
        assert!(!starts_with_then_digits_optional_ci(b"fo", b"foo"));
    }

    #[test]
    fn helper_starts_with_then_any() {
        assert!(starts_with_then_any_ci(b"v4-x", b"v4-"));
        assert!(starts_with_then_any_ci(b"v4-rmnet0", b"v4-"));
        assert!(!starts_with_then_any_ci(b"v4-", b"v4-"));
        assert!(!starts_with_then_any_ci(b"v3-x", b"v4-"));
    }
}
