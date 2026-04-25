// AUTO-GENERATED from data/interfaces.toml — do not edit by hand. Regenerate with: python3 scripts/codegen-interfaces.py

package dev.okhsunrog.vpnhide.generated

internal object IfaceLists {
    /** True if `name` is in the never-hide whitelist per data/interfaces.toml. */
    fun isNeverHide(name: String): Boolean {
        if (name.isEmpty()) return false
        val n = name.lowercase()
        // 464XLAT CLAT shadow iface (v4-rmnet0, v4-wlan0, ...). Required on IPv6-only carriers (T-Mobile US, Reliance Jio, ...) — without it IPv4-only apps lose internet. Created by clatd, lives as ARPHRD_NONE TUN, easy to mistake for a VPN tunnel. AOSP source: external/android-clat.
        if (n.startsWith("v4-") && n.length > 3) return true
        // OpenThread border router on Pixel 7+. Hard-coded in init.rc inside the com.android.tethering APEX (the same APEX that delivers VPN-related code). Used for Matter / smart-home Thread mesh, not connectivity for normal apps.
        if (n == "thread-wpan") return true
        // IPv6-in-IPv4 tunnel placeholder (kmod: sit). ARPHRD_SIT=776.
        if (n == "sit0") return true
        // IPv4 IPIP tunnel placeholder (kmod: ipip). ARPHRD_TUNNEL=768.
        if (n == "tunl0") return true
        // IPv6 tunnel placeholder (kmod: ip6_tunnel). ARPHRD_TUNNEL6=769.
        if (n == "ip6tnl0") return true
        // IPv4 VTI (IPsec) placeholder (kmod: ip_vti). ARPHRD_TUNNEL=768.
        if (n == "ip_vti0") return true
        // IPv6 VTI (IPsec) placeholder (kmod: ip6_vti). ARPHRD_TUNNEL6=769.
        if (n == "ip6_vti0") return true
        // GRE tunnel placeholder (kmod: ip_gre). ARPHRD_IPGRE=778.
        if (n == "gre0") return true
        // Android system IPsec/XFRM placeholder. Created by the platform on stock Android (observed on Pixel 8 Pro / Android 16) as ARPHRD_NONE without a tun_flags attr — looks like a TUN VPN by ARPHRD alone, but is not. The numeric suffix is the system token; if vendor builds use a different one we'll add it explicitly rather than blanket-whitelisting ipsec* (which would let real IKEv2 VPNs created via IpSecTunnelInterface slip past).
        if (n == "ipsec250") return true
        return false
    }
}
