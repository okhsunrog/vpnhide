package dev.okhsunrog.vpnhide.diagnostics

import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.os.Handler
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.withTimeoutOrNull
import uniffi.vpnhide_core_ffi.CoreCallbackEvent
import uniffi.vpnhide_core_ffi.CoreCallbackObservation
import uniffi.vpnhide_core_ffi.CoreCallbackRegistration
import uniffi.vpnhide_core_ffi.CoreFrameworkTask

/** Each registration uses the public app API and its own Android callback object. */
internal suspend fun observeFrameworkCallbacks(
    manager: ConnectivityManager,
    task: CoreFrameworkTask,
    handler: Handler,
): List<CoreCallbackObservation> =
    coroutineScope {
        val request = NetworkRequest.Builder().addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET).build()
        // Register in order on the collector thread. Per-registration observation windows can overlap.
        CoreCallbackRegistration.entries
            .map { kind ->
                async(start = CoroutineStart.UNDISPATCHED) { observeCallback(manager, task, handler, request, kind) }
            }.awaitAll()
            .filterNotNull()
    }

private suspend fun observeCallback(
    manager: ConnectivityManager,
    task: CoreFrameworkTask,
    handler: Handler,
    request: NetworkRequest,
    kind: CoreCallbackRegistration,
): CoreCallbackObservation? {
    val callback = CallbackObservation(kind, task)
    val registered =
        runCatching {
            when (kind) {
                CoreCallbackRegistration.LISTEN -> manager.registerNetworkCallback(request, callback, handler)
                CoreCallbackRegistration.DEFAULT -> manager.registerDefaultNetworkCallback(callback, handler)
                CoreCallbackRegistration.REQUEST -> manager.requestNetwork(request, callback, handler)
            }
        }.isSuccess
    if (!registered) return null
    return try {
        // Allow the handler to drain the cover/loss sequence after the ordinary control arrives.
        withTimeoutOrNull(900) {
            callback.ordinaryProperties.await()
            delay(100)
            callback.snapshot()
        }
    } finally {
        manager.unregisterNetworkCallback(callback)
    }
}

private class CallbackObservation(
    private val registration: CoreCallbackRegistration,
    private val task: CoreFrameworkTask,
) : ConnectivityManager.NetworkCallback() {
    val ordinaryProperties = CompletableDeferred<Unit>()
    private val events = mutableListOf<CoreCallbackEvent>()
    private var vpnTransport = false
    private var vpnProperties = false

    @Synchronized override fun onAvailable(network: Network) {
        record(network, lost = false)
    }

    @Synchronized override fun onLost(network: Network) {
        record(network, lost = true)
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
        if (network.networkHandle.toULong() != task.vpnHandle) ordinaryProperties.complete(Unit)
    }

    private fun record(
        network: Network,
        lost: Boolean,
    ) {
        // One extra entry signals overflow and prevents an absence claim from a truncated recording.
        if (events.size <= 32) events.add(CoreCallbackEvent(network.networkHandle.toULong(), lost))
    }

    @Synchronized fun snapshot(): CoreCallbackObservation? =
        if (events.size <= 32) CoreCallbackObservation(registration, events.toList(), vpnTransport, vpnProperties) else null
}
