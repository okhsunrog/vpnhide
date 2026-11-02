package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.LegacyInfoFields
import dev.okhsunrog.vpnhide.appendLegacyFixture
import dev.okhsunrog.vpnhide.legacyOrdinaryInfo
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

class LegacyFixtureDataTest {
    private fun info(
        type: Int,
        marker: String?,
    ) = LegacyInfoFields(type, 0, "type", "", "CONNECTED", "CONNECTED", true, false, false, null, marker)

    @Test
    fun `source appends dirty VPN and ordinary sentinel without changing original identities`() {
        val original = info(0, null)
        val vpn = info(17, "tunFixture")
        val ordinary = info(1, "ordinaryFixture")
        val result = requireNotNull(appendLegacyFixture(listOf(original, original), vpn, ordinary, "tunFixture", "ordinaryFixture") { it })
        assertEquals(4, result.infos.size)
        assertSame(original, result.infos[0])
        assertSame(original, result.infos[1])
        assertEquals("CONNECTED", result.infos[2].detailedState)
        assertEquals(3, result.ordinary.count)
        assertTrue(result.ordinary.fixturePresent)
        assertEquals(64, result.ordinary.sha256.length)
    }

    @Test
    fun `multiset control preserves duplicates and detects ordinary mutation independently of VPN cleaning`() {
        val ordinary = info(1, "ordinaryFixture")
        val other = info(0, null)
        val vpn = info(17, "tunFixture")
        val baseline = legacyOrdinaryInfo(listOf(ordinary, other, other, vpn), "ordinaryFixture")
        assertEquals(baseline, legacyOrdinaryInfo(listOf(other, vpn.copy(state = "DISCONNECTED"), ordinary, other), "ordinaryFixture"))
        assertNotEquals(baseline, legacyOrdinaryInfo(listOf(ordinary, other, vpn), "ordinaryFixture"))
        assertNotEquals(baseline, legacyOrdinaryInfo(listOf(ordinary, other, other.copy(available = false), vpn), "ordinaryFixture"))
        assertFalse(legacyOrdinaryInfo(listOf(other), "ordinaryFixture").fixturePresent)
    }

    @Test
    fun `null entries bound overflow and existing fixture markers prevent injection`() {
        val vpn = info(17, "tunFixture")
        val ordinary = info(1, "ordinaryFixture")
        for (original in listOf(listOf(null), List(255) { info(0, null) }, listOf(vpn), listOf(ordinary))) {
            assertNull(appendLegacyFixture(original, vpn, ordinary, "tunFixture", "ordinaryFixture") { it })
        }
        val empty = requireNotNull(appendLegacyFixture(emptyList(), vpn, ordinary, "tunFixture", "ordinaryFixture") { it })
        assertEquals(1, empty.ordinary.count)
        assertTrue(empty.ordinary.fixturePresent)
    }
}
