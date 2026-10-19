package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class DashboardParsersTest {
    @Test
    fun `parseModuleProp returns not-installed for blank input`() {
        assertEquals(ModulePropInfo(false, null, null), parseModuleProp(""))
        assertEquals(ModulePropInfo(false, null, null), parseModuleProp("   \n  "))
    }

    @Test
    fun `parseModuleProp strips v prefix and reads gki variant`() {
        val raw =
            """
            id=vpnhide_kmod
            version=v0.6.3
            gkiVariant=android13-5.10
            """.trimIndent()
        assertEquals(ModulePropInfo(true, "0.6.3", "android13-5.10"), parseModuleProp(raw))
    }

    @Test
    fun `parseModuleProp recovers variant from updateJson when not stamped`() {
        val raw =
            """
            version=0.6.2
            updateJson=https://example.com/update-kmod-android14-6.1.json
            """.trimIndent()
        assertEquals(ModulePropInfo(true, "0.6.2", "android14-6.1"), parseModuleProp(raw))
    }

    @Test
    fun `parseModuleProp prefers stamped gki variant over updateJson`() {
        val raw =
            """
            version=0.6.3
            gkiVariant=android13-5.10
            updateJson=https://example.com/update-kmod-android14-6.1.json
            """.trimIndent()
        assertEquals("android13-5.10", parseModuleProp(raw).gkiVariant)
    }

    @Test
    fun `resolveLsposedState active heartbeat wins`() {
        val state =
            resolveLsposedState(
                hooksActiveThisBoot = true,
                hookVersion = "0.6.3",
                lsposedTargetCount = 4,
                framework = LsposedFramework.NotInstalled,
                config = null,
            )
        assertEquals(LsposedState.Active("0.6.3", 4), state)
    }

    @Test
    fun `resolveLsposedState not configured maps to framework presence`() {
        assertEquals(
            LsposedState.NotInstalled,
            resolveLsposedState(false, null, 0, LsposedFramework.NotInstalled, LsposedConfig.ModuleNotConfigured),
        )
        assertEquals(
            LsposedState.InstalledInactive(null),
            resolveLsposedState(false, null, 0, LsposedFramework.Installed(disabled = false), LsposedConfig.ModuleNotConfigured),
        )
    }

    @Test
    fun `resolveLsposedState enabled with system scope needs reboot`() {
        val enabled = LsposedConfig.Enabled(entries = listOf("system"), hasSystemFramework = true, extraEntries = emptyList())
        assertEquals(
            LsposedState.NeedsReboot("0.6.3"),
            resolveLsposedState(false, "0.6.3", 0, LsposedFramework.Installed(disabled = false), enabled),
        )
    }

    @Test
    fun `resolveLsposedState enabled without system scope is inactive`() {
        val enabled = LsposedConfig.Enabled(entries = emptyList(), hasSystemFramework = false, extraEntries = emptyList())
        assertEquals(
            LsposedState.InstalledInactive(null),
            resolveLsposedState(false, "0.6.3", 0, LsposedFramework.Installed(disabled = false), enabled),
        )
    }

    @Test
    fun `resolveLsposedState null config is inactive`() {
        assertEquals(
            LsposedState.InstalledInactive(null),
            resolveLsposedState(false, null, 0, LsposedFramework.Installed(disabled = true), null),
        )
    }
}
