package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.settings.ConfiguredAppRole
import dev.okhsunrog.vpnhide.settings.PackageListFormat
import dev.okhsunrog.vpnhide.settings.PackageListSource
import dev.okhsunrog.vpnhide.settings.UnavailableConfiguredApp
import dev.okhsunrog.vpnhide.settings.formatPackageListExport
import dev.okhsunrog.vpnhide.settings.packageListExportPackages
import dev.okhsunrog.vpnhide.settings.removeConfiguredPackages
import dev.okhsunrog.vpnhide.settings.unavailableConfiguredApps
import org.junit.Assert.assertEquals
import org.junit.Test

class SettingsDataTest {
    private val self = "dev.okhsunrog.vpnhide"

    @Test
    fun `unavailable configured apps include only configured packages missing from visible set`() {
        val config =
            CanonicalConfig(
                apps =
                    mapOf(
                        self to CanonicalApp(java = true, hidden = true),
                        "com.available" to CanonicalApp(java = true),
                        "com.hidden.only" to CanonicalApp(hidden = true),
                        "com.missing.full" to CanonicalApp(java = true, native = NativeRole.All, appHiding = true, ports = true),
                    ),
            )

        assertEquals(
            listOf(
                UnavailableConfiguredApp("com.hidden.only", listOf(ConfiguredAppRole.Hidden)),
                UnavailableConfiguredApp(
                    "com.missing.full",
                    listOf(
                        ConfiguredAppRole.Java,
                        ConfiguredAppRole.Native,
                        ConfiguredAppRole.AppHiding,
                        ConfiguredAppRole.Ports,
                    ),
                ),
            ),
            unavailableConfiguredApps(config, visiblePackages = setOf("com.available"), selfPkg = self),
        )
    }

    @Test
    fun `remove configured packages keeps self and clears auto hidden package memory`() {
        val config =
            CanonicalConfig(
                apps =
                    mapOf(
                        self to CanonicalApp(java = true, hidden = true),
                        "com.keep" to CanonicalApp(java = true),
                        "com.remove" to CanonicalApp(hidden = true),
                    ),
                settings = CanonicalSettings(autoHiddenPackages = setOf("com.keep", "com.remove")),
            )

        val updated = removeConfiguredPackages(config, packages = setOf(self, "com.remove"), selfPkg = self)

        assertEquals(setOf(self, "com.keep"), updated.apps.keys)
        assertEquals(setOf("com.keep"), updated.settings.autoHiddenPackages)
    }

    @Test
    fun `remove configured packages also clears auto hide exclusions`() {
        val config =
            CanonicalConfig(
                apps =
                    mapOf(
                        "com.keep" to CanonicalApp(hidden = true),
                        "com.remove" to CanonicalApp(hidden = true),
                    ),
                settings =
                    CanonicalSettings(
                        autoHideExcludedPackages = setOf("com.keep", "com.remove"),
                    ),
            )

        val updated = removeConfiguredPackages(config, packages = setOf("com.remove"), selfPkg = self)

        assertEquals(setOf("com.keep"), updated.settings.autoHideExcludedPackages)
    }

    @Test
    fun `package list export keeps source-specific package ids sorted and excludes self`() {
        val config =
            CanonicalConfig(
                apps =
                    mapOf(
                        self to CanonicalApp(java = true, native = NativeRole.All),
                        "com.native" to CanonicalApp(native = NativeRole.All),
                        "com.java.b" to CanonicalApp(java = true),
                        "com.java.a" to CanonicalApp(java = true),
                        "com.hidden.only" to CanonicalApp(hidden = true),
                    ),
            )

        assertEquals(
            listOf("com.java.a", "com.java.b"),
            packageListExportPackages(config, PackageListSource.Java, self),
        )
        assertEquals(
            listOf("com.java.a", "com.java.b", "com.native"),
            packageListExportPackages(config, PackageListSource.AllProtection, self),
        )
    }

    @Test
    fun `package list export formats comma and line separated text`() {
        val config =
            CanonicalConfig(
                apps =
                    mapOf(
                        "com.b" to CanonicalApp(java = true),
                        "com.a" to CanonicalApp(java = true),
                    ),
            )

        assertEquals("com.a,com.b", formatPackageListExport(config, PackageListSource.Java, PackageListFormat.Comma, self))
        assertEquals("com.a\ncom.b", formatPackageListExport(config, PackageListSource.Java, PackageListFormat.Lines, self))
    }
}
