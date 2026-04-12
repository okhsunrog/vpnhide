//! Pure logic: is a given interface name a VPN tunnel?
//!
//! Kept as a leaf module with no dependencies so it's easy to unit-test on
//! the host. The set of prefixes must stay in sync with the Kotlin LSPosed
//! module's `isVpnInterfaceName`.

use core::ffi::CStr;

/// Interface name prefixes we treat as "this is a VPN tunnel". Any name
/// starting with one of these, or containing the substring `"vpn"`, is
/// hidden from the target app.
const VPN_PREFIXES: &[&[u8]] = &[
    b"tun",   // OpenVPN / WireGuard user-space / Tailscale / most tunneling
    b"ppp",   // PPTP / L2TP PPP tunnels
    b"tap",   // OpenVPN bridged mode
    b"wg",    // in-kernel WireGuard
    b"ipsec", // Android built-in IPsec VPN
    b"xfrm",  // kernel IPsec XFRM framework interfaces
    b"utun",  // Apple-style, rare on Android
    b"l2tp",  // L2TP
    b"gre",   // GRE tunnels
];

const VPN_SUBSTRING: &[u8] = b"vpn";

/// True if the bytes look like a VPN tunnel interface name.
///
/// Case-insensitive, works on raw `&[u8]` so we can call it straight from
/// a `libc::ifreq.ifr_name` buffer (which is `[c_char; IFNAMSIZ]`) without
/// having to copy into a String.
pub fn is_vpn_iface_bytes(name: &[u8]) -> bool {
    // Trim at the first NUL — ifr_name is a fixed-size buffer with a NUL
    // terminator somewhere inside it.
    let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
    let name = &name[..end];
    if name.is_empty() {
        return false;
    }

    // Ascii-lowercase compare for the prefix check
    let matches_prefix = VPN_PREFIXES.iter().any(|prefix| {
        name.len() >= prefix.len()
            && name[..prefix.len()]
                .iter()
                .zip(prefix.iter())
                .all(|(a, b)| a.to_ascii_lowercase() == *b)
    });
    if matches_prefix {
        return true;
    }

    // Substring check: "vpn" anywhere in the name
    contains_ignore_ascii_case(name, VPN_SUBSTRING)
}

/// Convenience wrapper: takes a `CStr` and dispatches to `is_vpn_iface_bytes`.
#[allow(dead_code)]
pub fn is_vpn_iface_cstr(name: &CStr) -> bool {
    is_vpn_iface_bytes(name.to_bytes())
}

/// Case-insensitive substring search for ASCII. Avoids pulling in regex or
/// heavy helpers so the library stays tiny.
fn contains_ignore_ascii_case(haystack: &[u8], needle: &[u8]) -> bool {
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

/// Filter `/proc/net/route` content in-place, removing lines whose
/// first tab-separated field is a VPN interface name.
/// Returns the new length of the valid data in `data`.
///
/// Format:
/// ```text
/// Iface   Destination Gateway     Flags RefCnt Use Metric Mask     MTU Window IRTT
/// wlan0   00000000    0101A8C0    0003  0      0   0      00000000 0   0      0
/// tun0    00000000    010010AC    0003  0      0   0      00000000 0   0      0
/// ```
/// The header line (starting with "Iface") is always kept.
pub fn filter_route_buf(data: &mut [u8]) -> usize {
    if data.is_empty() {
        return 0;
    }

    let len = data.len();
    let mut read_pos = 0usize;
    let mut write_pos = 0usize;

    while read_pos < len {
        // Find end of current line (including the '\n').
        let line_end = data[read_pos..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|p| read_pos + p + 1)
            .unwrap_or(len);

        // Extract first field (up to '\t').
        let line = &data[read_pos..line_end];
        let field_len = line
            .iter()
            .position(|&b| b == b'\t' || b == b'\n')
            .unwrap_or(line.len());
        let ifname = &line[..field_len];

        let hide = !ifname.is_empty() && is_vpn_iface_bytes(ifname);

        if !hide {
            let line_len = line_end - read_pos;
            if write_pos != read_pos {
                data.copy_within(read_pos..line_end, write_pos);
            }
            write_pos += line_len;
        }

        read_pos = line_end;
    }

    write_pos
}

/// Filter `/proc/net/ipv6_route` in-place. Interface name is the LAST
/// whitespace-delimited field on each line.
pub fn filter_ipv6_route_buf(data: &mut [u8]) -> usize {
    filter_by_last_field(data)
}

/// Filter `/proc/net/if_inet6` in-place. Interface name is the LAST
/// whitespace-delimited field on each line.
pub fn filter_if_inet6_buf(data: &mut [u8]) -> usize {
    filter_by_last_field(data)
}

/// Shared logic: filter lines where the LAST whitespace-delimited field
/// is a VPN interface name (used by ipv6_route and if_inet6).
fn filter_by_last_field(data: &mut [u8]) -> usize {
    if data.is_empty() {
        return 0;
    }

    let len = data.len();
    let mut read_pos = 0usize;
    let mut write_pos = 0usize;

    while read_pos < len {
        let line_end = data[read_pos..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|p| read_pos + p + 1)
            .unwrap_or(len);

        let line = &data[read_pos..line_end];
        let ifname = extract_last_field(line);
        let hide = !ifname.is_empty() && is_vpn_iface_bytes(ifname);

        if !hide {
            let line_len = line_end - read_pos;
            if write_pos != read_pos {
                data.copy_within(read_pos..line_end, write_pos);
            }
            write_pos += line_len;
        }

        read_pos = line_end;
    }

    write_pos
}

/// Extract the last whitespace-delimited field from a line (trimming
/// trailing newline/spaces).
fn extract_last_field(line: &[u8]) -> &[u8] {
    let mut end = line.len();
    while end > 0 && matches!(line[end - 1], b'\n' | b' ' | b'\t') {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && !matches!(line[start - 1], b' ' | b'\t') {
        start -= 1;
    }
    &line[start..end]
}

/// Maximum number of VPN addresses to track for tcp/tcp6 filtering.
pub const MAX_VPN_ADDRS: usize = 16;

/// Filter `/proc/net/tcp` in-place. Removes lines whose local address
/// (8-char hex after ": ") matches any of the given VPN IPv4 addresses.
///
/// `vpn_addrs` contains raw `sin_addr.s_addr` values (__be32) which
/// match the hex format in /proc/net/tcp directly.
pub fn filter_tcp4_buf(data: &mut [u8], vpn_addrs: &[u32], n_addrs: usize) -> usize {
    if data.is_empty() || n_addrs == 0 {
        return data.len();
    }
    filter_tcp_generic(data, &vpn_addrs[..n_addrs], 8, parse_hex_u32)
}

/// Filter `/proc/net/tcp6` in-place. Removes lines whose local address
/// (32-char hex after ": ") matches any of the given VPN IPv6 addresses.
///
/// `vpn_addrs` contains raw `s6_addr32` as 4×u32 in native byte order.
pub fn filter_tcp6_buf(data: &mut [u8], vpn_addrs: &[[u32; 4]], n_addrs: usize) -> usize {
    if data.is_empty() || n_addrs == 0 {
        return data.len();
    }
    filter_tcp6_inner(data, &vpn_addrs[..n_addrs])
}

/// Generic TCP filter: for each line, find ": ", parse `hex_len` hex
/// chars as an address, check against `vpn_addrs`.
fn filter_tcp_generic(
    data: &mut [u8],
    vpn_addrs: &[u32],
    hex_len: usize,
    parse: fn(&[u8]) -> Option<u32>,
) -> usize {
    let len = data.len();
    let mut read_pos = 0usize;
    let mut write_pos = 0usize;

    while read_pos < len {
        let line_end = data[read_pos..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|p| read_pos + p + 1)
            .unwrap_or(len);

        let line = &data[read_pos..line_end];
        let mut hide = false;

        // Find ": " separator, then parse hex address after it.
        if let Some(colon_pos) = find_colon_space(line) {
            let addr_start = colon_pos + 2;
            if addr_start + hex_len <= line.len() {
                if let Some(addr) = parse(&line[addr_start..addr_start + hex_len]) {
                    hide = vpn_addrs.contains(&addr);
                }
            }
        }

        if !hide {
            let line_len = line_end - read_pos;
            if write_pos != read_pos {
                data.copy_within(read_pos..line_end, write_pos);
            }
            write_pos += line_len;
        }

        read_pos = line_end;
    }

    write_pos
}

/// TCP6 filter: parse 32-char hex as 4×u32 and compare against VPN addrs.
fn filter_tcp6_inner(data: &mut [u8], vpn_addrs: &[[u32; 4]]) -> usize {
    let len = data.len();
    let mut read_pos = 0usize;
    let mut write_pos = 0usize;

    while read_pos < len {
        let line_end = data[read_pos..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|p| read_pos + p + 1)
            .unwrap_or(len);

        let line = &data[read_pos..line_end];
        let mut hide = false;

        if let Some(colon_pos) = find_colon_space(line) {
            let addr_start = colon_pos + 2;
            if addr_start + 32 <= line.len() {
                if let Some(addr) = parse_hex_addr6(&line[addr_start..addr_start + 32]) {
                    hide = vpn_addrs.contains(&addr);
                }
            }
        }

        if !hide {
            let line_len = line_end - read_pos;
            if write_pos != read_pos {
                data.copy_within(read_pos..line_end, write_pos);
            }
            write_pos += line_len;
        }

        read_pos = line_end;
    }

    write_pos
}

fn find_colon_space(line: &[u8]) -> Option<usize> {
    line.windows(2).position(|w| w == b": ")
}

fn parse_hex_u32(hex: &[u8]) -> Option<u32> {
    let mut val = 0u32;
    for &b in hex {
        let digit = match b {
            b'0'..=b'9' => b - b'0',
            b'A'..=b'F' => b - b'A' + 10,
            b'a'..=b'f' => b - b'a' + 10,
            _ => return None,
        };
        val = val.checked_shl(4)? | digit as u32;
    }
    Some(val)
}

fn parse_hex_addr6(hex: &[u8]) -> Option<[u32; 4]> {
    if hex.len() != 32 {
        return None;
    }
    Some([
        parse_hex_u32(&hex[0..8])?,
        parse_hex_u32(&hex[8..16])?,
        parse_hex_u32(&hex[16..24])?,
        parse_hex_u32(&hex[24..32])?,
    ])
}

// ============================================================================
//  Netlink RTM_NEWADDR / RTM_NEWLINK filter
// ============================================================================

const NLMSG_ALIGNTO: usize = 4;
const NLMSG_HDRLEN: usize = 16; // sizeof(struct nlmsghdr), already aligned
pub(crate) const RTM_NEWLINK: u16 = 16;
pub(crate) const RTM_NEWADDR: u16 = 20;

const fn nlmsg_align(len: usize) -> usize {
    (len + NLMSG_ALIGNTO - 1) & !(NLMSG_ALIGNTO - 1)
}

fn read_u32_ne(data: &[u8], off: usize) -> Option<u32> {
    let bytes: &[u8; 4] = data.get(off..off + 4)?.try_into().ok()?;
    Some(u32::from_ne_bytes(*bytes))
}

fn read_u16_ne(data: &[u8], off: usize) -> Option<u16> {
    let bytes: &[u8; 2] = data.get(off..off + 2)?.try_into().ok()?;
    Some(u16::from_ne_bytes(*bytes))
}

/// Filter netlink dump responses in-place: remove `RTM_NEWLINK` and
/// `RTM_NEWADDR` messages whose interface index is in `vpn_indices`.
///
/// Both `struct ifinfomsg` (RTM_NEWLINK) and `struct ifaddrmsg`
/// (RTM_NEWADDR) have the interface index as a `u32` at offset 4
/// within the payload, so the same extraction works for both.
///
/// Returns the new valid length of the buffer.
pub fn filter_netlink_dump(data: &mut [u8], vpn_indices: &[u32]) -> usize {
    if vpn_indices.is_empty() || data.len() < NLMSG_HDRLEN {
        return data.len();
    }

    let len = data.len();
    let mut read_pos = 0usize;
    let mut write_pos = 0usize;

    while read_pos + NLMSG_HDRLEN <= len {
        let Some(nlmsg_len_raw) = read_u32_ne(data, read_pos) else {
            break;
        };
        let nlmsg_len = nlmsg_len_raw as usize;
        if nlmsg_len < NLMSG_HDRLEN || read_pos + nlmsg_len > len {
            break;
        }
        let aligned_len = nlmsg_align(nlmsg_len).min(len - read_pos);
        let Some(nlmsg_type) = read_u16_ne(data, read_pos + 4) else {
            break;
        };

        let hide = if (nlmsg_type == RTM_NEWLINK || nlmsg_type == RTM_NEWADDR)
            && nlmsg_len >= NLMSG_HDRLEN + 8
        {
            // Interface index is at payload offset 4 in both
            // ifinfomsg and ifaddrmsg.
            let if_index = read_u32_ne(data, read_pos + NLMSG_HDRLEN + 4).unwrap_or(0);
            vpn_indices.contains(&if_index)
        } else {
            false
        };

        if !hide {
            if write_pos != read_pos {
                data.copy_within(read_pos..read_pos + aligned_len, write_pos);
            }
            write_pos += aligned_len;
        }

        read_pos += aligned_len;
    }

    // Trailing bytes (shouldn't happen in well-formed netlink).
    if read_pos < len {
        let tail = len - read_pos;
        if write_pos != read_pos {
            data.copy_within(read_pos..len, write_pos);
        }
        write_pos += tail;
    }

    write_pos
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_tun0() {
        assert!(is_vpn_iface_bytes(b"tun0"));
        assert!(is_vpn_iface_bytes(b"tun1"));
        assert!(is_vpn_iface_bytes(b"TUN0"));
    }

    #[test]
    fn detects_wireguard() {
        assert!(is_vpn_iface_bytes(b"wg0"));
        assert!(is_vpn_iface_bytes(b"wg-client"));
    }

    #[test]
    fn detects_ppp_and_l2tp() {
        assert!(is_vpn_iface_bytes(b"ppp0"));
        assert!(is_vpn_iface_bytes(b"l2tp0"));
    }

    #[test]
    fn detects_vpn_substring() {
        assert!(is_vpn_iface_bytes(b"my-vpn-iface"));
        assert!(is_vpn_iface_bytes(b"custom_VPN_42"));
    }

    #[test]
    fn rejects_real_interfaces() {
        assert!(!is_vpn_iface_bytes(b"lo"));
        assert!(!is_vpn_iface_bytes(b"wlan0"));
        assert!(!is_vpn_iface_bytes(b"rmnet16"));
        assert!(!is_vpn_iface_bytes(b"eth0"));
        assert!(!is_vpn_iface_bytes(b"dummy0"));
    }

    #[test]
    fn handles_embedded_nul_from_ifreq() {
        // IFNAMSIZ is 16 — simulate a kernel-filled ifr_name buffer
        let mut buf = [0u8; 16];
        buf[..4].copy_from_slice(b"tun0");
        assert!(is_vpn_iface_bytes(&buf));

        buf.fill(0);
        buf[..5].copy_from_slice(b"wlan0");
        assert!(!is_vpn_iface_bytes(&buf));
    }

    #[test]
    fn empty_name_is_not_vpn() {
        assert!(!is_vpn_iface_bytes(b""));
        assert!(!is_vpn_iface_bytes(&[0u8; 16]));
    }

    #[test]
    fn filter_route_removes_vpn_lines() {
        let input = b"Iface\tDestination\tGateway\n\
                       wlan0\t00000000\t0101A8C0\n\
                       tun0\t00000000\t010010AC\n\
                       rmnet0\tFEFFFFFF\t00000000\n";
        let mut buf = input.to_vec();
        let new_len = filter_route_buf(&mut buf);
        let result = core::str::from_utf8(&buf[..new_len]).unwrap();
        assert!(result.contains("Iface\t"));
        assert!(result.contains("wlan0\t"));
        assert!(result.contains("rmnet0\t"));
        assert!(!result.contains("tun0"));
    }

    #[test]
    fn filter_route_keeps_all_when_no_vpn() {
        let input = b"Iface\tDestination\n\
                       wlan0\t00000000\n\
                       rmnet0\tFEFFFFFF\n";
        let mut buf = input.to_vec();
        let new_len = filter_route_buf(&mut buf);
        assert_eq!(new_len, input.len());
    }

    #[test]
    fn filter_route_removes_wg_lines() {
        let input = b"Iface\tDest\nwg0\t00000000\nwlan0\t00000000\n";
        let mut buf = input.to_vec();
        let new_len = filter_route_buf(&mut buf);
        let result = core::str::from_utf8(&buf[..new_len]).unwrap();
        assert!(!result.contains("wg0"));
        assert!(result.contains("wlan0"));
    }

    #[test]
    fn filter_route_empty_input() {
        let mut buf = [];
        assert_eq!(filter_route_buf(&mut buf), 0);
    }

    // ---- Netlink filter tests ----

    /// Build a minimal nlmsghdr + ifaddrmsg/ifinfomsg for testing.
    /// `msg_type` is RTM_NEWADDR (20) or RTM_NEWLINK (16).
    /// `if_index` is the interface index placed at payload offset 4.
    fn make_nlmsg(msg_type: u16, if_index: u32) -> Vec<u8> {
        // nlmsghdr (16 bytes) + 8 bytes payload (family/pad/type + index)
        let total_len: u32 = 24;
        let mut msg = Vec::new();
        msg.extend_from_slice(&total_len.to_ne_bytes()); // nlmsg_len
        msg.extend_from_slice(&msg_type.to_ne_bytes()); // nlmsg_type
        msg.extend_from_slice(&0u16.to_ne_bytes()); // nlmsg_flags
        msg.extend_from_slice(&1u32.to_ne_bytes()); // nlmsg_seq
        msg.extend_from_slice(&0u32.to_ne_bytes()); // nlmsg_pid
        // payload: 4 bytes (family etc) + 4 bytes (if_index)
        msg.extend_from_slice(&[0u8; 4]);
        msg.extend_from_slice(&if_index.to_ne_bytes());
        msg
    }

    #[test]
    fn netlink_filter_removes_vpn_newaddr() {
        let vpn_idx: u32 = 7; // tun0
        let wlan_idx: u32 = 2;

        let mut buf = Vec::new();
        buf.extend(make_nlmsg(RTM_NEWADDR, wlan_idx));
        buf.extend(make_nlmsg(RTM_NEWADDR, vpn_idx));
        buf.extend(make_nlmsg(RTM_NEWADDR, wlan_idx));

        let orig_msgs = 3;
        let new_len = filter_netlink_dump(&mut buf, &[vpn_idx]);

        // Should have removed exactly the vpn_idx message (24 bytes).
        assert_eq!(new_len, 24 * (orig_msgs - 1));
        // First remaining msg should be wlan_idx.
        assert_eq!(read_u32_ne(&buf, NLMSG_HDRLEN + 4), Some(wlan_idx));
        // Second remaining msg should also be wlan_idx.
        assert_eq!(read_u32_ne(&buf, 24 + NLMSG_HDRLEN + 4), Some(wlan_idx));
    }

    #[test]
    fn netlink_filter_removes_vpn_newlink() {
        let vpn_idx: u32 = 5;
        let lo_idx: u32 = 1;

        let mut buf = Vec::new();
        buf.extend(make_nlmsg(RTM_NEWLINK, vpn_idx));
        buf.extend(make_nlmsg(RTM_NEWLINK, lo_idx));

        let new_len = filter_netlink_dump(&mut buf, &[vpn_idx]);
        assert_eq!(new_len, 24); // only lo remains
        assert_eq!(read_u16_ne(&buf, 4), Some(RTM_NEWLINK));
        assert_eq!(read_u32_ne(&buf, NLMSG_HDRLEN + 4), Some(lo_idx));
    }

    #[test]
    fn netlink_filter_keeps_all_no_match() {
        let mut buf = Vec::new();
        buf.extend(make_nlmsg(RTM_NEWADDR, 1));
        buf.extend(make_nlmsg(RTM_NEWADDR, 2));
        let orig_len = buf.len();

        let new_len = filter_netlink_dump(&mut buf, &[99]);
        assert_eq!(new_len, orig_len);
    }

    #[test]
    fn netlink_filter_removes_all() {
        let mut buf = Vec::new();
        buf.extend(make_nlmsg(RTM_NEWADDR, 7));
        buf.extend(make_nlmsg(RTM_NEWADDR, 7));

        let new_len = filter_netlink_dump(&mut buf, &[7]);
        assert_eq!(new_len, 0);
    }

    #[test]
    fn netlink_filter_preserves_non_newaddr_msgs() {
        let nlmsg_done_type: u16 = 3; // NLMSG_DONE
        let mut buf = Vec::new();
        buf.extend(make_nlmsg(RTM_NEWADDR, 7)); // VPN — remove
        buf.extend(make_nlmsg(nlmsg_done_type, 0)); // DONE — keep
        buf.extend(make_nlmsg(RTM_NEWADDR, 2)); // wlan — keep

        let new_len = filter_netlink_dump(&mut buf, &[7]);
        // Should keep DONE + wlan = 48 bytes
        assert_eq!(new_len, 48);
        assert_eq!(read_u16_ne(&buf, 4), Some(nlmsg_done_type));
        assert_eq!(read_u16_ne(&buf, 24 + 4), Some(RTM_NEWADDR));
        assert_eq!(read_u32_ne(&buf, 24 + NLMSG_HDRLEN + 4), Some(2));
    }

    #[test]
    fn netlink_filter_empty_indices() {
        let mut buf = make_nlmsg(RTM_NEWADDR, 7);
        let orig_len = buf.len();
        let new_len = filter_netlink_dump(&mut buf, &[]);
        assert_eq!(new_len, orig_len);
    }
}
