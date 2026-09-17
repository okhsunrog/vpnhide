package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.AppVpnStateSnapshot
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticGate
import dev.okhsunrog.vpnhide.diagnostics.appVpnStateNeedsConfirmation
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AppVpnStatePollerTest {
    private fun state(
        gate: DiagnosticGate,
        session: String? = "vpn:1",
    ) = AppVpnStateSnapshot(gate, session, listOf("tun0"))

    @Test
    fun `stable exclusion to routed requests confirmation`() {
        assertTrue(
            appVpnStateNeedsConfirmation(
                state(DiagnosticGate.SELF_NOT_ROUTED),
                state(DiagnosticGate.ROUTED),
            ),
        )
    }

    @Test
    fun `cold start and repeated routed polls do not request confirmation`() {
        val routed = state(DiagnosticGate.ROUTED)
        assertFalse(appVpnStateNeedsConfirmation(null, routed))
        assertFalse(appVpnStateNeedsConfirmation(routed, routed))
    }

    @Test
    fun `new routed VPN session requests confirmation without an off sample`() {
        assertTrue(
            appVpnStateNeedsConfirmation(
                state(DiagnosticGate.ROUTED, "vpn:1"),
                state(DiagnosticGate.ROUTED, "vpn:2"),
            ),
        )
    }
}
