package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class DebugCaptureLoggingTest {
    @Test
    fun `debug toggle updates only canonical debug flag when canonical config is valid`() {
        val update = debugToggledCanonicalConfig(canonicalSnapshot(), enabled = true)

        assertEquals("canonical_debug_only", update?.source)
        assertTrue(update?.config?.debug == true)
        assertExampleBankRoles(update?.config?.apps?.get("com.example.bank"))
        assertPreservedSettings(update?.config?.settings)
    }

    @Test
    fun `startup reconcile rewrites stale canonical debug from persisted preference`() {
        val stale = canonicalConfig()

        val update =
            runtimeReconcileCanonicalConfig(
                targetsSnapshot(stale.copy(debug = true)),
                persistedDebug = false,
            )

        assertEquals(false, update?.debug)
        assertExampleBankRoles(update?.apps?.get("com.example.bank"))
        assertPreservedSettings(update?.settings)
    }

    @Test
    fun `startup reconcile leaves canonical untouched when debug matches preference`() {
        val update =
            runtimeReconcileCanonicalConfig(
                targetsSnapshot(canonicalConfig().copy(debug = false)),
                persistedDebug = false,
            )

        assertNull(update)
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

    private fun canonicalConfig(): CanonicalConfig =
        parseCanonicalConfig(
            """
            {
              "version": 1,
              "debug": false,
              "apps": {
                "com.example.bank": {
                  "java": true,
                  "native": {
                    "enabled": true,
                    "kernel": ["dev_ioctl"],
                    "zygisk": ["zygisk_ioctl"]
                  },
                  "appHiding": true,
                  "ports": true,
                  "portPolicy": {
                    "mode": "custom",
                    "rules": [{ "start": 8080, "protocol": "tcp" }]
                  },
                  "hidden": true
                }
              },
              "settings": {
                "rememberSuperkey": true,
                "autoHideVpnServices": false,
                "autoHideVpnName": true,
                "autoHiddenPackages": ["com.example.vpn"]
              }
            }
            """.trimIndent(),
        ) ?: error("invalid test canonical config")

    private fun canonicalSnapshot(): RootSnapshot =
        RootSnapshot(
            mapOf(
                "canonical_config" to canonicalConfigJson(canonicalConfig()),
            ),
        )

    private fun targetsSnapshot(canonical: CanonicalConfig): TargetsSnapshot =
        TargetsSnapshot(
            kmodModuleInstalled = false,
            kpmModuleInstalled = false,
            zygiskModuleInstalled = false,
            portsModuleInstalled = false,
            kmodTargets = emptySet(),
            kpmTargets = emptySet(),
            zygiskTargets = emptySet(),
            lsposedTargets = emptySet(),
            hiddenPkgs = emptySet(),
            observerUids = emptySet(),
            portsObservers = emptySet(),
            uidToPkg = emptyMap(),
            canonicalConfig = canonical,
        )
}
