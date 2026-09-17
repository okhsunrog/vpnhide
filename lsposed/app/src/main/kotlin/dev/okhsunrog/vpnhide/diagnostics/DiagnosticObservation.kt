package dev.okhsunrog.vpnhide.diagnostics

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
