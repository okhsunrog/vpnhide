package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.DiagnosticGate
import dev.okhsunrog.vpnhide.diagnostics.coreRoutingSnapshot
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class CoreRoutingDataTest {
    @Test
    fun `root UID membership and session reach diagnostic readiness`() {
        val routing = CoreRoutingView(true, 8, 10123, uniffi.vpnhide_core_ffi.CoreUidVpnRouteState.ROUTED, listOf("tun0"), "vpn-session")
        val state = CoreStateView(1, 0, RootCanonicalRead.Missing, emptyList(), routing = routing)
        val snapshot = requireNotNull(coreRoutingSnapshot(CoreSessionUpdate(state, current = true), 10123))
        assertEquals(DiagnosticGate.ROUTED, snapshot.gate)
        assertEquals("vpn-session", snapshot.session)
        assertEquals(listOf("tun0"), snapshot.interfaces)
    }

    @Test
    fun `disconnect unknown and another UID never become VPN off`() {
        val routing = CoreRoutingView(true, 8, 10123, uniffi.vpnhide_core_ffi.CoreUidVpnRouteState.VPN_OFF, emptyList())
        val state = CoreStateView(1, 0, RootCanonicalRead.Missing, emptyList(), routing = routing)
        assertNull(coreRoutingSnapshot(CoreSessionUpdate(state), 10123))
        assertNull(coreRoutingSnapshot(CoreSessionUpdate(state, current = true), 10124))
        assertNull(coreRoutingSnapshot(CoreSessionUpdate(state.copy(routing = routing.copy(current = false)), current = true), 10123))
        assertNull(
            coreRoutingSnapshot(
                CoreSessionUpdate(
                    state.copy(routing = routing.copy(state = uniffi.vpnhide_core_ffi.CoreUidVpnRouteState.UNKNOWN)),
                    current = true,
                ),
                10123,
            ),
        )
    }
}
