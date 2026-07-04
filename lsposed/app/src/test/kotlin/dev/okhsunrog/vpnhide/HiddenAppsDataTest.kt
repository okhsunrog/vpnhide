package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class HiddenAppsDataTest {
    private val self = "dev.okhsunrog.vpnhide"

    @Test
    fun `auto hide exclusion prevents an auto matched package from being hidden`() {
        val config =
            CanonicalConfig(
                settings =
                    CanonicalSettings(
                        autoHideExcludedPackages = setOf("com.vpn.client"),
                    ),
            )

        val updated =
            applyAutoHiddenPackages(
                config = config,
                selfPkg = self,
                signals = listOf(AppAutoHideSignal("com.vpn.client", declaresVpnService = true)),
            )

        assertFalse(updated.apps.containsKey("com.vpn.client"))
        assertEquals(emptySet<String>(), updated.settings.autoHiddenPackages)
        assertEquals(setOf("com.vpn.client"), updated.settings.autoHideExcludedPackages)
    }

    @Test
    fun `hidden app states expose automatic manual and excluded sources`() {
        val config =
            CanonicalConfig(
                apps =
                    mapOf(
                        "com.manual" to CanonicalApp(hidden = true),
                        "com.auto" to CanonicalApp(hidden = true),
                    ),
                settings =
                    CanonicalSettings(
                        autoHideExcludedPackages = setOf("com.excluded"),
                        autoHiddenPackages = setOf("com.auto"),
                    ),
            )

        val states =
            hiddenAppStates(
                config = config,
                selfPkg = self,
                signals =
                    listOf(
                        AppAutoHideSignal("com.auto", declaresVpnService = true),
                        AppAutoHideSignal("com.excluded", declaresVpnService = true),
                    ),
            ).associateBy { it.packageName }

        assertTrue(states.getValue("com.manual").manual)
        assertFalse(states.getValue("com.manual").automatic)
        assertTrue(states.getValue("com.auto").automatic)
        assertEquals(listOf(AutoHideReason.VpnService), states.getValue("com.auto").reasons)
        assertTrue(states.getValue("com.excluded").excluded)
        assertFalse(states.getValue("com.excluded").hidden)
        assertEquals(listOf(AutoHideReason.VpnService), states.getValue("com.excluded").reasons)
    }

    @Test
    fun `hidden app states include ordinary visible apps for manual selection`() {
        val states =
            hiddenAppStates(
                config = CanonicalConfig(),
                selfPkg = self,
                signals = listOf(AppAutoHideSignal("com.normal")),
            )

        assertEquals(listOf("com.normal"), states.map { it.packageName })
        assertFalse(states.single().hidden)
        assertFalse(states.single().manual)
        assertFalse(states.single().automatic)
    }

    @Test
    fun `update hidden apps config saves exclusions and keeps manual choices`() {
        val config =
            CanonicalConfig(
                apps = mapOf("com.old.manual" to CanonicalApp(hidden = true)),
            )

        val updated =
            updateHiddenAppsConfig(
                config = config,
                selfPkg = self,
                visiblePackages = setOf("com.old.manual", "com.new.manual", "com.vpn"),
                selectedManualHiddenPackages = setOf("com.new.manual"),
                excludedPackages = setOf("com.vpn"),
                signals = listOf(AppAutoHideSignal("com.vpn", declaresVpnService = true)),
            )

        assertFalse(updated.apps.containsKey("com.old.manual"))
        assertTrue(updated.apps.getValue("com.new.manual").hidden)
        assertFalse(updated.apps.containsKey("com.vpn"))
        assertEquals(setOf("com.vpn"), updated.settings.autoHideExcludedPackages)
    }
}
