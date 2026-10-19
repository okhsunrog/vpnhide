package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
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
    fun `debug probe reads the backend state paths the app shares`() {
        // Section coverage is asserted on the probe's real output by core's
        // `debug_snapshot_covers_every_backend_state_path`; here pin the shared paths.
        val paths = coreProbeSource("mod.rs")
        assertTrue(paths.contains("\"$KPM_LOAD_STATUS_FILE\""))
        assertTrue(paths.contains("\"$KMOD_LOAD_STATUS_FILE\""))
        assertTrue(paths.contains("\"$PROC_CTL\""))
        assertTrue(paths.contains(ZYGISK_STATUS_FILE))
        assertTrue(paths.contains(LSPOSED_STATE_FILE))
        val debug = coreProbeSource("debug_snapshot.rs")
        assertTrue(debug.contains("\"vpnhide_core\""))
        assertTrue(debug.contains("/data/adb/lspd/config"))
        assertTrue(debug.contains("\"dumpsys\"") && debug.contains("\"connectivity\""))
        assertTrue(debug.contains("androidboot.serialno="))
        assertFalse(debug.contains("debug_logging"))
        val inventory = coreProbeSource("inventory.rs")
        assertTrue(inventory.contains("[\"list\", \"packages\", \"-U\", \"-f\", \"--user\"]"))
        assertFalse(inventory.contains("--user all"))
    }

    @Test
    fun `counter probe captures hook status without enumerating packages`() {
        val debug = coreProbeSource("debug_snapshot.rs")
        val counters = debug.substringAfter("pub fn hook_counters(").substringBefore("\n}\n")
        assertTrue(counters.contains("\"kmod_state\""))
        assertTrue(counters.contains("kpm_state(&mut out, paths)"))
        assertTrue(counters.contains("\"lsposed_state\""))
        assertTrue(counters.contains("\"zygisk_status\""))
        // The counter baseline is consumed only through counts (backend+uid+hookId),
        // never package names, so it skips the expensive multi-user pm enumeration.
        assertFalse(counters.contains("inventory"))
        assertFalse(counters.contains("dumpsys"))
        assertFalse(counters.contains("fib_trie"))
    }
}
