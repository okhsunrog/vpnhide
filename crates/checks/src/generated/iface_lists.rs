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
    // Tailscale native/root daemon tunnel (standard Android VpnService normally uses tun)
    if starts_with_ci(name, b"tailscale") {
        return true;
    }
    // ZeroTier virtual Ethernet (zt followed by a network-derived interface id)
    if starts_with_ci(name, b"zt") {
        return true;
    }
    // Hurricane Electric IPv6-in-IPv4 tunnel
    if starts_with_ci(name, b"he-ipv6") {
        return true;
    }
    // catch-all for renamed clients (myvpn0, vpn-client, xvpn1, ...)
    if contains_ci(name, b"vpn") {
        return true;
    }
    // Anonymous netdev / renamed tunnel using the kernel's default naming pattern (e.g. `ip link set tun0 name if33` from issue #86). Does NOT match `ifb<N>` — those are kernel intermediate-functional-block traffic-shaping ifaces (different shape: `if` + letter, not + digit).
    if starts_with_then_digits_ci(name, b"if") {
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
        assert!(matches_vpn(b"tun0"), "matches_vpn('tun0')");
        assert!(matches_vpn(b"tun"), "matches_vpn('tun')");
        assert!(matches_vpn(b"tun1234"), "matches_vpn('tun1234')");
        assert!(matches_vpn(b"tap0"), "matches_vpn('tap0')");
        assert!(matches_vpn(b"wg0"), "matches_vpn('wg0')");
        assert!(matches_vpn(b"wg-client"), "matches_vpn('wg-client')");
        assert!(matches_vpn(b"ppp0"), "matches_vpn('ppp0')");
        assert!(matches_vpn(b"ipsec0"), "matches_vpn('ipsec0')");
        assert!(matches_vpn(b"xfrm0"), "matches_vpn('xfrm0')");
        assert!(matches_vpn(b"utun3"), "matches_vpn('utun3')");
        assert!(matches_vpn(b"l2tp0"), "matches_vpn('l2tp0')");
        assert!(matches_vpn(b"gre0"), "matches_vpn('gre0')");
        assert!(matches_vpn(b"tailscale0"), "matches_vpn('tailscale0')");
        assert!(matches_vpn(b"ztyqb6mebi"), "matches_vpn('ztyqb6mebi')");
        assert!(matches_vpn(b"zt0"), "matches_vpn('zt0')");
        assert!(matches_vpn(b"he-ipv6"), "matches_vpn('he-ipv6')");
        assert!(matches_vpn(b"he-ipv6-1"), "matches_vpn('he-ipv6-1')");
        assert!(matches_vpn(b"TUN0"), "matches_vpn('TUN0')");
        assert!(matches_vpn(b"Wg99"), "matches_vpn('Wg99')");
        assert!(matches_vpn(b"MyVPN"), "matches_vpn('MyVPN')");
        assert!(matches_vpn(b"custom_VPN_42"), "matches_vpn('custom_VPN_42')");
        assert!(matches_vpn(b"myvpn0"), "matches_vpn('myvpn0')");
        assert!(matches_vpn(b"vpn"), "matches_vpn('vpn')");
        assert!(matches_vpn(b"xvpn1"), "matches_vpn('xvpn1')");
        assert!(matches_vpn(b"svpn0"), "matches_vpn('svpn0')");
        assert!(!matches_vpn(b"lo"), "matches_vpn('lo')");
        assert!(!matches_vpn(b"wlan0"), "matches_vpn('wlan0')");
        assert!(!matches_vpn(b"wlan"), "matches_vpn('wlan')");
        assert!(!matches_vpn(b"rmnet0"), "matches_vpn('rmnet0')");
        assert!(!matches_vpn(b"rmnet_data0"), "matches_vpn('rmnet_data0')");
        assert!(!matches_vpn(b"rmnet_ipa0"), "matches_vpn('rmnet_ipa0')");
        assert!(!matches_vpn(b"eth0"), "matches_vpn('eth0')");
        assert!(!matches_vpn(b"ccmni0"), "matches_vpn('ccmni0')");
        assert!(!matches_vpn(b"seth_lte8"), "matches_vpn('seth_lte8')");
        assert!(!matches_vpn(b"dummy0"), "matches_vpn('dummy0')");
        assert!(!matches_vpn(b"bnep0"), "matches_vpn('bnep0')");
        assert!(!matches_vpn(b"rndis0"), "matches_vpn('rndis0')");
        assert!(matches_vpn(b"if33"), "matches_vpn('if33')");
        assert!(matches_vpn(b"if0"), "matches_vpn('if0')");
        assert!(matches_vpn(b"if99"), "matches_vpn('if99')");
        assert!(!matches_vpn(b"ifb0"), "matches_vpn('ifb0')");
        assert!(!matches_vpn(b"ifb1"), "matches_vpn('ifb1')");
        assert!(!matches_vpn(b"if"), "matches_vpn('if')");
        assert!(!matches_vpn(b"if_inet6"), "matches_vpn('if_inet6')");
        assert!(!matches_vpn(b""), "matches_vpn('')");
        assert!(matches_vpn(b"tunl"), "matches_vpn('tunl')");
        assert!(!matches_vpn(b"atun0"), "matches_vpn('atun0')");
        assert!(matches_vpn(b"VPN"), "matches_vpn('VPN')");
    }
}
