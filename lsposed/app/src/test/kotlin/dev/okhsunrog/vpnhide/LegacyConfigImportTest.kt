package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The pre-1.0 importer. Covers the two things a wrong answer costs the user
 * directly: which packages/roles come back out of the old flat files, and what
 * merge/replace do to a config that already has roles.
 */
class LegacyConfigImportTest {
    private val selfPkg = "dev.okhsunrog.vpnhide"

    private val uidToPkg =
        mapOf(
            10123 to "com.bank.app",
            1010123 to "com.bank.app",
            10222 to "com.messenger",
        )

    private fun sections(vararg pairs: Pair<String, String>): Map<String, String> = pairs.toMap()

    @Test
    fun `no legacy files means nothing to offer`() {
        assertNull(parseLegacyConfigCandidate(emptyMap(), uidToPkg))
        assertNull(
            parseLegacyConfigCandidate(
                sections("legacy_kmod_targets" to "\n# comment\n  \n"),
                uidToPkg,
            ),
        )
    }

    @Test
    fun `every legacy file maps onto its role`() {
        val candidate =
            parseLegacyConfigCandidate(
                sections(
                    "legacy_kmod_targets" to "com.bank.app\n",
                    "legacy_zygisk_targets" to "com.messenger\n",
                    "legacy_lsposed_targets" to "com.bank.app\ncom.messenger\n",
                    "legacy_ports_observers" to "com.messenger\n",
                    "legacy_hidden_pkgs" to "com.vpn.client\n",
                    "legacy_observer_uids" to "10222\n",
                ),
                uidToPkg,
            )!!

        assertEquals(
            setOf("com.bank.app", "com.messenger", "com.vpn.client"),
            candidate.roles.keys,
        )
        assertEquals(LegacyRoles(java = true, native = true), candidate.roles["com.bank.app"])
        assertEquals(
            LegacyRoles(java = true, native = true, appHiding = true, ports = true),
            candidate.roles["com.messenger"],
        )
        assertEquals(LegacyRoles(hidden = true), candidate.roles["com.vpn.client"])
        assertEquals(0, candidate.unresolvedObserverUids)
    }

    @Test
    fun `observer uid from another user resolves through the app id`() {
        val candidate =
            parseLegacyConfigCandidate(
                // 1010123 = the bank app in user 10; only its user-0 uid is known here.
                sections("legacy_observer_uids" to "1010123\n"),
                mapOf(10123 to "com.bank.app"),
            )!!
        assertEquals(setOf("com.bank.app"), candidate.roles.keys)
        assertEquals(0, candidate.unresolvedObserverUids)
    }

    @Test
    fun `observer uid of an uninstalled app is counted, not invented`() {
        val candidate =
            parseLegacyConfigCandidate(
                sections(
                    "legacy_kmod_targets" to "com.bank.app\n",
                    "legacy_observer_uids" to "10999\n",
                ),
                uidToPkg,
            )!!
        assertEquals(setOf("com.bank.app"), candidate.roles.keys)
        assertEquals(1, candidate.unresolvedObserverUids)
    }

    @Test
    fun `junk lines never reach the config`() {
        assertNull(
            parseLegacyConfigCandidate(
                sections("legacy_kmod_targets" to "(missing: /data/adb/vpnhide_kmod/targets.txt)\nnodots\n"),
                uidToPkg,
            ),
        )
    }

    @Test
    fun `merge unions roles and keeps existing hook overrides`() {
        val base =
            CanonicalConfig(
                apps =
                    mapOf(
                        "com.bank.app" to
                            CanonicalApp(java = true, javaHooks = listOf("lsposed_get_all_networks")),
                        "com.other" to CanonicalApp(ports = true),
                    ),
            )
        val candidate =
            LegacyConfigCandidate(
                roles =
                    mapOf(
                        "com.bank.app" to LegacyRoles(java = true, native = true),
                        "com.messenger" to LegacyRoles(java = true),
                    ),
            )

        val merged = applyLegacyImport(base, candidate, LegacyImportMode.Merge, selfPkg)

        // Already-on role keeps its narrower hook selection; the newly enabled
        // native role gets everything, since legacy files carry no hook detail.
        assertEquals(listOf("lsposed_get_all_networks"), merged.apps["com.bank.app"]?.javaHooks)
        assertEquals(NativeRole.All, merged.apps["com.bank.app"]?.native)
        assertEquals(true, merged.apps["com.messenger"]?.java)
        assertNull(merged.apps["com.messenger"]?.javaHooks)
        // Untouched by the import, and still present.
        assertEquals(true, merged.apps["com.other"]?.ports)
    }

    @Test
    fun `replace drops current app roles but keeps settings, self and auto-hidden vpns`() {
        val base =
            CanonicalConfig(
                debug = true,
                apps =
                    mapOf(
                        "com.other" to CanonicalApp(java = true, native = NativeRole.All),
                        // Auto-hidden by the VpnService scan, and role-bearing:
                        // the role goes, the hidden mark stays.
                        "com.vpn.client" to CanonicalApp(java = true, hidden = true),
                        selfPkg to CanonicalApp(java = true, native = NativeRole.All, hidden = true),
                    ),
                settings = CanonicalSettings(autoHideVpnName = true, autoHiddenPackages = setOf("com.vpn.client")),
            )
        val candidate = LegacyConfigCandidate(roles = mapOf("com.bank.app" to LegacyRoles(native = true)))

        val replaced = applyLegacyImport(base, candidate, LegacyImportMode.Replace, selfPkg)

        assertEquals(setOf("com.bank.app", "com.vpn.client", selfPkg), replaced.apps.keys)
        assertEquals(base.settings, replaced.settings)
        assertTrue(replaced.debug)
        // Auto-hide mark survives; the app's own roles do not.
        assertEquals(CanonicalApp(hidden = true), replaced.apps["com.vpn.client"])
        // Self keeps full roles whatever the mode did.
        assertEquals(NativeRole.All, replaced.apps[selfPkg]?.native)
        assertEquals(true, replaced.apps[selfPkg]?.java)
    }

    @Test
    fun `auto-hidden vpn apps do not count as a configured setup`() {
        val autoHiddenOnly =
            CanonicalConfig(
                apps =
                    mapOf(
                        selfPkg to CanonicalApp(java = true, native = NativeRole.All, hidden = true),
                        "com.vpn.client" to CanonicalApp(hidden = true),
                    ),
            )
        assertFalse(hasUserConfiguredApps(autoHiddenOnly, selfPkg))

        val configured =
            autoHiddenOnly.copy(
                apps = autoHiddenOnly.apps + ("com.bank.app" to CanonicalApp(java = true)),
            )
        assertTrue(hasUserConfiguredApps(configured, selfPkg))
    }

    @Test
    fun `delete command retires legacy files and only the orphan dirs`() {
        val cmd = buildLegacyConfigDeleteCommand()
        LEGACY_FILES.forEach { assertTrue("$it missing from $cmd", cmd.contains(it)) }
        assertTrue(cmd.contains("rmdir /data/adb/vpnhide_zygisk /data/adb/vpnhide_lsposed"))
        // Live directories: load_status / load_dmesg / ctl.lock still live here.
        assertFalse(cmd.contains("rmdir /data/adb/vpnhide_kmod"))
        assertFalse(cmd.contains("rm -rf"))
    }

    @Test
    fun `both shell probes read every legacy path`() {
        listOf(buildRootShellSnapshotCommand(), buildDebugShellSnapshotCommand()).forEach { cmd ->
            LEGACY_CONFIG_SECTIONS.forEach { (section, path) ->
                // Both probes iterate the pairs the prelude hands them.
                assertTrue("$section missing", cmd.contains("$section=$path"))
            }
        }
    }
}
