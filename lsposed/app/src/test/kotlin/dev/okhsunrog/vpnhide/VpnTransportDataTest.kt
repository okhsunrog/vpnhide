package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.VpnTransportEvent
import dev.okhsunrog.vpnhide.diagnostics.VpnTransportKnowledge
import dev.okhsunrog.vpnhide.diagnostics.reduceVpnTransport
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class VpnTransportDataTest {
    @Test
    fun `registration replay of an existing VPN network is not a transition`() {
        var knowledge = VpnTransportKnowledge(known = setOf(100))
        val available = reduceVpnTransport(knowledge, VpnTransportEvent.Available, 100)
        assertFalse(available.transition)
        knowledge = available.knowledge
        val firstCapabilities = reduceVpnTransport(knowledge, VpnTransportEvent.CapabilitiesChanged, 100)
        assertFalse(firstCapabilities.transition)
        knowledge = firstCapabilities.knowledge
        assertTrue(reduceVpnTransport(knowledge, VpnTransportEvent.CapabilitiesChanged, 100).transition)
        assertTrue(reduceVpnTransport(knowledge, VpnTransportEvent.Lost, 100).transition)
    }

    @Test
    fun `a new VPN network and its later loss are transitions and forget the handle`() {
        val arrived = reduceVpnTransport(VpnTransportKnowledge(), VpnTransportEvent.Available, 7)
        assertTrue(arrived.transition)
        val settled = reduceVpnTransport(arrived.knowledge, VpnTransportEvent.CapabilitiesChanged, 7)
        assertFalse(settled.transition)
        val lost = reduceVpnTransport(settled.knowledge, VpnTransportEvent.Lost, 7)
        assertTrue(lost.transition)
        assertEquals(VpnTransportKnowledge(), lost.knowledge)
        // The same handle appearing again after a loss is a fresh arrival.
        assertTrue(reduceVpnTransport(lost.knowledge, VpnTransportEvent.Available, 7).transition)
    }

    @Test
    fun `capabilities of a network that was never announced count as its arrival`() {
        val decision = reduceVpnTransport(VpnTransportKnowledge(), VpnTransportEvent.CapabilitiesChanged, 9)
        assertFalse(decision.transition)
        assertEquals(VpnTransportKnowledge(known = setOf(9), capabilitiesSeen = setOf(9)), decision.knowledge)
    }
}
