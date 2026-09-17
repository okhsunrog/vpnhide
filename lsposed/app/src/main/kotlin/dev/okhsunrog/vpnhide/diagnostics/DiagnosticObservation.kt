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
