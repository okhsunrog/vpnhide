package dev.okhsunrog.vpnhide.diagnostics

import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.os.Handler
import android.os.SystemClock
import android.util.Log
import dev.okhsunrog.vpnhide.BuildConfig
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import uniffi.vpnhide_core_ffi.CoreCallbackEvent
import uniffi.vpnhide_core_ffi.CoreCallbackObservation
import uniffi.vpnhide_core_ffi.CoreCallbackRegistration
import uniffi.vpnhide_core_ffi.CoreFrameworkPhase
import uniffi.vpnhide_core_ffi.CoreFrameworkTask

private const val CONTROL_EVENT_TIMEOUT_MS = 900L
private const val LATE_EVENT_WINDOW_MS = 100L

/** Each registration uses the public app API and its own Android callback object. */
internal suspend fun observeFrameworkCallbacks(
    manager: ConnectivityManager,
    task: CoreFrameworkTask,
    handler: Handler,
    queriesFinished: Deferred<Unit>,
): List<CoreCallbackObservation> =
    coroutineScope {
        val request = NetworkRequest.Builder().addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET).build()
        // Register in order on the collector thread. Per-registration observation windows can overlap.
        CoreCallbackRegistration.entries
            .map { kind ->
                async(start = CoroutineStart.UNDISPATCHED) {
                    val observed = observeCallback(manager, task, handler, request, kind, queriesFinished)
                    val quiet = task.phase == CoreFrameworkPhase.PREPARED || task.phase == CoreFrameworkPhase.REVOKED
                    // In a quiet phase there is no source ACK to consume. Reopen just this
                    // registration if it yielded nothing, while the same phase is still active.
                    if (quiet && observed != null && observed.events.isEmpty() && !observed.vpnProperties && !observed.vpnTransport) {
                        if (BuildConfig.DEBUG) Log.i("VpnHideDiagnosticWorker", "callback phase=${task.phase} kind=$kind retry=empty")
                        observeCallback(manager, task, handler, request, kind, queriesFinished)
                    } else {
                        observed
                    }
                }
            }.awaitAll()
            .filterNotNull()
    }

private suspend fun observeCallback(
    manager: ConnectivityManager,
    task: CoreFrameworkTask,
    handler: Handler,
    request: NetworkRequest,
    kind: CoreCallbackRegistration,
    queriesFinished: Deferred<Unit>,
): CoreCallbackObservation? {
    val started = SystemClock.elapsedRealtime()
    val callback = CallbackObservation(kind, task)
    val registration =
        runCatching {
            when (kind) {
                CoreCallbackRegistration.LISTEN -> manager.registerNetworkCallback(request, callback, handler)
                CoreCallbackRegistration.DEFAULT -> manager.registerDefaultNetworkCallback(callback, handler)
                CoreCallbackRegistration.REQUEST -> manager.requestNetwork(request, callback, handler)
            }
        }
    if (registration.isFailure) {
        if (BuildConfig.DEBUG) {
            Log.i(
                "VpnHideDiagnosticWorker",
                "callback phase=${task.phase} kind=$kind registration_error=${registration.exceptionOrNull()?.javaClass?.simpleName}",
            )
        }
        return null
    }
    val registeredAt = SystemClock.elapsedRealtime()
    if (BuildConfig.DEBUG) {
        Log.i(
            "VpnHideDiagnosticWorker",
            "callback phase=${task.phase} kind=$kind stage=registered start_ms=$started registered_ms=$registeredAt",
        )
    }
    return try {
        queriesFinished.await()
        callback.observe().also { observation ->
            if (BuildConfig.DEBUG) callback.logTiming(started, registeredAt)
        }
    } finally {
        manager.unregisterNetworkCallback(callback)
    }
}

private class CallbackObservation(
    private val registration: CoreCallbackRegistration,
    private val task: CoreFrameworkTask,
) : ConnectivityManager.NetworkCallback() {
    private val ordinaryProperties = CompletableDeferred<Long>()
    private val events = mutableListOf<CoreCallbackEvent>()
    private var vpnTransport = false
    private var vpnProperties = false
    private var firstEventAt: Long? = null
    private var ordinaryPropertiesAt: Long? = null
    private var unavailableAt: Long? = null

    suspend fun observe(): CoreCallbackObservation? =
        withContext(Dispatchers.Default) {
            // Other registrations can be unregistering on the collector during this wait.
            val ordinaryAt = withTimeoutOrNull(CONTROL_EVENT_TIMEOUT_MS) { ordinaryProperties.await() }
            if (ordinaryAt != null) delay((ordinaryAt + LATE_EVENT_WINDOW_MS - SystemClock.elapsedRealtime()).coerceAtLeast(0))
            snapshot()
        }

    @Synchronized override fun onAvailable(network: Network) {
        record(network, lost = false)
    }

    @Synchronized override fun onLost(network: Network) {
        record(network, lost = true)
    }

    @Synchronized override fun onUnavailable() {
        unavailableAt = SystemClock.elapsedRealtime()
    }

    @Synchronized override fun onCapabilitiesChanged(
        network: Network,
        networkCapabilities: NetworkCapabilities,
    ) {
        vpnTransport = vpnTransport || networkCapabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN)
    }

    @Synchronized override fun onLinkPropertiesChanged(
        network: Network,
        linkProperties: LinkProperties,
    ) {
        vpnProperties = vpnProperties || linkProperties.interfaceName == task.vpnMarker
        if (network.networkHandle.toULong() == task.ordinaryHandle && ordinaryPropertiesAt == null) {
            val now = SystemClock.elapsedRealtime()
            ordinaryPropertiesAt = now
            ordinaryProperties.complete(now)
            if (BuildConfig.DEBUG) {
                Log.i("VpnHideDiagnosticWorker", "callback phase=${task.phase} kind=$registration stage=ordinary boot_ms=$now")
            }
        }
    }

    private fun record(
        network: Network,
        lost: Boolean,
    ) {
        if (firstEventAt == null) firstEventAt = SystemClock.elapsedRealtime()
        // One extra entry signals overflow and prevents an absence claim from a truncated recording.
        if (events.size <= 32) events.add(CoreCallbackEvent(network.networkHandle.toULong(), lost))
    }

    @Synchronized private fun snapshot(): CoreCallbackObservation? {
        if (events.size > 32) return null
        // AVAILABLE by itself does not prove the ordinary control. Keep all other
        // events, in particular a VPN delivery even if ordinary properties timed out.
        val qualified =
            if (ordinaryPropertiesAt != null) {
                events.toList()
            } else {
                events.filterNot { it.handle == task.ordinaryHandle && !it.lost }
            }
        return CoreCallbackObservation(registration, qualified, vpnTransport, vpnProperties)
    }

    @Synchronized fun logTiming(
        started: Long,
        registered: Long,
    ) {
        Log.i(
            "VpnHideDiagnosticWorker",
            "callback phase=${task.phase} kind=$registration start_ms=$started registered_ms=$registered " +
                "first_event_ms=$firstEventAt properties_ms=$ordinaryPropertiesAt unavailable_ms=$unavailableAt " +
                "finish_ms=${SystemClock.elapsedRealtime()} events=${events.size}",
        )
    }
}
