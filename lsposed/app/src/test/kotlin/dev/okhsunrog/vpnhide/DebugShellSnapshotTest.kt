package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.core.CoreProbeResult
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class DebugShellSnapshotTest {
    @Test
    fun `parser keeps multiline sections`() {
        val raw =
            """
            __VPNHIDE_DEBUG_SECTION_BEGIN__:kpm_state
            vpnhide 1 status
            backend 0x1
            __VPNHIDE_DEBUG_SECTION_END__:kpm_state
            __VPNHIDE_DEBUG_SECTION_BEGIN__:empty
            __VPNHIDE_DEBUG_SECTION_END__:empty
            """.trimIndent()

        val sections = parseDebugShellSnapshot(raw)

        assertEquals("vpnhide 1 status\nbackend 0x1", sections["kpm_state"])
        assertEquals("", sections["empty"])
    }

    @Test
    fun `parser keeps an unclosed partial section but flags it truncated`() {
        val raw =
            """
            __VPNHIDE_DEBUG_SECTION_BEGIN__:complete
            ok
            __VPNHIDE_DEBUG_SECTION_END__:complete
            __VPNHIDE_DEBUG_SECTION_BEGIN__:partial
            lost
            """.trimIndent()

        val sections = parseDebugShellSnapshot(raw)

        assertEquals("ok", sections["complete"])
        // The partial (timeout-truncated) section is preserved and flagged rather
        // than silently dropped, so a bug report shows where the snapshot cut off.
        assertTrue(sections.containsKey("partial"))
        assertTrue(sections["partial"].orEmpty().contains("lost"))
        assertTrue(sections["partial"].orEmpty().contains("TRUNCATED"))
        assertEquals("partial", sections["debug_snapshot_truncated"])
    }

    @Test
    fun `parser ignores a mismatched end marker while collecting diagnostics`() {
        val raw =
            """
            __VPNHIDE_DEBUG_SECTION_BEGIN__:expected
            first
            __VPNHIDE_DEBUG_SECTION_END__:other
            second
            __VPNHIDE_DEBUG_SECTION_END__:expected
            """.trimIndent()

        assertEquals("first\nsecond", parseDebugShellSnapshot(raw)["expected"])
    }

    @Test
    fun `framing failure retains partial data and parent probe failure independently`() {
        val raw =
            "__VPNHIDE_DEBUG_SECTION_BEGIN__:partial\nkept\n" +
                "__VPNHIDE_DEBUG_SECTION_BEGIN__:${"x".repeat(10_000)}"
        val result = snapshotFrom(CoreProbeResult(exit = 7, text = raw), "debug")
        assertEquals(7, result.exitCode)
        assertTrue(result.sections["partial"].orEmpty().contains("kept"))
        assertEquals("partial", result.sections["debug_snapshot_truncated"])
        assertTrue(result.sections["debug_snapshot_error"].orEmpty().contains("NAME_TOO_LONG"))
        assertTrue(result.sections["debug_snapshot_error"].orEmpty().contains("exit=7"))
    }

    @Test
    fun `native decoder matches JVM whitespace semantics over valid BMP scalars`() {
        val scalars = ('\u0000'..'\uffff').filter { it !in '\ud800'..'\udfff' }
        val preserved = scalars.filterNot(Char::isWhitespace).joinToString("")
        val trailing = scalars.filter(Char::isWhitespace).joinToString("")
        val raw = "__VPNHIDE_DEBUG_SECTION_BEGIN__:x\n$preserved$trailing\n__VPNHIDE_DEBUG_SECTION_END__:x"
        assertEquals(preserved, parseDebugShellSnapshot(raw)["x"])
    }
}
