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
            __VPNHIDE_ROOT_SECTION_BEGIN__:kmod_targets
            # Managed by VPN Hide app
            dev.okhsunrog.vpnhide
            com.example.target
            __VPNHIDE_ROOT_SECTION_END__:kmod_targets
            __VPNHIDE_ROOT_TIMING__:target_files=42
            __VPNHIDE_ROOT_SECTION_BEGIN__:empty_file
            __VPNHIDE_ROOT_SECTION_END__:empty_file
            """.trimIndent()

        val metrics = mutableMapOf<String, Long>()
        val sections =
            parseRootShellSnapshot(raw) { name, durationMs ->
                metrics[name] = durationMs
            }

        assertEquals("# Managed by VPN Hide app\ndev.okhsunrog.vpnhide\ncom.example.target", sections["kmod_targets"])
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
    fun `snapshot validation rejects missing sections`() {
        val sections = REQUIRED_ROOT_SNAPSHOT_SECTIONS.associateWith { "" } - "vpn_ifaces"
        var thrown: RootSnapshotException? = null

        try {
            validateRootSnapshotSections(sections)
        } catch (e: RootSnapshotException) {
            thrown = e
        }

        assertTrue(thrown?.message?.contains("vpn_ifaces") == true)
    }

    @Test
    fun `snapshot command avoids per-section base64 and external date timing`() {
        val command = buildRootShellSnapshotCommand()

        assertTrue(command.contains("__VPNHIDE_ROOT_SECTION_BEGIN__:"))
        assertTrue(command.contains("__VPNHIDE_ROOT_SECTION_END__:"))
        assertTrue(command.contains("EPOCHREALTIME"))
        assertTrue(command.contains("/proc/uptime"))
        assertTrue(command.contains("cat \"${'$'}PATH_TO_READ\""))
        assertTrue(command.contains("pm list packages -U --user all"))
        assertTrue(command.contains("grep -H . /sys/class/net/*/operstate"))
        assertTrue(command.contains("[ -s $SUPERKEY_FILE ] && echo 1 || echo 0"))
        assertTrue(command.contains("cat $PROC_CTL"))
        assertTrue(command.contains("$KPM_ACTIVATOR state"))
        assertTrue(command.contains(ZYGISK_STATUS_FILE))
        assertTrue(command.contains(PORTS_LOAD_STATUS_FILE))
        assertTrue(command.contains("probe_ok=1"))
        assertTrue(command.contains("iptables -C OUTPUT -j vpnhide_out"))
        assertTrue(command.contains("ip6tables -C OUTPUT -j vpnhide_out6"))
        assertFalse(command.contains("while IFS= read"))
        assertFalse(command.contains("base64"))
        assertFalse(command.contains("date +%s%3N"))
        assertFalse(command.contains("exit 0"))
        assertFalse(command.contains("run_phase"))
        assertFalse(command.contains("TMP_DIR"))
        assertFalse(command.contains("debug_logging"))
    }

    @Test
    fun `snapshot command can skip package enumeration when startup seeded it`() {
        val command = buildRootShellSnapshotCommand(includePmPackages = false)

        assertFalse(command.contains("pm list packages -U --user all"))
        assertTrue(command.contains("phase_target_files"))
        assertTrue(command.contains("phase_vpn_ifaces"))
    }
}
