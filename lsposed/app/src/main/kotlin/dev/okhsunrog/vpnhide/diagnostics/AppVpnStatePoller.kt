package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.DashboardCache
import dev.okhsunrog.vpnhide.ReadReason
import kotlinx.coroutines.delay

private const val APP_VPN_STATE_POLL_INTERVAL_MS = 1_000L

/**
 * Foreground owner of the current app-scoped VPN state. Each iteration asks the
 * privileged helper for the actual fact; it does not infer a UID transition from
 * changes in global interface, route or rule text.
 */
internal object AppVpnStatePoller {
    suspend fun pollWhileVisible() {
        while (true) {
            val state = RoutingGateCache.observation.value
            if (state.attempted && !state.quarantined) {
                RoutingGateCache.refreshInPlace(force = true, reason = ReadReason.Background)
            }
            delay(APP_VPN_STATE_POLL_INTERVAL_MS)
        }
    }

    /** Run one confirmation whenever stable eligibility returns from off/excluded to routed. */
    suspend fun confirmRoutedTransitions() {
        var previous = RoutingGateCache.current.value
        RoutingGateCache.current.collect { next ->
            if (next == null) return@collect
            if (appVpnStateNeedsConfirmation(previous, next)) {
                DashboardCache.refreshRetained(ReadReason.Transition)
            }
            previous = next
        }
    }
}
