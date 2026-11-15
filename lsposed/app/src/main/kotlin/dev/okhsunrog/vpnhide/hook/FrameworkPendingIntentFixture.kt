@file:Suppress("DEPRECATION")

package dev.okhsunrog.vpnhide.hook

import android.app.PendingIntent
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.os.Build
import de.robv.android.xposed.XC_MethodHook
import de.robv.android.xposed.XposedHelpers
import java.lang.reflect.Method

/** A detached agent supplies the VPN handle to the real one-shot platform dispatcher. */
internal class FrameworkPendingIntentFixture {
    private val available =
        runCatching {
            XposedHelpers.getStaticIntField(ConnectivityManager::class.java, "CALLBACK_AVAILABLE")
        }.getOrDefault(-1)

    fun input(
        param: XC_MethodHook.MethodHookParam,
        worker: FixtureWorker,
    ): Input? {
        if (available < 0 || param.args.size != 3 || param.args[2] != available) return null
        val requestInfo = param.args[0] ?: return null
        val agent = param.args[1] ?: return null
        if (!fixtureRequestOwnedBy(requestInfo, worker) || XposedHelpers.getBooleanField(requestInfo, "mPendingIntentSent")) return null
        val pending = XposedHelpers.getObjectField(requestInfo, "mPendingIntent") as? PendingIntent ?: return null
        if (pending.creatorUid != worker.uid) return null
        val request = XposedHelpers.getObjectField(requestInfo, "mNetworkRequestForCallback") as? NetworkRequest ?: return null
        val kind =
            when (XposedHelpers.getObjectField(request, "type").toString()) {
                "LISTEN" -> FixturePendingRegistration.LISTEN
                "REQUEST" -> FixturePendingRegistration.REQUEST
                else -> return null
            }
        val ordinary = XposedHelpers.getObjectField(agent, "network") as? Network ?: return null
        val capabilities = XposedHelpers.getObjectField(agent, "networkCapabilities") as? NetworkCapabilities ?: return null
        if (capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN)) return null
        return Input(kind, ordinary, agent)
    }

    fun dispatch(
        param: XC_MethodHook.MethodHookParam,
        agent: Any,
    ) {
        val method = param.method as Method
        method.isAccessible = true
        // RAW and protected REQUEST consume the same one-shot slot. Suppressing a
        // protected LISTEN leaves it free for the original ordinary dispatch.
        method.invoke(param.thisObject, param.args[0], agent, param.args[2])
    }

    fun detachedAgent(
        original: Any,
        objects: FrameworkFixtureObjects,
    ): Any? {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) return null
        return constructAgent(original, objects)
    }

    private fun constructAgent(
        original: Any,
        objects: FrameworkFixtureObjects,
    ): Any? {
        val constructor =
            original.javaClass.constructors.singleOrNull { candidate ->
                candidate.parameterTypes.map { it.simpleName } == agentParameters
            } ?: return null
        val scoreBuilder = constructor.parameterTypes[6].declaredClasses.singleOrNull { it.simpleName == "Builder" } ?: return null
        val score = XposedHelpers.callMethod(XposedHelpers.newInstance(scoreBuilder), "build")
        // This Android constructor initializes a detached object. The fixture
        // supplies no agent binder, never registers it, and never edits the original.
        return constructor.newInstance(
            null,
            objects.vpn,
            objects.copyInfo(objects.vpnInfo),
            objects.properties(objects.vpn),
            objects.capabilities(objects.vpn),
            null,
            score,
            XposedHelpers.getObjectField(original, "mContext"),
            XposedHelpers.getObjectField(original, "mHandler"),
            XposedHelpers.getObjectField(original, "networkAgentConfig"),
            XposedHelpers.getObjectField(original, "mConnService"),
            XposedHelpers.getObjectField(original, "mNetd"),
            XposedHelpers.getObjectField(original, "mDnsResolver"),
            XposedHelpers.getIntField(original, "factorySerialNumber"),
            XposedHelpers.getIntField(original, "creatorUid"),
            false,
            XposedHelpers.getIntField(original, "mLingerDurationMs"),
            XposedHelpers.getObjectField(original, "mQosCallbackTracker"),
            XposedHelpers.getObjectField(original, "mConnServiceDeps"),
        )
    }

    data class Input(
        val registration: FixturePendingRegistration,
        val ordinary: Network,
        val agent: Any,
    )

    private val agentParameters =
        listOf(
            "INetworkAgent",
            "Network",
            "NetworkInfo",
            "LinkProperties",
            "NetworkCapabilities",
            "LocalNetworkConfig",
            "NetworkScore",
            "Context",
            "Handler",
            "NetworkAgentConfig",
            "ConnectivityService",
            "INetd",
            "IDnsResolver",
            "int",
            "int",
            "boolean",
            "int",
            "QosCallbackTracker",
            "Dependencies",
        )
}
