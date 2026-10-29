package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.migration.parseImportedCanonicalConfig
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Test

class CanonicalImportDataTest {
    @Test
    fun `shared v1 fixture imports without intermediate install`() {
        val raw = javaClass.getResource("/canonical-v1.json")!!.readText()
        val config = parseImportedCanonicalConfig(raw, "dev.test.self")!!
        assertEquals(true, config.apps.getValue("com.bank.app").ports)
        assertEquals(config, parseImportedCanonicalConfig(canonicalConfigJson(config), "dev.test.self"))
    }

    @Test
    fun `corrupt and future data never becomes an empty imported policy`() {
        listOf(
            "null",
            "[]",
            "{",
            "{\"apps\":[]}",
            "{\"version\":4294967297}",
            "{\"version\":2}",
            "{\"version\":\"1\"}",
            "{\"apps\":{\"com.bank\":{\"native\":42}}}",
            "{\"settings\":{\"autoHideVpnName\":\"true\"}}",
        ).forEach {
            assertNull(it, parseImportedCanonicalConfig(it, "dev.test.self"))
        }
    }

    @Test
    fun `v1 polymorphic roles import directly and current roundtrip is idempotent`() {
        val imported =
            parseImportedCanonicalConfig(
                """{"version":1,"apps":{"com.bank":{"java":["hook"],"native":{"kernel":["hook"],"zygisk":null},"ports":true}}}""",
                "dev.test.self",
            )
        assertNotNull(imported)
        assertEquals(imported, parseImportedCanonicalConfig(canonicalConfigJson(imported!!), "dev.test.self"))
        assertEquals(
            true,
            imported.apps
                .getValue("com.bank")
                .native.enabled,
        )
    }
}
