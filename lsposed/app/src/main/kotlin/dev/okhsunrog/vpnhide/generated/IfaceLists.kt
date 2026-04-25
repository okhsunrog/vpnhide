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
        return false
    }
}
