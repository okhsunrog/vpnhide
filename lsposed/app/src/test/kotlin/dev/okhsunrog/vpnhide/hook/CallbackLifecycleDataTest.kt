package dev.okhsunrog.vpnhide.hook

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CallbackLifecycleDataTest {
    @Test
    fun `cover changes announce available before properties`() {
        assertEquals(CallbackTransition(101, CallbackDelivery.Available), transitionCallback(100, 101, CallbackEventKind.Changed))
        assertEquals(CallbackTransition(101, CallbackDelivery.Forward), transitionCallback(101, 101, CallbackEventKind.Changed))
    }

    @Test
    fun `offline loses the previously delivered handle only once`() {
        assertEquals(CallbackTransition(null, CallbackDelivery.Lost, 100), transitionCallback(100, null, CallbackEventKind.Lost))
        assertEquals(CallbackDelivery.Suppress, transitionCallback(null, null, CallbackEventKind.Lost).delivery)
        assertEquals(CallbackDelivery.Available, transitionCallback(null, 101, CallbackEventKind.Changed).delivery)
    }

    @Test
    fun `underlying loss without VPN teardown still reports offline`() {
        assertEquals(CallbackDelivery.Lost, transitionCallback(100, null, CallbackEventKind.Changed).delivery)
    }

    @Test
    fun `VPN restart does not lose or duplicate a surviving cover`() {
        assertEquals(CallbackDelivery.Suppress, transitionCallback(100, 100, CallbackEventKind.Lost).delivery)
        assertEquals(CallbackDelivery.Suppress, transitionCallback(100, 100, CallbackEventKind.Available).delivery)
    }

    @Test
    fun `separate registrations retain independent delivered handles`() {
        val first = transitionCallback(null, 100, CallbackEventKind.Available)
        val second = transitionCallback(null, 101, CallbackEventKind.Available)
        assertEquals(100, transitionCallback(first.held, null, CallbackEventKind.Lost).network)
        assertEquals(101, transitionCallback(second.held, null, CallbackEventKind.Lost).network)
    }

    @Test
    fun `listen for best needs lifecycle tracking rather than silent suppression`() {
        assertTrue(isPassiveNetworkRequest("LISTEN"))
        assertFalse(isPassiveNetworkRequest("LISTEN_FOR_BEST"))
        assertFalse(isPassiveNetworkRequest("TRACK_DEFAULT"))
    }
}
