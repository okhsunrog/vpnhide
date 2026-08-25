package dev.okhsunrog.vpnhide.diagnostics

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import dev.okhsunrog.vpnhide.StateCache
import dev.okhsunrog.vpnhide.debug.captureGateFrom
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.FlowPreview
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.debounce
import kotlinx.coroutines.flow.launchIn
import kotlinx.coroutines.flow.onEach
import kotlinx.coroutines.launch

private const val VPN_TRANSPORT_DEBOUNCE_MS = 750L

/**
 * Process-scoped VPN transport watcher (Phase 3 of the routing-gate unification):
 * a lightweight [ConnectivityManager] callback whose ONLY job is to trigger a fresh
 * [RoutingGateCache] probe when a VPN network appears, disappears, or changes
 * capabilities, so Diagnostics/Dashboard/export/logcat auto-update on VPN up/down
 * instead of waiting for the user to press a manual re-check.
 *
 * It is deliberately just a trigger. The gate VALUE must still come from the
 * root-shell probe path ([RoutingGateCache.load] -> [captureGateFrom] -> the root
 * `operstate` read + [GroundTruthProbe.selfRoutedThroughVpn]) — NEVER from
 * [NetworkCapabilities] itself. In split-tunnel or hardened setups the framework's
 * view of "is a VPN network up" and the kernel/root ground truth diverge, and
 * deriving the gate from the callback's capabilities would regress exactly the
 * correctness this project's diagnostics exist to guarantee.
 *
 * Callbacks land on a binder thread, so every event is funneled through a
 * [MutableSharedFlow] and debounced ~750ms before the actual (suspend, `su`-backed)
 * refresh — a burst of onCapabilitiesChanged around VPN up/down collapses into one
 * probe, and [StateCache]'s single-inflight [refreshInPlace][StateCache.refreshInPlace]
 * guards against overlap with a manual re-check.
 *
 * The manual "re-check" buttons remain everywhere as a fallback for what this
 * callback cannot see: root access just granted, a split-tunnel "app-in-tunnel"
 * change with no VPN transport event, or a missed callback.
 */
internal object VpnTransportWatcher {
    private val watcherScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val events = MutableSharedFlow<Unit>(extraBufferCapacity = 8)

    @Volatile private var started = false

    /** Registers the callback once for the process lifetime. Safe to call from
     * every `onCreate` (e.g. after an activity recreation) — idempotent. */
    @OptIn(FlowPreview::class)
    fun start(context: Context) {
        if (started) return
        started = true

        events
            .debounce(VPN_TRANSPORT_DEBOUNCE_MS)
            .onEach { runCatching { RoutingGateCache.refreshInPlace(force = true) } }
            .launchIn(watcherScope)

        val cm = context.applicationContext.getSystemService(ConnectivityManager::class.java) ?: return
        val request = NetworkRequest.Builder().addTransportType(NetworkCapabilities.TRANSPORT_VPN).build()
        val callback =
            object : ConnectivityManager.NetworkCallback() {
                override fun onAvailable(network: Network) = signal()

                override fun onLost(network: Network) = signal()

                override fun onCapabilitiesChanged(
                    network: Network,
                    networkCapabilities: NetworkCapabilities,
                ) = signal()

                private fun signal() {
                    watcherScope.launch { events.emit(Unit) }
                }
            }
        runCatching { cm.registerNetworkCallback(request, callback) }
    }
}
