package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.DiagnosticGate
import dev.okhsunrog.vpnhide.diagnostics.vpnConfirmLatch
import org.junit.Assert.assertEquals
import org.junit.Test

class DiagnosticObservationTest {
    /** Fold the latch over a gate event stream the way the poller's collector does. */
    private fun fires(
        initiallyRouted: Boolean,
        events: List<DiagnosticGate?>,
    ): Int {
        var armed = !initiallyRouted
        var count = 0
        for (gate in events) {
            val (next, fire) = vpnConfirmLatch(armed, gate)
            armed = next
            if (fire) count++
        }
        return count
    }

    @Test
    fun `one confirmation per VPN-up across settling flaps and a real disconnect`() {
        val stream =
            listOf(
                null,
                DiagnosticGate.SELF_NOT_ROUTED,
                DiagnosticGate.ROUTED, // first VPN-up: fire
                DiagnosticGate.ROUTED,
                DiagnosticGate.SELF_NOT_ROUTED,
                DiagnosticGate.ROUTED, // settling flap / handover: no fire
                DiagnosticGate.VPN_OFF, // real disconnect: re-arm
                null,
                DiagnosticGate.SELF_NOT_ROUTED,
                DiagnosticGate.ROUTED, // second VPN-up: fire
            )
        assertEquals(2, fires(initiallyRouted = false, events = stream))
    }

    @Test
    fun `a session that starts already routed does not confirm until a real cycle`() {
        // Cold start / resume while routed: the first routed reads must not fire.
        assertEquals(0, fires(initiallyRouted = true, events = listOf(DiagnosticGate.ROUTED, null, DiagnosticGate.ROUTED)))
        // Only after a real VPN_OFF and a fresh routed read does it confirm.
        assertEquals(
            1,
            fires(initiallyRouted = true, events = listOf(DiagnosticGate.ROUTED, DiagnosticGate.VPN_OFF, DiagnosticGate.ROUTED)),
        )
    }
}
