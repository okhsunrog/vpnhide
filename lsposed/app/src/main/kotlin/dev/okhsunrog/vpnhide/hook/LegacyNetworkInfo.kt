@file:Suppress("DEPRECATION")

package dev.okhsunrog.vpnhide.hook

import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.NetworkCapabilities
import android.net.NetworkInfo
import de.robv.android.xposed.XposedHelpers

internal fun isInactiveVpnInfo(info: NetworkInfo): Boolean =
    info.type == ConnectivityManager.TYPE_VPN &&
        (
            info.detailedState == NetworkInfo.DetailedState.DISCONNECTED ||
                info.detailedState == NetworkInfo.DetailedState.BLOCKED
        )

/**
 * Ask the platform to apply its no-network UID policy. Never clear Binder identity:
 * this runs after the public method's permission check, for the original caller.
 * AOSP 12+ has a factory; 9–11 filters an empty NetworkState instead.
 */
internal fun disconnectedVpnInfo(
    service: Any,
    uid: Int,
): NetworkInfo {
    val factory =
        XposedHelpers.findMethodExactIfExists(
            service.javaClass,
            "makeFakeNetworkInfo",
            Integer.TYPE,
            Integer.TYPE,
        )
    if (factory != null) {
        return factory.invoke(service, ConnectivityManager.TYPE_VPN, uid) as NetworkInfo
    }

    // These constructors/mutators exist on 9–11 but were only exposed in the
    // public SDK later. Resolve them inside system_server like the parcel hooks.
    val info = XposedHelpers.newInstance(NetworkInfo::class.java, ConnectivityManager.TYPE_VPN, 0, "VPN", "") as NetworkInfo
    XposedHelpers.callMethod(info, "setDetailedState", NetworkInfo.DetailedState.DISCONNECTED, null, null)
    XposedHelpers.setBooleanField(info, "mIsAvailable", true)
    val capabilities = XposedHelpers.newInstance(NetworkCapabilities::class.java)
    XposedHelpers.callMethod(capabilities, "addCapability", NetworkCapabilities.NET_CAPABILITY_NOT_ROAMING)
    val stateClass = XposedHelpers.findClass("android.net.NetworkState", service.javaClass.classLoader)
    val properties = XposedHelpers.newInstance(LinkProperties::class.java)
    val state = XposedHelpers.newInstance(stateClass, info, properties, capabilities, null, null, null)
    XposedHelpers.callMethod(service, "filterNetworkStateForUid", state, uid, false)
    return info
}
