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
        assertTrue(command.contains("pm list users"))
        assertTrue(command.contains("pm list packages -U -f --user \"${'$'}PM_USER_ID\""))
        assertTrue(command.contains("grep -H . /sys/class/net/*/operstate"))
        // Paths reach the script through the assignment prelude now, so both
        // halves are checked: the value Kotlin passes, and the script using it.
        assertTrue(command.contains("VPNHIDE_SUPERKEY_FILE='$SUPERKEY_FILE'"))
        assertTrue(command.contains("VPNHIDE_KMOD_ACTIVATOR='$KMOD_ACTIVATOR'"))
        assertTrue(command.contains("VPNHIDE_KPM_ACTIVATOR='$KPM_ACTIVATOR'"))
        assertTrue(command.contains("VPNHIDE_ZYGISK_ACTIVATOR='$ZYGISK_ACTIVATOR'"))
        assertTrue(command.contains("VPNHIDE_PORTS_ACTIVATOR='$PORTS_ACTIVATOR'"))
        assertTrue(command.contains("VPNHIDE_PROC_CTL='$PROC_CTL'"))
        assertTrue(command.contains("[ -s ${'$'}VPNHIDE_SUPERKEY_FILE ] && echo 1 || echo 0"))
        assertTrue(command.contains("activator_state ${'$'}VPNHIDE_KMOD_ACTIVATOR"))
        assertTrue(command.contains("activator_state ${'$'}VPNHIDE_KPM_ACTIVATOR"))
        assertTrue(command.contains("activator_state ${'$'}VPNHIDE_ZYGISK_ACTIVATOR"))
        assertTrue(command.contains("activator_state ${'$'}VPNHIDE_PORTS_ACTIVATOR"))
        assertTrue(command.contains("[ -f ${'$'}VPNHIDE_KMOD_DIR/disable ] && echo 1 || echo 0"))
        assertTrue(command.contains("cat ${'$'}VPNHIDE_PROC_CTL"))
        assertTrue(command.contains("${'$'}VPNHIDE_KPM_ACTIVATOR state"))
        assertTrue(command.contains("kpm_runtime_modules"))
        assertTrue(command.contains(ZYGISK_STATUS_FILE))
        assertTrue(command.contains(PORTS_LOAD_STATUS_FILE))
        assertTrue(command.contains("probe_ok=1"))
        assertTrue(command.contains("iptables -C OUTPUT -j vpnhide_out"))
        assertTrue(command.contains("ip6tables -C OUTPUT -j vpnhide_out6"))
        assertFalse(command.contains("--user all"))
        assertFalse(command.contains("while IFS= read"))
        assertFalse(command.contains("base64"))
        assertFalse(command.contains("date +%s%3N"))
        assertFalse(command.contains("exit 0"))
        assertFalse(command.contains("run_phase"))
        assertFalse(command.contains("TMP_DIR"))
        assertFalse(command.contains("debug_logging"))
    }

    @Test
    fun `snapshot stages APatch runtime probe only from validated app path`() {
        val command = buildRootShellSnapshotCommand(runtimeProbeSource = "/data/user/0/dev.okhsunrog.vpnhide/files/vhprobe")

        assertTrue(command.contains("VPNHIDE_KPM_PROBE_SOURCE='/data/user/0/dev.okhsunrog.vpnhide/files/vhprobe'"))
        assertTrue(command.contains("--apatch-kpm-list"))
        assertTrue(command.contains("\"${'$'}KPATCH\" kpm list"))
        assertTrue(command.contains("rm -f \"${'$'}KPM_PROBE\""))
    }

    @Test
    fun `snapshot command can skip package enumeration when startup seeded it`() {
        val command = buildRootShellSnapshotCommand(includePmPackages = false)

        // The inventory is a function in the shared script now, so it is always
        // defined; what changes is whether the phase calls it.
        assertTrue(command.contains("VPNHIDE_WITH_PM='0'"))
        assertTrue(command.contains("if [ \"${'$'}VPNHIDE_WITH_PM\" = 1 ]; then"))
        assertTrue(buildRootShellSnapshotCommand(includePmPackages = true).contains("VPNHIDE_WITH_PM='1'"))
        assertTrue(command.contains("phase_target_files"))
        assertTrue(command.contains("phase_vpn_ifaces"))
    }

    @Test
    fun `snapshot command remains valid POSIX shell with runtime probe`() {
        val process = ProcessBuilder("sh", "-c", buildRootShellSnapshotCommand(includePmPackages = false)).start()
        val stdout = process.inputStream.bufferedReader().readText()
        val stderr = process.errorStream.bufferedReader().readText()

        assertEquals("shell stderr: $stderr", 0, process.waitFor())
        val sections = parseRootShellSnapshot(stdout, recordMetric = { _, _ -> })
        assertEquals("available=0", sections["kpm_runtime_modules"])
    }

    // Anti-drift guard: the detectors and the debug export read sections by name
    // from this one snapshot. If a required section stops being emitted, module
    // liveness silently reads wrong (this is exactly how the old debug bundle showed
    // a healthy kmod as "inactive"). Pin that every required section is produced.
    @Test
    fun `the snapshot command emits every required section`() {
        val command = buildRootShellSnapshotCommand()
        val missing = REQUIRED_ROOT_SNAPSHOT_SECTIONS.filterNot { command.contains(it) }
        assertTrue("snapshot command is missing emits for: $missing", missing.isEmpty())
    }

    @Test
    fun `the snapshot command probes the shell identity for the honest-render gate`() {
        val command = buildRootShellSnapshotCommand()
        assertTrue(command.contains("snapshot_shell_uid"))
        assertTrue("captures the effective uid", command.contains("id -u"))
    }
}
