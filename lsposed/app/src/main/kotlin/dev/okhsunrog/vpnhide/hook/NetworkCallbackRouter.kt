// The legacy Bundle accessor is required on Android 9–12.
@file:Suppress("DEPRECATION")

package dev.okhsunrog.vpnhide.hook

import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkRequest
import android.os.Bundle
import android.os.Message
import android.os.Messenger
import de.robv.android.xposed.XC_MethodHook
import de.robv.android.xposed.XposedBridge
import de.robv.android.xposed.XposedHelpers
import dev.okhsunrog.vpnhide.LsposedStats
import dev.okhsunrog.vpnhide.generated.HookIds
import java.util.WeakHashMap

/**
 * New frameworks queue callbacks before the Bundle overload: rewrite only at
 * delivery, leaving frozen-process queues in the platform's original identities.
 * Older frameworks have one dispatcher, so it is the delivery boundary instead.
 * Weak registration keys expire when CS releases an NRI; values hold no NRI refs.
 */
internal class NetworkCallbackRouter(
    serviceClass: Class<*>,
    private val isVpn: (Any, Network) -> Boolean,
    private val coverFor: (Any, Int) -> Network?,
    private val isListen: (NetworkRequest?) -> Boolean,
) {
    private val states = WeakHashMap<Any, Int>()
    private val forwarding = ThreadLocal<Boolean>()
    private val payloads = CallbackPayloadFilter()
    private val builder =
        serviceClass.declaredMethods
            .firstOrNull {
                it.name == "callCallbackForRequest" && it.parameterTypes.size == 4 &&
                    it.parameterTypes[1].simpleName == "NetworkAgentInfo"
            }?.apply { isAccessible = true }
    private val hasBundleDispatcher =
        serviceClass.declaredMethods.any {
            it.name == "callCallbackForRequest" && Bundle::class.java in it.parameterTypes
        }
    private val available = callbackType("CALLBACK_AVAILABLE")
    private val lost = callbackType("CALLBACK_LOST")
    private val losing = callbackType("CALLBACK_LOSING")
    private val blocked = callbackType("CALLBACK_BLK_CHANGED")

    // True means this router owns the callback; false leaves PendingIntents to their adapter.
    fun dispatch(
        param: XC_MethodHook.MethodHookParam,
        uid: Int,
        request: NetworkRequest?,
    ): Boolean {
        if (param.method.name != "callCallbackForRequest") return false
        val bundle = param.args.getOrNull(2) as? Bundle
        if (forwarding.get() == true) {
            if (bundle != null) payloads.filter(param, bundle)
            return true
        }
        if (hasBundleDispatcher && bundle == null) return true
        val nri = param.args[0]
        val source =
            if (bundle != null) {
                bundle.getParcelable<Network>(Network::class.java.simpleName)
            } else {
                param.args.getOrNull(1)?.let { XposedHelpers.getObjectField(it, "network") as? Network }
            } ?: return true // UNAVAILABLE / RESERVED have no network.
        val vpn = isVpn(param.thisObject, source)
        if (isListen(request)) {
            if (vpn) param.result = null
            return true
        }
        val type = (if (bundle != null) param.args[1] else param.args[2]) as Int
        val held = synchronized(states) { states[nri] }
        val event = eventKind(type)
        if (!vpn) {
            val next = transitionPhysicalCallback(held, netId(source), event)
            filterPhysical(param, next, type, bundle)
            remember(nri, next)
            return true
        }
        LsposedStats.record(uid, HookIds.Hook.LSPOSED_CONNECTIVITY_CALLBACK)
        val cover = coverFor(param.thisObject, uid)
        val transition = transitionCallback(held, cover?.let(::netId), event)
        // A duplicate AVAILABLE may still carry new properties; filter the rebuilt Bundle.
        val next =
            if (bundle != null && cover != null && type == available && transition.delivery == CallbackDelivery.Suppress) {
                transition.copy(delivery = CallbackDelivery.Forward)
            } else {
                transition
            }
        if (!deliver(param, request, next, cover, type, bundle)) return true
        remember(nri, next)
        return true
    }

    private fun filterPhysical(
        param: XC_MethodHook.MethodHookParam,
        next: CallbackTransition,
        type: Int,
        bundle: Bundle?,
    ) {
        if (next.delivery == CallbackDelivery.Suppress && !(type == available && bundle != null)) {
            param.result = null
        } else if (bundle != null) {
            payloads.filter(param, bundle)
        }
    }

    private fun eventKind(type: Int): CallbackEventKind =
        when (type) {
            available -> CallbackEventKind.Available
            lost -> CallbackEventKind.Lost
            losing -> CallbackEventKind.Losing
            else -> CallbackEventKind.Changed
        }

    private fun remember(
        nri: Any,
        next: CallbackTransition,
    ) {
        if (next.held == null) payloads.forget(nri)
        synchronized(states) {
            if (next.held == null) states.remove(nri) else states[nri] = next.held
        }
    }

    private fun deliver(
        param: XC_MethodHook.MethodHookParam,
        request: NetworkRequest?,
        next: CallbackTransition,
        cover: Network?,
        type: Int,
        bundle: Bundle?,
    ): Boolean {
        param.result = null
        return when (next.delivery) {
            CallbackDelivery.Suppress -> {
                true
            }

            CallbackDelivery.Lost -> {
                sendLost(param, request, next.network!!, bundle)
            }

            CallbackDelivery.Available, CallbackDelivery.Forward -> {
                val nai = XposedHelpers.callMethod(param.thisObject, "getNetworkAgentInfoForNetwork", cover) ?: return false
                val method = builder ?: error("No callback builder on ${param.thisObject.javaClass.name}")
                // Ask CS to construct/redact the entire event for the actual recipient.
                // A new cover needs AVAILABLE (with NC+LP), even if VPN only emitted CAP_CHANGED.
                val outputType = if (next.delivery == CallbackDelivery.Available) available else type
                val arg1 = if (outputType == blocked || outputType == available) 0 else param.args.last()
                forwarding.set(true)
                try {
                    XposedBridge.invokeOriginalMethod(method, param.thisObject, arrayOf(param.args[0], nai, outputType, arg1))
                } finally {
                    forwarding.remove()
                }
                true
            }
        }
    }

    private fun sendLost(
        param: XC_MethodHook.MethodHookParam,
        request: NetworkRequest?,
        held: Int,
        original: Bundle?,
    ): Boolean {
        val network = XposedHelpers.newInstance(Network::class.java, held) as Network
        val payload =
            Bundle().apply {
                putParcelable(Network::class.java.simpleName, network)
                val nr = original?.getParcelable<NetworkRequest>(NetworkRequest::class.java.simpleName) ?: request
                putParcelable(NetworkRequest::class.java.simpleName, nr)
            }
        if (original != null) {
            XposedBridge.invokeOriginalMethod(param.method, param.thisObject, arrayOf(param.args[0], lost, payload, 0))
        } else {
            // Pre-Bundle dispatchers build LOST using only NetworkRequest + Network.
            val nri = param.args[0]
            val messenger =
                listOf("mMessenger", "messenger").firstNotNullOfOrNull {
                    runCatching { XposedHelpers.getObjectField(nri, it) as? Messenger }.getOrNull()
                } ?: return false
            messenger.send(
                Message.obtain().apply {
                    what = lost
                    data = payload
                },
            )
        }
        return true
    }

    private fun callbackType(name: String): Int =
        runCatching { XposedHelpers.getStaticIntField(ConnectivityManager::class.java, name) }.getOrDefault(-1)

    private fun netId(network: Network): Int = network.toString().toInt()
}
