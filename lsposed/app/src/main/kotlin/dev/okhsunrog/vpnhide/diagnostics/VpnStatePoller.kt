package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.DashboardCache
import dev.okhsunrog.vpnhide.LogTags
import dev.okhsunrog.vpnhide.ObservationRequest
import dev.okhsunrog.vpnhide.ReadReason
import dev.okhsunrog.vpnhide.RootProcessResult
import dev.okhsunrog.vpnhide.RootProcessRunner
import dev.okhsunrog.vpnhide.StateCache
import dev.okhsunrog.vpnhide.buildRootShellSnapshotCommand
import dev.okhsunrog.vpnhide.parseRootShellSnapshot
import dev.okhsunrog.vpnhide.vpnPresenceFromSnapshot
import kotlinx.coroutines.delay

private const val VPN_POLL_INTERVAL_MS = 1_000L
private val routeExpiry = Regex(" expires [0-9]+(?:sec|msec)?\\b")

/** Process-owned reads survive a detached UI waiter; StateCache bounds and quarantines workers. */
internal object VpnStatePoller : StateCache<List<String>>(
    traceName = "vpn_poll",
    logTag = LogTags.DIAG,
    timeoutMillis = 15_000,
) {
    private val reader = RootProcessRunner()

    override val ready: Boolean get() = RoutingGateCache.observation.value.attempted

    /**
     * Keep the routing gate fresh while the app is foregrounded: the app's own Java
     * backend hides VPN callbacks from its process, so a light network-only root
     * sample is how a VPN change is noticed. A changed sample re-reads the gate; a
     * stable one costs only the sample. This does not decide the confirmation suite —
     * [confirmRoutedTransitions] watches the gate value for that, because self-routing
     * can resolve a beat after the interfaces appear.
     */
    suspend fun pollWhileVisible() {
        var previous: List<String>? = null
        var failed = false
        while (true) {
            // Startup and ON_RESUME already request a fresh gate. Wait before the first tick.
            delay(VPN_POLL_INTERVAL_MS)
            if (!ready || observation.value.quarantined) continue
            refreshInPlace(force = false, reason = ReadReason.Background)
            val next = current.value
            if (vpnPollNeedsRefresh(previous, next, failed)) {
                RoutingGateCache.markStale(if (next == null) ReadReason.Background else ReadReason.Transition)
                RoutingGateCache.refreshInPlace(force = true, reason = ReadReason.Background)
            }
            failed = next == null
            previous = next
        }
    }

    /**
     * The single confirmation trigger on a VPN-up, unifying what two screens used to
     * do off the gate. It watches the routing gate the poller keeps fresh and, latched
     * by [vpnConfirmLatch], requests exactly one confirmation once the gate actually
     * reads routed — which may be a later read than the first interface change, since
     * self-routing settles after the tunnel appears. `DashboardCache.refreshRetained`
     * runs one fresh suite (via `beforeRefresh`) and re-derives the tiles.
     */
    suspend fun confirmRoutedTransitions() {
        var armed = RoutingGateCache.gate.value != DiagnosticGate.ROUTED
        RoutingGateCache.gate.collect { gate ->
            val (next, fire) = vpnConfirmLatch(armed, gate)
            armed = next
            if (fire) DashboardCache.refreshRetained(ReadReason.Transition)
        }
    }

    override suspend fun load(request: ObservationRequest): List<String> {
        val result = reader.runAndDrain(listOf("su", "-c", buildRootShellSnapshotCommand(networkOnly = true)))
        val output = (result as? RootProcessResult.Completed)?.output ?: error("VPN observation unavailable")
        return vpnPollFingerprint(parseRootShellSnapshot(output))
    }
}

/** Initial sampling also refreshes: VPN may have changed between startup/resume and the first tick. */
internal fun vpnPollNeedsRefresh(
    previous: List<String>?,
    next: List<String>?,
    failed: Boolean,
): Boolean = if (next == null) !failed else failed || previous != next

internal fun vpnPollFingerprint(sections: Map<String, String>): List<String> {
    val presence = vpnPresenceFromSnapshot(sections)
    val routing = listOf("vpn_routes4", "vpn_routes6", "vpn_rules4", "vpn_rules6")
    require(routing.all { sections[it].orEmpty().lineSequence().any { line -> line == "probe_ok" } }) {
        "VPN routing observation incomplete"
    }
    return listOf(presence.interfaces.sorted().joinToString(","), presence.frameworkInterfaces.sorted().joinToString(",")) +
        routing.map { key ->
            sections
                .getValue(key)
                .lineSequence()
                .filter { it.isNotBlank() }
                // Kernel RA lifetimes count down even on a completely stable network.
                // Actual route expiration still changes the fingerprint by removing the route.
                .map { it.replace(routeExpiry, "") }
                .joinToString("\n")
        }
}
