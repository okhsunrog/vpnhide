package dev.okhsunrog.vpnhide.diagnostics

/**
 * A routed result needs a fresh hiding check when either this app has just moved
 * into the VPN or the VPN session itself changed. Repeated polls of one routed
 * session do not retrigger it, and a cold-start routed value has no previous edge.
 */
internal fun appVpnStateNeedsConfirmation(
    previous: AppVpnStateSnapshot?,
    next: AppVpnStateSnapshot,
): Boolean =
    previous != null &&
        next.gate == DiagnosticGate.ROUTED &&
        (previous.gate != DiagnosticGate.ROUTED || previous.session != next.session)

/** A silent poll only publishes a refresh after it found new or recoverable evidence. */
internal fun appVpnStateNeedsRefresh(
    published: AppVpnStateSnapshot?,
    observed: AppVpnStateSnapshot?,
    stale: Boolean = false,
    failed: Boolean = false,
): Boolean =
    published != null &&
        observed != null &&
        published.gate != DiagnosticGate.NEEDS_RESTART &&
        (published != observed || stale || failed)
