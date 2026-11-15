package dev.okhsunrog.vpnhide.hook

import android.app.PendingIntent
import android.net.ConnectivityManager
import android.net.Network
import de.robv.android.xposed.XposedHelpers

/** Feed detached agents through the platform dispatchers for actual worker registrations. */
internal class FrameworkNetworkDispatchFixture(
    serviceClass: Class<*>,
) {
    private val callback =
        serviceClass.declaredMethods
            .single {
                it.name == "callCallbackForRequest" && it.parameterTypes.getOrNull(1)?.simpleName == "NetworkAgentInfo"
            }.apply { isAccessible = true }
    private val pending =
        serviceClass.declaredMethods
            .single {
                it.name == "sendPendingIntentForRequest" && it.parameterTypes.getOrNull(1)?.simpleName == "NetworkAgentInfo"
            }.apply { isAccessible = true }
    private val constructor =
        callback.parameterTypes[1].constructors.single { candidate ->
            candidate.parameterTypes.map { it.simpleName } == agentParameters
        }
    private val available = XposedHelpers.getStaticIntField(ConnectivityManager::class.java, "CALLBACK_AVAILABLE")
    private val lost = XposedHelpers.getStaticIntField(ConnectivityManager::class.java, "CALLBACK_LOST")

    fun registration(
        requestInfo: Any,
        worker: FixtureWorker,
    ): Registration? {
        val request = XposedHelpers.callMethod(requestInfo, "getNetworkRequestForCallback") ?: return null
        val kind = XposedHelpers.getObjectField(request, "type").toString()
        val intent = XposedHelpers.getObjectField(requestInfo, "mPendingIntent") as? PendingIntent
        if (intent != null) {
            if (intent.creatorUid != worker.uid || XposedHelpers.getBooleanField(requestInfo, "mPendingIntentSent")) return null
            val mode =
                when (kind) {
                    "LISTEN" -> FixturePendingRegistration.LISTEN
                    "REQUEST" -> FixturePendingRegistration.REQUEST
                    else -> return null
                }
            return Registration.Broadcast(mode)
        }
        val mode =
            when (kind) {
                "LISTEN" -> FixtureCallbackRegistration.LISTEN
                "TRACK_DEFAULT" -> FixtureCallbackRegistration.DEFAULT
                "REQUEST" -> FixtureCallbackRegistration.REQUEST
                else -> return null
            }
        return Registration.Callback(mode)
    }

    fun send(
        instance: Any,
        requestInfo: Any,
        registration: Registration,
        vpn: Any?,
        ordinary: Any,
        stillAuthorized: () -> Boolean,
    ) {
        when (registration) {
            is Registration.Callback -> {
                if (vpn != null && stillAuthorized()) callback.invoke(instance, requestInfo, vpn, available, 0)
                if (vpn != null && stillAuthorized()) callback.invoke(instance, requestInfo, vpn, lost, 0)
                if (stillAuthorized()) callback.invoke(instance, requestInfo, ordinary, available, 0)
            }

            is Registration.Broadcast -> {
                if (vpn != null && stillAuthorized()) pending.invoke(instance, requestInfo, vpn, available)
                if (stillAuthorized() && !XposedHelpers.getBooleanField(requestInfo, "mPendingIntentSent")) {
                    pending.invoke(instance, requestInfo, ordinary, available)
                }
            }
        }
    }

    @Suppress("DEPRECATION") // NetworkAgentInfo still takes the platform's legacy NetworkInfo.
    fun detachedAgent(
        instance: Any,
        network: Network,
        objects: FrameworkFixtureObjects,
        uid: Int,
    ): Any {
        val score = buildArgument(6)
        val config = buildArgument(9)
        val info = if (network == objects.vpn) objects.vpnInfo else objects.ordinaryInfo
        return constructor.newInstance(
            null,
            network,
            objects.copyInfo(info),
            objects.properties(network),
            objects.capabilities(network),
            null,
            score,
            XposedHelpers.getObjectField(instance, "mContext"),
            XposedHelpers.getObjectField(instance, "mHandler"),
            config,
            instance,
            XposedHelpers.getObjectField(instance, "mNetd"),
            XposedHelpers.getObjectField(instance, "mDnsResolver"),
            0,
            uid,
            false,
            0,
            XposedHelpers.getObjectField(instance, "mQosCallbackTracker"),
            XposedHelpers.getObjectField(instance, "mDeps"),
        )
    }

    private fun buildArgument(index: Int): Any {
        val builder = constructor.parameterTypes[index].declaredClasses.single { it.simpleName == "Builder" }
        return XposedHelpers.callMethod(XposedHelpers.newInstance(builder), "build")
    }

    sealed interface Registration {
        data class Callback(
            val kind: FixtureCallbackRegistration,
        ) : Registration

        data class Broadcast(
            val kind: FixturePendingRegistration,
        ) : Registration
    }
}

internal fun fixtureRequestOwnedBy(
    requestInfo: Any,
    worker: FixtureWorker,
): Boolean =
    fixtureRequestField(requestInfo, "mUid", "uid") == worker.uid &&
        fixtureRequestField(requestInfo, "mPid", "pid") == worker.pid

private fun fixtureRequestField(
    instance: Any,
    vararg names: String,
): Int? = names.firstNotNullOfOrNull { runCatching { XposedHelpers.getIntField(instance, it) }.getOrNull() }

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
