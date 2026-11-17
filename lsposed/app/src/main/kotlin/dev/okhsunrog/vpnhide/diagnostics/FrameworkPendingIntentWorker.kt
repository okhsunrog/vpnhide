package dev.okhsunrog.vpnhide.diagnostics

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.os.Build
import android.os.Handler
import android.os.SystemClock
import android.util.Log
import androidx.core.content.IntentCompat
import dev.okhsunrog.vpnhide.BuildConfig
import dev.okhsunrog.vpnhide.networkBroadcastIntent
import dev.okhsunrog.vpnhide.registerPrivateNetworkReceiver
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
import uniffi.vpnhide_core_ffi.CoreFrameworkPhase
import uniffi.vpnhide_core_ffi.CorePendingObservation
import uniffi.vpnhide_core_ffi.CorePendingRegistration
import java.util.UUID

private const val BROADCAST_TIMEOUT_MS = 800L
private const val LATE_BROADCAST_WINDOW_MS = 60L

internal suspend fun observeFrameworkPendingIntents(
    context: Context,
    manager: ConnectivityManager,
    handler: Handler,
    packagesFinished: Deferred<Unit>,
    phase: CoreFrameworkPhase,
): List<CorePendingObservation> {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) return emptyList()
    val request = NetworkRequest.Builder().addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET).build()
    return coroutineScope {
        CorePendingRegistration.entries
            .map { kind ->
                async(start = CoroutineStart.UNDISPATCHED) {
                    observePendingIntent(context, manager, request, kind, handler, packagesFinished, phase)
                }
            }.awaitAll()
            .filterNotNull()
    }
}

private suspend fun observePendingIntent(
    context: Context,
    manager: ConnectivityManager,
    request: NetworkRequest,
    kind: CorePendingRegistration,
    handler: Handler,
    packagesFinished: Deferred<Unit>,
    phase: CoreFrameworkPhase,
): CorePendingObservation? {
    val action = "${context.packageName}.diagnostic.network.${UUID.randomUUID()}"
    val receiver = PendingNetworkObservation(action, kind, phase)
    if (runCatching { registerPrivateNetworkReceiver(context, receiver, IntentFilter(action), handler) }.isFailure) return null
    try {
        val pending =
            runCatching {
                networkBroadcastIntent(context, action, 0, Intent.FLAG_RECEIVER_FOREGROUND)
            }.getOrNull() ?: return null
        try {
            val started = SystemClock.elapsedRealtime()
            val registration =
                runCatching {
                    when (kind) {
                        CorePendingRegistration.LISTEN -> manager.registerNetworkCallback(request, pending)
                        CorePendingRegistration.REQUEST -> manager.requestNetwork(request, pending)
                    }
                }
            if (registration.isFailure) {
                receiver.trace("registration_error=${registration.exceptionOrNull()?.javaClass?.simpleName}")
                return null
            }
            val registeredAt = SystemClock.elapsedRealtime()
            receiver.trace("stage=registered start_ms=$started registered_ms=$registeredAt")
            try {
                packagesFinished.await()
                return receiver.observe().also { observation ->
                    receiver.trace(
                        "stage=finished boot_ms=${SystemClock.elapsedRealtime()} " +
                            "handles=${observation.handles.size} malformed=${observation.malformed}",
                    )
                }
            } finally {
                manager.unregisterNetworkCallback(pending)
            }
        } finally {
            pending.cancel()
        }
    } finally {
        context.unregisterReceiver(receiver)
    }
}

private class PendingNetworkObservation(
    private val action: String,
    private val kind: CorePendingRegistration,
    private val phase: CoreFrameworkPhase,
) : BroadcastReceiver() {
    private val delivered = CompletableDeferred<Long>()
    private val handles = mutableListOf<ULong>()
    private var malformed = false

    @Synchronized override fun onReceive(
        context: Context,
        intent: Intent,
    ) {
        if (intent.action != action) return
        val network = IntentCompat.getParcelableExtra(intent, ConnectivityManager.EXTRA_NETWORK, Network::class.java)
        val request = IntentCompat.getParcelableExtra(intent, ConnectivityManager.EXTRA_NETWORK_REQUEST, NetworkRequest::class.java)
        // Preserve even a tagged network with an invalid request extra as possible leak evidence.
        val validHandle = network?.networkHandle?.takeIf { it != 0L }
        if (validHandle != null && handles.size < 4) {
            handles.add(validHandle.toULong())
        } else {
            malformed = true
        }
        if (request?.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) != true) malformed = true
        val now = SystemClock.elapsedRealtime()
        if (delivered.complete(now)) trace("stage=received boot_ms=$now")
    }

    suspend fun observe(): CorePendingObservation =
        withContext(Dispatchers.Default) {
            val first = withTimeoutOrNull(BROADCAST_TIMEOUT_MS) { delivered.await() }
            if (first != null) delay((first + LATE_BROADCAST_WINDOW_MS - SystemClock.elapsedRealtime()).coerceAtLeast(0))
            snapshot()
        }

    @Synchronized private fun snapshot(): CorePendingObservation = CorePendingObservation(kind, handles.toList(), malformed)

    fun trace(message: String) {
        if (BuildConfig.DEBUG) Log.i("VpnHideDiagnosticWorker", "pending phase=$phase kind=$kind $message")
    }
}
