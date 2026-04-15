package dev.okhsunrog.vpnhide

import android.net.LinkProperties
import android.net.NetworkCapabilities
import android.net.NetworkInfo
import android.net.RouteInfo
import de.robv.android.xposed.XposedBridge
import de.robv.android.xposed.XposedHelpers

/**
 * Pure functions that produce a "VPN-free" view of the three network
 * state objects we filter. Each returns a new object with VPN details
 * stripped, or `null` if the input did not contain any VPN signal (so
 * callers can skip the work and pass the original through).
 *
 * Used by both the legacy `writeToParcel` hooks in `HookEntry` and the
 * new server-side `ConnectivityService` getter hooks. Neither caller
 * ever mutates the input object — all changes land on the returned
 * copy, which keeps broadcast/callback paths that share the original
 * safe.
 */
internal object NetworkSanitizers {
    const val TRANSPORT_VPN = 4
    const val NET_CAPABILITY_NOT_VPN = 15
    const val TYPE_VPN = 17
    const val TYPE_WIFI = 1

    fun isVpnInterfaceName(name: String): Boolean {
        if (name.isEmpty()) return false
        val n = name.lowercase()
        return n.startsWith("tun") ||
            n.startsWith("ppp") ||
            n.startsWith("tap") ||
            n.startsWith("wg") ||
            n.startsWith("ipsec") ||
            n.startsWith("xfrm") ||
            n.startsWith("utun") ||
            n.startsWith("l2tp") ||
            n.startsWith("gre") ||
            n.contains("vpn")
    }

    /**
     * Returns a NetworkCapabilities copy with TRANSPORT_VPN cleared,
     * NOT_VPN capability set, and VpnTransportInfo removed. Returns
     * null if the input has no VPN transport (no work to do).
     */
    fun sanitizeNetworkCapabilities(nc: NetworkCapabilities): NetworkCapabilities? {
        val transportTypes = XposedHelpers.getLongField(nc, "mTransportTypes")
        val vpnBit = 1L shl TRANSPORT_VPN
        if (transportTypes and vpnBit == 0L) return null

        val copy = NetworkCapabilities(nc)
        XposedHelpers.setLongField(copy, "mTransportTypes", transportTypes and vpnBit.inv())
        val caps = XposedHelpers.getLongField(copy, "mNetworkCapabilities")
        XposedHelpers.setLongField(copy, "mNetworkCapabilities", caps or (1L shl NET_CAPABILITY_NOT_VPN))
        try {
            val ti = XposedHelpers.getObjectField(copy, "mTransportInfo")
            if (ti != null && ti.javaClass.name == "android.net.VpnTransportInfo") {
                XposedHelpers.setObjectField(copy, "mTransportInfo", null)
            }
        } catch (_: Throwable) {
            // mTransportInfo doesn't exist on older Android — ignore
        }
        return copy
    }

    /**
     * Returns a WIFI-typed NetworkInfo copy preserving the original
     * state. Returns null if the input isn't a VPN NetworkInfo.
     */
    @Suppress("DEPRECATION")
    fun sanitizeNetworkInfo(ni: NetworkInfo): NetworkInfo? {
        val type = XposedHelpers.getIntField(ni, "mNetworkType")
        if (type != TYPE_VPN) return null

        val ctor =
            NetworkInfo::class.java.getDeclaredConstructor(
                Integer.TYPE,
                Integer.TYPE,
                String::class.java,
                String::class.java,
            )
        ctor.isAccessible = true
        val copy = ctor.newInstance(TYPE_WIFI, 0, "WIFI", "") as NetworkInfo
        XposedHelpers.setIntField(copy, "mState", XposedHelpers.getIntField(ni, "mState"))
        XposedHelpers.setIntField(copy, "mDetailedState", XposedHelpers.getIntField(ni, "mDetailedState"))
        XposedHelpers.setBooleanField(copy, "mIsAvailable", XposedHelpers.getBooleanField(ni, "mIsAvailable"))
        return copy
    }

    /**
     * Returns a LinkProperties copy with VPN interface name and
     * VPN-bound routes stripped (recursively across `mStackedLinks`).
     * Returns null if no VPN signal was present (no copy needed).
     */
    fun sanitizeLinkProperties(lp: LinkProperties): LinkProperties? {
        val ctor = LinkProperties::class.java.getDeclaredConstructor(LinkProperties::class.java)
        ctor.isAccessible = true
        val copy = ctor.newInstance(lp) as LinkProperties
        return if (sanitizeInPlace(copy)) copy else null
    }

    /**
     * Mutates `copy` in place to strip VPN data. Returns true if any
     * change was made. Recurses through `mStackedLinks`.
     *
     * Separate from `sanitizeLinkProperties` so the recursive path
     * can reuse the mutation without allocating an outer copy.
     */
    private fun sanitizeInPlace(copy: LinkProperties): Boolean {
        var modified = false

        val ifaceName = XposedHelpers.getObjectField(copy, "mIfaceName") as? String
        if (ifaceName != null && isVpnInterfaceName(ifaceName)) {
            XposedHelpers.setObjectField(copy, "mIfaceName", null)
            modified = true
        }

        try {
            @Suppress("UNCHECKED_CAST")
            val routesField = XposedHelpers.getObjectField(copy, "mRoutes") as? MutableList<RouteInfo>
            if (routesField != null) {
                val filtered =
                    routesField.filterNot { route ->
                        val routeIface = route.`interface`
                        routeIface != null && isVpnInterfaceName(routeIface)
                    }
                if (filtered.size != routesField.size) {
                    routesField.clear()
                    routesField.addAll(filtered)
                    modified = true
                }
            }
        } catch (t: Throwable) {
            XposedBridge.log("VpnHide: failed to sanitize mRoutes: ${t.message}")
        }

        try {
            @Suppress("UNCHECKED_CAST")
            val stacked = XposedHelpers.getObjectField(copy, "mStackedLinks") as? MutableMap<String, LinkProperties>
            if (stacked != null && stacked.isNotEmpty()) {
                val filtered = LinkedHashMap<String, LinkProperties>()
                for ((key, value) in stacked) {
                    val stackedCopy =
                        try {
                            val ctor = LinkProperties::class.java.getDeclaredConstructor(LinkProperties::class.java)
                            ctor.isAccessible = true
                            ctor.newInstance(value) as LinkProperties
                        } catch (_: Throwable) {
                            value
                        }
                    val stackedModified = sanitizeInPlace(stackedCopy)
                    val stackedIface = XposedHelpers.getObjectField(stackedCopy, "mIfaceName") as? String
                    if (stackedIface == null && stackedCopy.routes.isEmpty()) {
                        if (stackedModified || isVpnInterfaceName(key)) {
                            modified = true
                        } else {
                            filtered[key] = stackedCopy
                        }
                    } else {
                        if (stackedModified || stackedCopy !== value) modified = true
                        filtered[key] = stackedCopy
                    }
                }
                if (filtered.size != stacked.size || modified) {
                    stacked.clear()
                    stacked.putAll(filtered)
                }
            }
        } catch (t: Throwable) {
            XposedBridge.log("VpnHide: failed to sanitize mStackedLinks: ${t.message}")
        }

        return modified
    }
}
