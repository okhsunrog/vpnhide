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

/// True if `name` is in the never-hide whitelist from data/interfaces.toml.
pub fn is_never_hide(name: &[u8]) -> bool {
    if name.is_empty() {
        return false;
    }
    // 464XLAT CLAT shadow iface (v4-rmnet0, v4-wlan0, ...). Required on IPv6-only carriers (T-Mobile US, Reliance Jio, ...) — without it IPv4-only apps lose internet. Created by clatd, lives as ARPHRD_NONE TUN, easy to mistake for a VPN tunnel. AOSP source: external/android-clat.
    if starts_with_then_any_ci(name, b"v4-") {
        return true;
    }
    // OpenThread border router on Pixel 7+. Hard-coded in init.rc inside the com.android.tethering APEX (the same APEX that delivers VPN-related code). Used for Matter / smart-home Thread mesh, not connectivity for normal apps.
    if equals_ci(name, b"thread-wpan") {
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
        assert_eq!(is_never_hide(b"v4-rmnet0"), true, "is_never_hide('v4-rmnet0')");
        assert_eq!(is_never_hide(b"v4-rmnet_data0"), true, "is_never_hide('v4-rmnet_data0')");
        assert_eq!(is_never_hide(b"v4-wlan0"), true, "is_never_hide('v4-wlan0')");
        assert_eq!(is_never_hide(b"v4-x"), true, "is_never_hide('v4-x')");
        assert_eq!(is_never_hide(b"thread-wpan"), true, "is_never_hide('thread-wpan')");
        assert_eq!(is_never_hide(b"Thread-Wpan"), true, "is_never_hide('Thread-Wpan')");
        assert_eq!(is_never_hide(b"v4-"), false, "is_never_hide('v4-')");
        assert_eq!(is_never_hide(b"v4"), false, "is_never_hide('v4')");
        assert_eq!(is_never_hide(b"tun0"), false, "is_never_hide('tun0')");
        assert_eq!(is_never_hide(b"wg0"), false, "is_never_hide('wg0')");
        assert_eq!(is_never_hide(b"wlan0"), false, "is_never_hide('wlan0')");
        assert_eq!(is_never_hide(b"thread-wpan-extra"), false, "is_never_hide('thread-wpan-extra')");
        assert_eq!(is_never_hide(b"if33"), false, "is_never_hide('if33')");
        assert_eq!(is_never_hide(b""), false, "is_never_hide('')");
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
