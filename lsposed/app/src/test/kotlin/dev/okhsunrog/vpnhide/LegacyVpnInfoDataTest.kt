package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.LegacyVpnInfoSnapshot
import dev.okhsunrog.vpnhide.diagnostics.assessLegacyVpnInfo
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class LegacyVpnInfoDataTest {
    private val absent = LegacyVpnInfoSnapshot(17, "DISCONNECTED", "DISCONNECTED", true)
    private val blocked = absent.copy(detailedState = "BLOCKED")
    private val wifi = LegacyVpnInfoSnapshot(1, "CONNECTED", "CONNECTED", true)

    @Test
    fun `normal and policy blocked VPN entries pass without requiring equal sequential snapshots`() {
        for (direct in listOf(absent, blocked)) {
            for (enumerated in listOf(absent, blocked)) {
                assertEquals(true, assessLegacyVpnInfo(direct, listOf(wifi, enumerated)).clean)
            }
        }
    }

    @Test
    fun `old null suppression is inconclusive instead of a green pass`() {
        assertNull(assessLegacyVpnInfo(null, listOf(wifi)).clean)
        assertNull(assessLegacyVpnInfo(null, listOf(wifi, absent)).clean)
    }

    @Test
    fun `disconnected wifi and mobile substitutions fail`() {
        for (type in listOf(0, 1)) {
            assertEquals(false, assessLegacyVpnInfo(absent.copy(type = type), listOf(wifi, absent)).clean)
        }
    }

    @Test
    fun `active and transitional states fail on either path`() {
        for (state in listOf("CONNECTED", "CONNECTING", "SUSPENDED", "DISCONNECTING", "UNKNOWN")) {
            val active = absent.copy(state = state, detailedState = state)
            assertEquals(false, assessLegacyVpnInfo(active, listOf(wifi, absent)).clean)
            assertEquals(false, assessLegacyVpnInfo(absent, listOf(wifi, active)).clean)
            assertEquals(false, assessLegacyVpnInfo(null, listOf(wifi, active)).clean)
        }
    }

    @Test
    fun `mismatched detailed state and missing or duplicate VPN entries fail`() {
        assertEquals(false, assessLegacyVpnInfo(absent.copy(detailedState = "CONNECTED"), listOf(absent)).clean)
        assertEquals(false, assessLegacyVpnInfo(absent, listOf(wifi)).clean)
        assertEquals(false, assessLegacyVpnInfo(absent, listOf(wifi, absent, absent)).clean)
    }
}
