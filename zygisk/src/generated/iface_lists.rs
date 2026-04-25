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
    // catch-all for renamed clients (myvpn0, vpn-client, xvpn1, ...)
    if contains_ci(name, b"vpn") {
        return true;
    }
    false
}
