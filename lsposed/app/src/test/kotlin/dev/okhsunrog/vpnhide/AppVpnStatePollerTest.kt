package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.AppVpnStateSnapshot
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticGate
import dev.okhsunrog.vpnhide.diagnostics.appVpnStateNeedsRefresh
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AppVpnStatePollerTest {
    private fun state(
        gate: DiagnosticGate,
        session: String? = "vpn:1",
    ) = AppVpnStateSnapshot(gate, session, listOf("tun0"))

    @Test
    fun `stable routed poll stays silent`() {
        val routed = state(DiagnosticGate.ROUTED)
        assertFalse(appVpnStateNeedsRefresh(routed, routed))
    }

    @Test
    fun `changed app VPN fact requests shared refresh`() {
        assertTrue(
            appVpnStateNeedsRefresh(
                state(DiagnosticGate.SELF_NOT_ROUTED),
                state(DiagnosticGate.ROUTED),
            ),
        )
    }

    @Test
    fun `successful hidden poll recovers a failed shared observation`() {
        val routed = state(DiagnosticGate.ROUTED)
        assertTrue(appVpnStateNeedsRefresh(routed, routed, failed = true))
    }

    @Test
    fun `missing poll result and pending restart stay silent`() {
        assertFalse(appVpnStateNeedsRefresh(state(DiagnosticGate.ROUTED), null))
        assertFalse(
            appVpnStateNeedsRefresh(
                state(DiagnosticGate.NEEDS_RESTART),
                state(DiagnosticGate.ROUTED),
            ),
        )
    }
}
