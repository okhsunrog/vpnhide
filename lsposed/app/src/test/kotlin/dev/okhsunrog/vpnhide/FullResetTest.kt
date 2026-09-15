package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class FullResetTest {
    private val installed = ModuleState.Installed(version = "1.0", active = false)
    private val activeLsposed = LsposedState.Active(version = "1.0", targetCount = 0)

    @Test
    fun `reset preserves mutation lock inode and removes ordinary and hidden state`() {
        val root =
            kotlin.io.path
                .createTempDirectory("reset-state")
                .toFile()
        try {
            val state = root.resolve("data/adb/vpnhide/app-state").apply { mkdirs() }
            val lock = state.resolve("lock").apply { writeText("held") }
            val inode =
                java.nio.file.Files
                    .getAttribute(lock.toPath(), "unix:ino")
            root.resolve("data/adb/vpnhide/superkey").writeText("private")
            root.resolve("data/adb/vpnhide/.old").writeText("stale")
            root.resolve("data/adb/vpnhide/..old").writeText("stale")
            root.resolve("data/system").mkdirs()
            root.resolve("data/system/vpnhide_config.json").writeText("{}")
            val command = buildFullResetCommand().replace("/data/", "${root.absolutePath}/data/")
            repeat(2) {
                val process = ProcessBuilder("sh", "-c", command).redirectErrorStream(true).start()
                val output = process.inputStream.bufferedReader().readText()
                assertEquals(output, 0, process.waitFor())
            }
            assertEquals(
                inode,
                java.nio.file.Files
                    .getAttribute(lock.toPath(), "unix:ino"),
            )
            assertEquals(listOf("app-state"), requireNotNull(state.parentFile).list()?.toList())
            assertFalse(root.resolve("data/system/vpnhide_config.json").exists())
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun `reset command removes every leftover service path`() {
        val cmd = buildFullResetCommand()
        // Canonical config, current status files, and backend-owned state dirs.
        assertTrue(cmd.contains(CANONICAL_CONFIG_FILE))
        assertTrue(cmd.contains(LSPOSED_STATE_FILE))
        assertTrue(cmd.contains(LEGACY_HOOK_STATUS_FILE))
        assertTrue(cmd.contains("/data/adb/vpnhide/app-state) ;;"))
        listOf("kmod", "kpm", "ports").forEach {
            assertTrue("missing /data/adb/vpnhide_$it", cmd.contains("rm -rf /data/adb/vpnhide_$it"))
        }
        // Pre-1.0 lists too: leaving them behind is what "leftover files" means,
        // and a reset device would otherwise come back offering to import the
        // config it just wiped (LegacyConfigImport).
        listOf("vpnhide_uids.txt", "vpnhide_hidden_pkgs.txt", "vpnhide_observer_uids.txt").forEach {
            assertTrue("pre-1.0 path not cleaned: $it", cmd.contains(it))
        }
        listOf("vpnhide_zygisk", "vpnhide_lsposed").forEach {
            assertTrue("missing /data/adb/$it", cmd.contains("rm -rf /data/adb/$it"))
        }
    }

    @Test
    fun `reset command never touches module dirs, iptables, or proc`() {
        val cmd = buildFullResetCommand()
        assertFalse(cmd.contains("/data/adb/modules"))
        assertFalse(cmd.contains("iptables"))
        assertFalse(cmd.contains("ip6tables"))
        assertFalse(cmd.contains("/proc/"))
    }

    @Test
    fun `ready when nothing is installed or active`() {
        assertEquals(
            emptyList<ResetBlocker>(),
            resetBlockers(
                kmod = ModuleState.NotInstalled,
                kpm = ModuleState.NotInstalled,
                zygisk = ModuleState.NotInstalled,
                ports = ModuleState.NotInstalled,
                lsposed = LsposedState.NotInstalled,
                kernelCtlPresent = false,
            ),
        )
    }

    @Test
    fun `each remaining backend or hook is a blocker`() {
        assertEquals(
            listOf(
                ResetBlocker.KmodInstalled,
                ResetBlocker.KpmInstalled,
                ResetBlocker.ZygiskInstalled,
                ResetBlocker.PortsInstalled,
                ResetBlocker.KernelStillHooked,
                ResetBlocker.LsposedActive,
            ),
            resetBlockers(
                kmod = installed,
                kpm = installed,
                zygisk = installed,
                ports = installed,
                lsposed = activeLsposed,
                kernelCtlPresent = true,
            ),
        )
    }

    @Test
    fun `a disabled-but-present module still blocks`() {
        // module dir present (Installed, inactive) -> user must remove it, not
        // just disable it.
        assertEquals(
            listOf(ResetBlocker.KmodInstalled),
            resetBlockers(
                kmod = installed,
                kpm = ModuleState.NotInstalled,
                zygisk = ModuleState.NotInstalled,
                ports = ModuleState.NotInstalled,
                lsposed = LsposedState.NotInstalled,
                kernelCtlPresent = false,
            ),
        )
    }

    @Test
    fun `loaded ko with no module dir still blocks via proc`() {
        // Removed the module but didn't reboot: /proc/vpnhide_ctl still exists.
        assertEquals(
            listOf(ResetBlocker.KernelStillHooked),
            resetBlockers(
                kmod = ModuleState.NotInstalled,
                kpm = ModuleState.NotInstalled,
                zygisk = ModuleState.NotInstalled,
                ports = ModuleState.NotInstalled,
                lsposed = LsposedState.NotInstalled,
                kernelCtlPresent = true,
            ),
        )
    }

    @Test
    fun `inactive lsposed module does not block`() {
        // disabled hook (InstalledInactive) is fine — only an active hook blocks.
        assertEquals(
            emptyList<ResetBlocker>(),
            resetBlockers(
                kmod = ModuleState.NotInstalled,
                kpm = ModuleState.NotInstalled,
                zygisk = ModuleState.NotInstalled,
                ports = ModuleState.NotInstalled,
                lsposed = LsposedState.InstalledInactive(version = "1.0"),
                kernelCtlPresent = false,
            ),
        )
    }
}
