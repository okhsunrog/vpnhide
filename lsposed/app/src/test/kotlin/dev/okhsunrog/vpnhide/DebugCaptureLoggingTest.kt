package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.debug.debugToggledCanonicalConfig
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class DebugCaptureLoggingTest {
    @Test
    fun `debug toggle preserves debugSwitch when changing debug only`() {
        val toggled = debugToggledCanonicalConfig(canonicalSnapshotWithSwitch(debug = true, debugSwitch = false), enabled = false)

        assertEquals(false, toggled?.debug)
        assertEquals(false, toggled?.debugSwitch)
        assertPreservedSettings(toggled?.settings)
    }

    @Test
    fun `debug toggle target updates debug only and preserves settings`() {
        val toggled = debugToggledCanonicalConfig(canonicalSnapshot(), enabled = true)

        assertNotNull(toggled)
        assertEquals(true, toggled?.debug)
        assertExampleBankRoles(toggled?.apps?.get("com.example.bank"))
        assertPreservedSettings(toggled?.settings)
    }

    @Test
    fun `debug toggle false keeps canonical roles unchanged`() {
        val toggled = debugToggledCanonicalConfig(canonicalSnapshot(), enabled = false)

        assertEquals(false, toggled?.debug)
        assertExampleBankRoles(toggled?.apps?.get("com.example.bank"))
        assertPreservedSettings(toggled?.settings)
    }

    @Test
    fun `debug toggle returns null when canonical config missing`() {
        val toggled = debugToggledCanonicalConfig(RootSnapshot(mapOf("something_else" to "{}")), enabled = true)
        assertNull(toggled)
    }

    @Test
    fun `startup debug reconcile updates debug only when different from debugSwitch`() {
        val canonical =
            canonicalConfig(
                debug = true,
                debugSwitch = false,
                canonicalApps = mapOf("com.example.bank" to canonicalSampleBankApp()),
            )
        val reconciled = canonicalConfigForStartupDebugReconcile(canonical)

        assertEquals(false, reconciled?.debug)
        assertEquals(false, reconciled?.debugSwitch)
        assertEquals("com.example.bank", reconciled?.apps?.keys?.single())
    }

    @Test
    fun `startup debug reconcile skips rewrite when already aligned`() {
        val canonical = canonicalConfig(debug = true, debugSwitch = true)
        val reconciled = canonicalConfigForStartupDebugReconcile(canonical)
        assertNull(reconciled)
    }

    private fun assertExampleBankRoles(app: CanonicalApp?) {
        assertTrue(app?.java == true)
        assertTrue(app?.native?.enabled == true)
        assertEquals(listOf("dev_ioctl"), app?.native?.overrides?.kernel)
        assertEquals(listOf("zygisk_ioctl"), app?.native?.overrides?.zygisk)
        assertTrue(app?.appHiding == true)
        assertTrue(app?.ports == true)
        assertTrue(app?.hidden == true)
        assertEquals(
            8080,
            app
                ?.portPolicy
                ?.rules
                ?.single()
                ?.start,
        )
    }

    private fun assertPreservedSettings(settings: CanonicalSettings?) {
        assertTrue(settings?.rememberSuperkey == true)
        assertFalse(settings?.autoHideVpnServices == true)
        assertTrue(settings?.autoHideVpnName == true)
        assertEquals(setOf("com.example.vpn"), settings?.autoHiddenPackages)
    }

    private fun canonicalConfig(
        debug: Boolean = true,
        debugSwitch: Boolean = debug,
        canonicalApps: Map<String, CanonicalApp>? = null,
    ): CanonicalConfig =
        CanonicalConfig(
            debug = debug,
            debugSwitch = debugSwitch,
            apps = canonicalApps ?: canonicalSampleApps(),
            settings =
                CanonicalSettings(
                    rememberSuperkey = true,
                    autoHideVpnServices = false,
                    autoHideVpnName = true,
                    autoHiddenPackages = setOf("com.example.vpn"),
                ),
        )

    private fun canonicalSnapshot(): RootSnapshot =
        RootSnapshot(
            mapOf(
                "canonical_config" to canonicalConfigJson(canonicalConfig()),
            ),
        )

    private fun canonicalSnapshotWithSwitch(
        debug: Boolean,
        debugSwitch: Boolean,
    ): RootSnapshot =
        RootSnapshot(
            mapOf(
                "canonical_config" to
                    canonicalConfigJson(canonicalConfig(debug = debug, debugSwitch = debugSwitch)),
            ),
        )

    private fun canonicalSampleApps(): Map<String, CanonicalApp> =
        mapOf(
            "com.example.bank" to canonicalSampleBankApp(),
        )

    private fun canonicalSampleBankApp(): CanonicalApp =
        CanonicalApp(
            java = true,
            native =
                NativeRole(
                    enabled = true,
                    overrides =
                        NativeHookOverrides(
                            kernel = listOf("dev_ioctl"),
                            zygisk = listOf("zygisk_ioctl"),
                        ),
                ),
            appHiding = true,
            ports = true,
            portPolicy =
                PortPolicy(
                    mode = PortPolicyMode.Custom,
                    rules = listOf(PortRule(start = 8080, end = 8080, protocol = PortProtocol.Tcp)),
                ),
            hidden = true,
        )
}
