@file:Suppress("DEPRECATION")

package dev.okhsunrog.vpnhide.hook

import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.os.Bundle
import de.robv.android.xposed.XC_MethodHook
import de.robv.android.xposed.XposedBridge
import de.robv.android.xposed.XposedHelpers
import java.util.WeakHashMap

/** Compare CS-built, recipient-redacted payloads, never raw VPN properties. */
internal class CallbackPayloadFilter {
    private val histories = WeakHashMap<Any, CallbackPayloadHistory>()
    private val available = type("CALLBACK_AVAILABLE")
    private val capabilities = type("CALLBACK_CAP_CHANGED")
    private val properties = type("CALLBACK_IP_CHANGED")
    private val blocked = type("CALLBACK_BLK_CHANGED")

    fun forget(registration: Any) {
        synchronized(histories) { histories.remove(registration) }
    }

    fun filter(
        param: XC_MethodHook.MethodHookParam,
        bundle: Bundle,
    ) {
        val network = bundle.getParcelable<Network>(Network::class.java.simpleName) ?: return
        val event = param.args[1] as Int
        if (event !in setOf(available, capabilities, properties, blocked)) return
        val values = payloads(bundle, param.args.last() as Int)
        val (initial, changed) =
            synchronized(histories) {
                val history = histories.getOrPut(param.args[0]) { CallbackPayloadHistory() }
                val initial = history.available(network.toString().toInt())
                val relevant = if (event == available) values else values.filterKeys { it == event }
                initial to relevant.filter { (kind, value) -> history.changed(kind, value) }
            }
        if (event == available && !initial) {
            // AVAILABLE includes NC/LP. Preserve changed properties without announcing
            // the same network for a second time.
            param.result = null
            changed.keys.forEach { kind ->
                XposedBridge.invokeOriginalMethod(param.method, param.thisObject, arrayOf(param.args[0], kind, bundle, param.args.last()))
            }
        } else if (event in values && event !in changed) {
            param.result = null
        }
    }

    private fun payloads(
        bundle: Bundle,
        blockedReason: Int,
    ): Map<Int, Any> =
        buildMap {
            bundle.getParcelable<NetworkCapabilities>(NetworkCapabilities::class.java.simpleName)?.let {
                put(capabilities, NetworkCapabilities(it))
            }
            bundle.getParcelable<LinkProperties>(LinkProperties::class.java.simpleName)?.let {
                put(properties, XposedHelpers.newInstance(LinkProperties::class.java, it))
            }
            if (blocked >= 0) put(blocked, blockedReason)
        }

    private fun type(name: String): Int =
        runCatching { XposedHelpers.getStaticIntField(ConnectivityManager::class.java, name) }.getOrDefault(-1)
}
