package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

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
        // cat'd by the snapshot script) puts its last line and the END marker on
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
        val sections = REQUIRED_ROOT_SNAPSHOT_SECTIONS.associateWith { "" } - "kmod_prop"
        var thrown: RootSnapshotException? = null

        try {
            validateRootSnapshotSections(sections)
        } catch (e: RootSnapshotException) {
            thrown = e
        }

        assertTrue(thrown?.message?.contains("kmod_prop") == true)
    }

    // Anti-drift guard: the detectors and the debug export read sections by name from
    // this one snapshot. Core's `root_snapshot_emits_every_section_the_app_requires` test
    // runs the Rust probe and checks its output against REQUIRED_ROOT_SNAPSHOT_SECTIONS;
    // here, pin that the probe still reports the shell identity the honest-render gate needs.
    @Test
    fun `the snapshot probe reports the probe identity for the honest-render gate`() {
        val source = coreProbeSource("root_snapshot.rs")
        assertTrue(source.contains("\"snapshot_shell_uid\""))
        assertTrue("captures the effective uid", source.contains("sys::uid()"))
    }
}
