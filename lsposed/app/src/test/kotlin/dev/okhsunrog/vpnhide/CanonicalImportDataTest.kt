package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.migration.parseImportedCanonicalConfig
import dev.okhsunrog.vpnhide.migration.readCanonicalImportText
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Test
import java.io.ByteArrayInputStream

class CanonicalImportDataTest {
    @Test
    fun `generated native import bridge preserves the shared fixture and self policy`() {
        val raw = javaClass.getResource("/canonical-v1.json")!!.readText()
        val config = parseImportedCanonicalConfig(raw, "dev.test.self")!!
        assertEquals(true, config.apps.getValue("com.bank.app").ports)
        assertEquals(NativeRole.All, config.apps.getValue("dev.test.self").native)
        assertEquals(config, parseImportedCanonicalConfig(canonicalConfigJson(config), "dev.test.self"))
        assertNull(parseImportedCanonicalConfig("null", "dev.test.self"))
    }

    @Test
    fun `SAF adapter bounds bytes using the shared core limit`() {
        val limit = MAX_CORE_CONFIG_BYTES
        assertNotNull(readCanonicalImportText(ByteArrayInputStream(ByteArray(limit))))
        val oversized = ByteArrayInputStream(ByteArray(limit * 3))
        assertNull(readCanonicalImportText(oversized))
        assertEquals(limit * 2 - 1, oversized.available())
    }
}
