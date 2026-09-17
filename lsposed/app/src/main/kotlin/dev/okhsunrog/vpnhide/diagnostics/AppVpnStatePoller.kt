package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.ReadReason
import kotlinx.coroutines.delay

private const val APP_VPN_STATE_POLL_INTERVAL_MS = 1_000L

/**
 * Foreground owner of the current app-scoped VPN state. Each iteration asks the
 * privileged helper for the actual fact; it does not infer a UID transition from
 * changes in global interface, route or rule text. It only keeps the shared
 * observation current: whether a changed fact needs a confirmation suite is
 * decided from the presentation it produces (`owedConfirmation`), never here.
 */
internal object AppVpnStatePoller {
    suspend fun pollWhileVisible() {
        while (true) {
            if (RoutingGateCache.observedStateChanged()) {
                RoutingGateCache.refreshRetained(ReadReason.Transition)
            }
            delay(APP_VPN_STATE_POLL_INTERVAL_MS)
        }
    }
}
