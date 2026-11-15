@file:Suppress("DEPRECATION")

package dev.okhsunrog.vpnhide.hook

import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.os.Bundle
import de.robv.android.xposed.XC_MethodHook
import de.robv.android.xposed.XposedHelpers
import java.lang.reflect.Method

/** Uses the platform's real callback registration and final Messenger dispatcher. */
internal class FrameworkCallbackFixture {
    private val available = callbackType("CALLBACK_AVAILABLE")
    private val lost = callbackType("CALLBACK_LOST")

    fun input(
        param: XC_MethodHook.MethodHookParam,
        worker: FixtureWorker,
    ): Input? {
        if (available < 0 || lost < 0 || param.method.name != "callCallbackForRequest" || param.args.getOrNull(1) != available) return null
        val bundle = param.args.getOrNull(2) as? Bundle ?: return null
        val request = bundle.getParcelable<NetworkRequest>(NetworkRequest::class.java.simpleName) ?: return null
        val nri = param.args[0] ?: return null
        if (field(nri, "mUid", "uid") != worker.uid || field(nri, "mPid", "pid") != worker.pid) return null
        val ordinary = bundle.getParcelable<Network>(Network::class.java.simpleName) ?: return null
        val capabilities = bundle.getParcelable<NetworkCapabilities>(NetworkCapabilities::class.java.simpleName) ?: return null
        if (capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN)) return null
        val registration = registration(runCatching { XposedHelpers.getObjectField(request, "type").toString() }.getOrNull()) ?: return null
        return Input(registration, ordinary, request)
    }

    fun dispatch(
        param: XC_MethodHook.MethodHookParam,
        input: Input,
        objects: FrameworkFixtureObjects,
        stillAuthorized: () -> Boolean,
    ) {
        val method = param.method as Method
        val baseline = param.args[2] as Bundle
        val vpnAvailable =
            Bundle(baseline).apply {
                putParcelable(Network::class.java.simpleName, objects.vpn)
                putParcelable(NetworkCapabilities::class.java.simpleName, objects.capabilities(objects.vpn))
                putParcelable(LinkProperties::class.java.simpleName, objects.properties(objects.vpn))
            }
        val vpnLost =
            Bundle().apply {
                putParcelable(NetworkRequest::class.java.simpleName, input.request)
                putParcelable(Network::class.java.simpleName, objects.vpn)
            }
        method.isAccessible = true
        if (stillAuthorized()) method.invoke(param.thisObject, param.args[0], available, vpnAvailable, param.args[3])
        if (stillAuthorized()) method.invoke(param.thisObject, param.args[0], lost, vpnLost, 0)
    }

    data class Input(
        val registration: FixtureCallbackRegistration,
        val ordinary: Network,
        val request: NetworkRequest,
    )

    private fun field(
        instance: Any,
        vararg names: String,
    ): Int? = names.firstNotNullOfOrNull { runCatching { XposedHelpers.getIntField(instance, it) }.getOrNull() }

    private fun registration(type: String?): FixtureCallbackRegistration? =
        when (type) {
            "LISTEN" -> FixtureCallbackRegistration.LISTEN
            "TRACK_DEFAULT" -> FixtureCallbackRegistration.DEFAULT
            "REQUEST" -> FixtureCallbackRegistration.REQUEST
            else -> null
        }

    private fun callbackType(name: String): Int =
        runCatching { XposedHelpers.getStaticIntField(ConnectivityManager::class.java, name) }.getOrDefault(-1)
}
