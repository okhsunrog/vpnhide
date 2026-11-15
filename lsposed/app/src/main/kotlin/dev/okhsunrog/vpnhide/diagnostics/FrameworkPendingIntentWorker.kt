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
import androidx.core.content.IntentCompat
import dev.okhsunrog.vpnhide.networkBroadcastIntent
import dev.okhsunrog.vpnhide.registerPrivateNetworkReceiver
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.delay
import kotlinx.coroutines.withTimeoutOrNull
import uniffi.vpnhide_core_ffi.CorePendingObservation
import uniffi.vpnhide_core_ffi.CorePendingRegistration
import java.util.UUID

internal suspend fun observeFrameworkPendingIntents(
    context: Context,
    manager: ConnectivityManager,
): List<CorePendingObservation> {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) return emptyList()
    val request = NetworkRequest.Builder().addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET).build()
    return CorePendingRegistration.entries.mapNotNull { kind -> observePendingIntent(context, manager, request, kind) }
}

private suspend fun observePendingIntent(
    context: Context,
    manager: ConnectivityManager,
    request: NetworkRequest,
    kind: CorePendingRegistration,
): CorePendingObservation? {
    val action = "${context.packageName}.diagnostic.network.${UUID.randomUUID()}"
    val receiver = PendingNetworkObservation(action, kind)
    if (runCatching { registerPrivateNetworkReceiver(context, receiver, IntentFilter(action)) }.isFailure) return null
    try {
        val pending = runCatching { networkBroadcastIntent(context, action, 0) }.getOrNull() ?: return null
        try {
            val registered =
                runCatching {
                    when (kind) {
                        CorePendingRegistration.LISTEN -> manager.registerNetworkCallback(request, pending)
                        CorePendingRegistration.REQUEST -> manager.requestNetwork(request, pending)
                    }
                }.isSuccess
            if (!registered) return null
            try {
                withTimeoutOrNull(800) {
                    receiver.delivered.await()
                    delay(60)
                }
                return receiver.snapshot()
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
) : BroadcastReceiver() {
    val delivered = CompletableDeferred<Unit>()
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
        delivered.complete(Unit)
    }

    @Synchronized fun snapshot(): CorePendingObservation = CorePendingObservation(kind, handles.toList(), malformed)
}
