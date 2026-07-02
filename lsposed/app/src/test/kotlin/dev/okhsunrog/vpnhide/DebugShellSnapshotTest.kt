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
    fun `parser ignores unclosed partial section`() {
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
        assertFalse(sections.containsKey("partial"))
    }

    @Test
    fun `debug command captures all backend state paths`() {
        val command = buildDebugShellSnapshotCommand()

        assertTrue(command.contains("$KPM_ACTIVATOR state"))
        assertTrue(command.contains("$KPM_LOAD_STATUS_FILE"))
        assertTrue(command.contains("$KPM_MODULE_DIR/vpnhide.kpm"))
        assertTrue(command.contains("$KMOD_LOAD_STATUS_FILE"))
        assertTrue(command.contains("cat $PROC_CTL"))
        assertTrue(command.contains(ZYGISK_STATUS_FILE))
        assertTrue(command.contains("$ZYGISK_MODULE_DIR/zygisk/arm64-v8a.so"))
        assertTrue(command.contains("$PORTS_MODULE_DIR/module.prop"))
        assertTrue(command.contains(PORTS_LOAD_STATUS_FILE))
        assertTrue(command.contains(PORTS_LOAD_LOG_FILE))
        assertTrue(command.contains("iptables -S vpnhide_out"))
        assertTrue(command.contains("ip6tables -S vpnhide_out6"))
        assertTrue(command.contains(LSPOSED_STATE_FILE))
        assertTrue(command.contains("/data/adb/lspd/config/modules_config.db"))
        assertTrue(command.contains("dumpsys connectivity"))
        assertTrue(command.contains("androidboot[.]serialno"))
        assertTrue(command.contains("pm list packages -U --user all"))
        assertFalse(command.contains("debug_logging"))
    }

    @Test
    fun `counter command captures only hook status and package uid map`() {
        val command = buildHookCounterSnapshotCommand()

        assertTrue(command.contains("pm list packages -U --user all"))
        assertTrue(command.contains("cat $PROC_CTL"))
        assertTrue(command.contains("$KPM_ACTIVATOR state"))
        assertTrue(command.contains(LSPOSED_STATE_FILE))
        assertTrue(command.contains(ZYGISK_STATUS_FILE))
        assertFalse(command.contains("dumpsys connectivity"))
        assertFalse(command.contains("/proc/net/fib_trie"))
    }
}
