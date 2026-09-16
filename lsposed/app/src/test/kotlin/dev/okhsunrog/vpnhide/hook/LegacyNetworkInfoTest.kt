package dev.okhsunrog.vpnhide.hook

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Test

class LegacyNetworkInfoTest {
    private data class Info(
        val type: Int,
        val state: String,
    ) {
        val connectedOrConnecting get() = state == "CONNECTED" || state == "CONNECTING"
    }

    private fun normalize(
        original: Info?,
        requestedType: Int? = 17,
        absent: () -> Info = { Info(17, "DISCONNECTED") },
    ): Info? =
        normalizeLegacyVpnInfo(
            original,
            requestedType,
            Info::type,
            { it.state == "DISCONNECTED" || it.state == "BLOCKED" },
            absent,
        )

    @Test
    fun `both legacy detectors accept a hidden active VPN without renaming its type`() {
        for (type in listOf(0, 1, 17)) {
            for (state in listOf("CONNECTING", "CONNECTED", "SUSPENDED", "DISCONNECTING")) {
                val result = normalize(Info(type, state))
                // Old Ruru flags null OR connected; the other legacy detector flags connected.
                assertFalse(result == null || result.connectedOrConnecting)
                assertEquals(17, result!!.type)
                assertEquals("DISCONNECTED", result.state)
            }
        }
    }

    @Test
    fun `disconnected and policy blocked replies survive repeated sanitization by identity`() {
        for (state in listOf("DISCONNECTED", "BLOCKED")) {
            val original = Info(17, state)
            val direct = normalize(original) { error("must not recompute an inactive reply") }
            val enumerated = normalize(direct, requestedType = null) { error("must remain unchanged") }
            assertSame(original, direct)
            assertSame(original, enumerated)
        }
    }

    @Test
    fun `enumeration retains physical entries and exactly one disconnected VPN entry`() {
        val wifi = Info(1, "CONNECTED")
        val mobile = Info(0, "DISCONNECTED")
        val result = listOf(wifi, mobile, Info(17, "CONNECTED")).map { normalize(it, requestedType = null) }
        assertEquals(listOf(1, 0, 17), result.map { it!!.type })
        assertSame(wifi, result[0])
        assertSame(mobile, result[1])
        assertFalse(result[2]!!.connectedOrConnecting)
    }

    @Test
    fun `platform nulls and non VPN replies need no synthetic state`() {
        assertNull(normalize(null) { error("must preserve a platform null") })
        val wifi = Info(1, "CONNECTED")
        assertSame(wifi, normalize(wifi, requestedType = 1) { error("must preserve other types") })
    }

    @Test
    fun `platform UID policy controls the replacement state`() {
        val blocked = Info(17, "BLOCKED")
        val result = normalize(Info(17, "CONNECTED")) { blocked }
        assertSame(blocked, result)
        assertFalse(result!!.connectedOrConnecting)
    }
}
