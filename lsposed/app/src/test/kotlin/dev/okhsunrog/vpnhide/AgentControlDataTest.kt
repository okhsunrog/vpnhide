package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.fail
import org.junit.Test

class AgentControlDataTest {
    @Test
    fun `package name validation trims valid names`() {
        assertEquals("com.example.app", requirePackageName("  com.example.app  "))
    }

    @Test
    fun `package name validation rejects blanks and whitespace`() {
        expectIllegalArgument { requirePackageName("") }
        expectIllegalArgument { requirePackageName("com.example app") }
    }

    @Test
    fun `hook resolution deduplicates known hook ids`() {
        val first = LsposedJavaHookEntries.first().hookName

        assertEquals(listOf(first), resolveHookIds(listOf(first, first), LsposedJavaHookEntries))
    }

    @Test
    fun `hook resolution rejects unknown hook ids`() {
        expectIllegalArgument { resolveHookIds(listOf("missing_hook"), LsposedJavaHookEntries) }
    }

    @Test
    fun `native hook family accepts backend aliases`() {
        assertEquals(NativeHookFamily.Kernel, parseNativeHookFamily("kernel"))
        assertEquals(NativeHookFamily.Kernel, parseNativeHookFamily("kmod"))
        assertEquals(NativeHookFamily.Kernel, parseNativeHookFamily("kpm"))
        assertEquals(NativeHookFamily.Zygisk, parseNativeHookFamily("zygisk"))
    }

    @Test
    fun `native hook family rejects unknown values`() {
        expectIllegalArgument { parseNativeHookFamily("native") }
    }

    @Test
    fun `agent port policy parses all preset and custom modes`() {
        assertNull(parseAgentPortPolicy("all", preset = null, rules = emptyList()))

        val preset = parseAgentPortPolicy("preset", preset = PORT_PRESET_COMMON_PROXY, rules = emptyList())
        assertEquals(portPolicyForPreset(PORT_PRESET_COMMON_PROXY), preset)

        val custom =
            parseAgentPortPolicy(
                mode = "custom",
                preset = null,
                rules =
                    listOf(
                        AgentPortRule(protocol = "tcp", start = 7890, end = 7892),
                        AgentPortRule(protocol = "both", start = 1080, end = 1080),
                    ),
            )
        assertEquals(
            PortPolicy(
                mode = PortPolicyMode.Custom,
                rules =
                    listOf(
                        PortRule(start = 1080),
                        PortRule(protocol = PortProtocol.Tcp, start = 7890, end = 7892),
                    ),
            ),
            custom,
        )
    }

    @Test
    fun `agent port policy reports validation failures`() {
        assertEquals("Port preset is required", parseAgentPortPolicyResult("preset", null, emptyList()).errorMessage())
        assertEquals(
            "Unknown port preset: missing",
            parseAgentPortPolicyResult("preset", "missing", emptyList()).errorMessage(),
        )
        assertEquals(
            "Custom port policy requires at least one rule",
            parseAgentPortPolicyResult("custom", null, emptyList()).errorMessage(),
        )
    }

    private fun expectIllegalArgument(block: () -> Unit) {
        try {
            block()
            fail("Expected IllegalArgumentException")
        } catch (_: IllegalArgumentException) {
            // Expected.
        }
    }

    private fun Result<*>.errorMessage(): String? = exceptionOrNull()?.message
}
