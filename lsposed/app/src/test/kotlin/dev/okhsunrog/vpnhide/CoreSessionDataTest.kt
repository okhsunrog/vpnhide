package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CoreSessionDataTest {
    private fun state(
        epoch: Long,
        revision: Long,
    ) = CoreStateView(epoch, 1, RootCanonicalRead.Missing, emptyList(), revision = revision)

    @Test
    fun `old connection cannot introduce an unseen epoch after reconnect`() {
        var order = acceptCorePublication(CorePublicationData(), 1, 1, state(77, 30))
        order = acceptCorePublication(order, 3, 4, state(99, 1))
        val late = acceptCorePublication(order, 2, 3, state(88, 500))
        assertEquals(order, late)
        assertEquals(99L, late.update.state!!.epoch)
    }

    @Test
    fun `late failures cannot withdraw a successful new connection or newer request`() {
        val order = acceptCorePublication(CorePublicationData(), 3, 7, state(99, 5))
        assertEquals(order, failCorePublication(order, 2, 8, true))
        assertEquals(order, failCorePublication(order, 3, 6, false))
        val failed = failCorePublication(order, 3, 8, false)
        assertFalse(failed.update.current)
        assertEquals(order.update.state, failed.update.state)
        assertTrue(acceptCorePublication(failed, 4, 9, state(100, 1)).update.current)
    }

    @Test
    fun `lower revisions and an epoch change on one connection are rejected`() {
        val order = acceptCorePublication(CorePublicationData(), 3, 7, state(99, 5))
        assertEquals(order, acceptCorePublication(order, 3, 8, state(99, 4)))
        assertEquals(order, acceptCorePublication(order, 3, 8, state(88, 100)))
    }

    @Test
    fun `failed new connection can subsequently publish its own new epoch`() {
        val previous = acceptCorePublication(CorePublicationData(), 1, 7, state(77, 5))
        val failed = failCorePublication(previous, 2, 8, false)
        val recovered = acceptCorePublication(failed, 2, 9, state(88, 1))
        assertTrue(recovered.update.current)
        assertEquals(88L, recovered.update.state!!.epoch)
    }

    @Test
    fun `new connection failure dominates request issue order`() {
        val previous = acceptCorePublication(CorePublicationData(), 1, 8, state(77, 5))
        val failed = failCorePublication(previous, 2, 7, false)
        assertFalse(failed.update.current)
        assertEquals(2L, failed.update.connectionGeneration)
        assertEquals(1L, failed.successfulStateGeneration)
    }

    @Test
    fun `late same connection success cannot undo a newer failure`() {
        val previous = acceptCorePublication(CorePublicationData(), 1, 7, state(77, 5))
        val failed = failCorePublication(previous, 1, 9, false)
        assertEquals(failed, acceptCorePublication(failed, 1, 8, state(77, 6)))
        assertTrue(acceptCorePublication(failed, 1, 10, state(77, 6)).update.current)
    }

    @Test
    fun `request watermarks belong to their connection generation`() {
        val previous = acceptCorePublication(CorePublicationData(), 1, 8, state(77, 5))
        val connected = acceptCorePublication(previous, 2, 5, state(88, 1))
        assertEquals(5L, connected.latestSuccessfulRequest)
        assertFalse(failCorePublication(connected, 2, 6, false).update.current)
    }
}
