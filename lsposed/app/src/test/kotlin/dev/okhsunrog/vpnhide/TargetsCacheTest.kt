package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class TargetsCacheTest {
    @Test
    fun `targets snapshot derives roles and observer UIDs from canonical config`() {
        val rootSnapshot =
            RootSnapshot(
                sections =
                    mapOf(
                        "kmod_module_dir" to "1",
                        "zygisk_module_dir" to "0",
                        "ports_prop" to "version=1.2.3",
                        "canonical_config" to
                            """
                            {
                              "version": 1,
                              "apps": {
                                "dev.okhsunrog.vpnhide": { "native": true },
                                "com.bank.app": { "native": true },
                                "com.chat.app": { "native": true },
                                "system": { "java": true },
                                "com.hidden.one": { "hidden": true },
                                "com.hidden.two": { "hidden": true },
                                "com.observer": { "appHiding": true },
                                "com.browser": { "ports": true }
                              }
                            }
                            """.trimIndent(),
                        "superkey_saved" to "1",
                        "pm_packages" to
                            "package:com.observer uid:10123,1010123\n" +
                            "package:com.other uid:20222\n",
                    ),
            )

        val targets = parseTargetsSnapshot(rootSnapshot)

        assertTrue(targets.kmodModuleInstalled)
        assertFalse(targets.zygiskModuleInstalled)
        assertTrue(targets.portsModuleInstalled)
        assertEquals(setOf("dev.okhsunrog.vpnhide", "com.bank.app", "com.chat.app"), targets.nativeTargets)
        assertEquals(setOf("system"), targets.lsposedTargets)
        assertEquals(setOf("com.hidden.one", "com.hidden.two"), targets.hiddenPkgs)
        assertEquals(setOf(10123, 1010123), targets.observerUids)
        assertEquals(setOf("com.observer"), targets.observerNames)
        assertEquals(listOf(10123, 1010123), targets.packageUids["com.observer"])
        assertEquals(setOf("com.browser"), targets.portsObservers)
        assertTrue(targets.apatchSuperkeySaved)
    }

    /**
     * The role must survive a package the inventory cannot see.
     *
     * `appHiding` used to be stored as resolved UIDs and mapped back through
     * `pm list packages`, so a target in a profile the scan could not read
     * vanished from the snapshot — and the next settings write, rebuilt from
     * that snapshot, dropped the role on disk. A toggle unrelated to the app
     * list silently unconfigured an app.
     */
    @Test
    fun `an app-hiding target keeps its role when the inventory cannot see it`() {
        val rootSnapshot =
            RootSnapshot(
                sections =
                    mapOf(
                        "canonical_config" to
                            """
                            {
                              "version": 1,
                              "apps": {
                                "com.invisible": { "appHiding": true },
                                "com.known": { "appHiding": true }
                              }
                            }
                            """.trimIndent(),
                        // com.invisible is installed in a profile this scan missed.
                        "pm_packages" to "package:com.known uid:10123\n",
                    ),
            )

        val targets = parseTargetsSnapshot(rootSnapshot)

        assertEquals(setOf("com.invisible", "com.known"), targets.observerNames)
        // Its UID is genuinely unknown, so it contributes none — that is a
        // property of the inventory, not a reason to forget the role.
        assertEquals(setOf(10123), targets.observerUids)
    }

    @Test
    fun `targets snapshot preserves canonical per-hook selections`() {
        val rootSnapshot =
            RootSnapshot(
                sections =
                    mapOf(
                        "kmod_module_dir" to "1",
                        "zygisk_module_dir" to "1",
                        "kpm_module_dir" to "0",
                        "ports_prop" to "version=1.2.3",
                        "canonical_config" to
                            """
                            {
                              "version": 1,
                              "apps": {
                                "com.java": { "java": ["lsposed_network_capabilities"] },
                                "com.native": { "native": true },
                                "com.observer": { "appHiding": true },
                                "com.ports": { "ports": true },
                                "com.hidden": { "hidden": true }
                              }
                            }
                            """.trimIndent(),
                        "pm_packages" to "package:com.observer uid:10123,1010123\n",
                    ),
            )

        val targets = parseTargetsSnapshot(rootSnapshot)

        assertEquals(setOf("com.java"), targets.lsposedTargets)
        assertEquals(
            listOf("lsposed_network_capabilities"),
            targets.canonicalConfig
                ?.apps
                ?.get("com.java")
                ?.javaHooks,
        )
        assertEquals(setOf("com.native"), targets.nativeTargets)
        assertEquals(setOf("com.hidden"), targets.hiddenPkgs)
        assertEquals(setOf(10123, 1010123), targets.observerUids)
        assertEquals(setOf("com.ports"), targets.portsObservers)
    }

    @Test
    fun `targets snapshot reports active zygisk only for current boot heartbeat`() {
        val sections =
            mapOf(
                "kmod_module_dir" to "0",
                "zygisk_module_dir" to "1",
                "kpm_module_dir" to "0",
                "ports_prop" to "",
                "canonical_config" to "",
                "superkey_saved" to "0",
                "pm_packages" to "",
                "kmod_prop" to "",
                "zygisk_prop" to "version=1.0",
                "kpm_prop" to "",
                "proc_exists" to "0",
                "current_boot_id" to "boot-1",
                "zygisk_status" to "boot_id=boot-1",
                "kpm_load_status" to "",
            )

        assertEquals(
            NativeBackendId.Zygisk,
            parseTargetsSnapshot(RootSnapshot(sections)).activeNativeBackendId,
        )
        assertEquals(
            null,
            parseTargetsSnapshot(RootSnapshot(sections + ("zygisk_status" to "boot_id=old"))).activeNativeBackendId,
        )
    }
}
