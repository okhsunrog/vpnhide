package dev.okhsunrog.vpnhide.diagnostics

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import dev.okhsunrog.vpnhide.LogTags
import dev.okhsunrog.vpnhide.StateCache
import dev.okhsunrog.vpnhide.VpnHideLog
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
 * Registering the callback replays the current state (`onAvailable` plus the first
 * `onCapabilitiesChanged`) for every VPN network that already exists. That replay
 * is not a transition: the VPN networks present at registration are recorded
 * first and their replay is ignored ([reduceVpnTransport]), otherwise every cold
 * start with the VPN up would invalidate the routing gate a moment after its first
 * read and pay a second root snapshot on the Dashboard's critical path.
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
    private val lock = Any()
    private var knowledge = VpnTransportKnowledge()

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
        synchronized(lock) { knowledge = VpnTransportKnowledge(known = existingVpnNetworks(cm)) }
        val request = NetworkRequest.Builder().addTransportType(NetworkCapabilities.TRANSPORT_VPN).build()
        val callback =
            object : ConnectivityManager.NetworkCallback() {
                override fun onAvailable(network: Network) = observe(VpnTransportEvent.Available, network)

                override fun onLost(network: Network) = observe(VpnTransportEvent.Lost, network)

                override fun onCapabilitiesChanged(
                    network: Network,
                    networkCapabilities: NetworkCapabilities,
                ) = observe(VpnTransportEvent.CapabilitiesChanged, network)
            }
        runCatching { cm.registerNetworkCallback(request, callback) }
    }

    private fun observe(
        event: VpnTransportEvent,
        network: Network,
    ) {
        val decision = synchronized(lock) { reduceVpnTransport(knowledge, event, network.networkHandle).also { knowledge = it.knowledge } }
        VpnHideLog.d(LogTags.DIAG, "vpn transport $event net=$network transition=${decision.transition}")
        if (!decision.transition) return
        // Invalidate readiness now; debounce only the expensive read.
        RoutingGateCache.markStale()
        watcherScope.launch { events.emit(Unit) }
    }

    // allNetworks is deprecated for apps but is the one enumeration of every VPN
    // network present before registration; a callback cannot report what already existed.
    @Suppress("DEPRECATION")
    private fun existingVpnNetworks(cm: ConnectivityManager): Set<Long> =
        runCatching {
            cm.allNetworks
                .filter { cm.getNetworkCapabilities(it)?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) == true }
                .map { it.networkHandle }
                .toSet()
        }.getOrDefault(emptySet())
}
