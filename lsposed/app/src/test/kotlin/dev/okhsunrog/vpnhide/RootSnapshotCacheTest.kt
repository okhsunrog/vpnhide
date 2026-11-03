package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.vpnhide_core_ffi.coreRequiredRootSnapshotSections

class RootSnapshotCacheTest {
    @Test
    fun `parser keeps multiline sections and records timing metrics`() {
        val raw =
            """
            __VPNHIDE_ROOT_SECTION_BEGIN__:canonical_config
            # Managed by VPN Hide app
            dev.okhsunrog.vpnhide
            com.example.target
            __VPNHIDE_ROOT_SECTION_END__:canonical_config
            __VPNHIDE_ROOT_TIMING__:target_files=42
            __VPNHIDE_ROOT_SECTION_BEGIN__:empty_file
            __VPNHIDE_ROOT_SECTION_END__:empty_file
            """.trimIndent()

        val metrics = mutableMapOf<String, Long>()
        val sections =
            parseRootShellSnapshot(raw) { name, durationMs ->
                metrics[name] = durationMs
            }

        assertEquals("# Managed by VPN Hide app\ndev.okhsunrog.vpnhide\ncom.example.target", sections["canonical_config"])
        assertEquals("", sections["empty_file"])
        assertEquals(42L, metrics["root_shell_target_files"])
    }

    @Test
    fun `parser ignores unclosed partial section`() {
        val raw =
            """
            __VPNHIDE_ROOT_SECTION_BEGIN__:complete
            ok
            __VPNHIDE_ROOT_SECTION_END__:complete
            __VPNHIDE_ROOT_SECTION_BEGIN__:partial
            incomplete
            """.trimIndent()

        val sections = parseRootShellSnapshot(raw, recordMetric = { _, _ -> })

        assertEquals("ok", sections["complete"])
        assertFalse(sections.containsKey("partial"))
    }

    @Test
    fun `parser discards a section after a mismatched end marker`() {
        val raw =
            """
            __VPNHIDE_ROOT_SECTION_BEGIN__:expected
            unsafe partial body
            __VPNHIDE_ROOT_SECTION_END__:other
            __VPNHIDE_ROOT_SECTION_END__:expected
            """.trimIndent()

        assertFalse(parseRootShellSnapshot(raw, recordMetric = { _, _ -> }).containsKey("expected"))
    }

    @Test
    fun `parser recovers content glued onto the end marker without a trailing newline`() {
        // A file emitted without a trailing newline (e.g. a hand-edited config
        // read by the snapshot probe) puts its last line and the END marker on
        // the same line. The section must still parse, not be dropped as unclosed.
        val raw =
            "__VPNHIDE_ROOT_SECTION_BEGIN__:canonical_config\n" +
                "{\"version\":1,\"debug\":true}__VPNHIDE_ROOT_SECTION_END__:canonical_config\n"

        val sections = parseRootShellSnapshot(raw, recordMetric = { _, _ -> })

        assertEquals("{\"version\":1,\"debug\":true}", sections["canonical_config"])
    }

    @Test
    fun `parser recovers a multiline body whose last line is glued to the end marker`() {
        val raw =
            "__VPNHIDE_ROOT_SECTION_BEGIN__:canonical_config\n" +
                "line1\n" +
                "line2__VPNHIDE_ROOT_SECTION_END__:canonical_config\n"

        val sections = parseRootShellSnapshot(raw, recordMetric = { _, _ -> })

        assertEquals("line1\nline2", sections["canonical_config"])
    }

    @Test
    fun `snapshot validation rejects missing sections`() {
        val sections = coreRequiredRootSnapshotSections().associateWith { "" } - "kmod_prop"
        var thrown: RootSnapshotException? = null

        try {
            validateRootSnapshotSections(sections)
        } catch (e: RootSnapshotException) {
            thrown = e
        }

        assertTrue(thrown?.message?.contains("kmod_prop") == true)
    }

    @Test
    fun `complete required keys cannot mask invalid framing names`() {
        val sections = coreRequiredRootSnapshotSections().associateWith { "" } + ("x".repeat(10_000) to "")
        val error = runCatching { validateRootSnapshotSections(sections) }.exceptionOrNull()
        assertTrue(error is RootSnapshotException)
        assertEquals(SelfTargetFailureKind.IncompleteData, (error as RootSnapshotException).kind)
        assertTrue(error.message.orEmpty().contains("NAME_TOO_LONG"))
    }
}
