package dev.okhsunrog.vpnhide.diagnostics

/**
 * Latched detection of the VPN coming up, so hiding is confirmed once per session.
 * Fires when the routing gate reads [DiagnosticGate.ROUTED] while armed, then
 * disarms; a real [DiagnosticGate.VPN_OFF] re-arms it. The settling states the gate
 * passes on the way up (SELF_NOT_ROUTED, NEEDS_RESTART) and a loading null keep the
 * arm state, so a flap through them — or a Wi-Fi/cellular handover that stays routed
 * — does not re-fire. Returns the next arm state and whether to fire now.
 */
internal fun vpnConfirmLatch(
    armed: Boolean,
    gate: DiagnosticGate?,
): Pair<Boolean, Boolean> =
    when (gate) {
        DiagnosticGate.VPN_OFF -> true to false
        DiagnosticGate.ROUTED -> false to armed
        else -> armed to false
    }
