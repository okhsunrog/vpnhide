package dev.okhsunrog.vpnhide.hook

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CallbackPayloadHistoryTest {
    @Test
    fun `same cover suppresses replay but retains changed properties`() {
        val history = CallbackPayloadHistory()
        assertTrue(history.available(100))
        assertTrue(history.changed(1, "wifi capabilities"))
        assertTrue(history.changed(2, "wifi properties"))
        assertFalse(history.available(100))
        assertFalse(history.changed(1, "wifi capabilities"))
        assertFalse(history.changed(2, "wifi properties"))
        assertTrue(history.changed(2, "new DNS"))
    }

    @Test
    fun `new network and new registrations replay initial payloads`() {
        val first = CallbackPayloadHistory()
        first.available(100)
        first.changed(1, "capabilities")
        assertTrue(first.available(101))
        assertTrue(first.changed(1, "capabilities"))
        val second = CallbackPayloadHistory()
        assertTrue(second.available(101))
        assertTrue(second.changed(1, "capabilities"))
        assertFalse(first.changed(1, "capabilities"))
    }
}
