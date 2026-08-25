package dev.okhsunrog.vpnhide.settings

import dev.okhsunrog.vpnhide.CanonicalApp
import dev.okhsunrog.vpnhide.CanonicalConfig

internal enum class ConfiguredAppRole {
    Java,
    Native,
    AppHiding,
    Ports,
    Hidden,
}

internal data class UnavailableConfiguredApp(
    val packageName: String,
    val roles: List<ConfiguredAppRole>,
)

internal enum class PackageListSource {
    Java,
    Native,
    AppHiding,
    Ports,
    AllProtection,
}

internal enum class PackageListFormat {
    Comma,
    Lines,
}

internal fun unavailableConfiguredApps(
    config: CanonicalConfig,
    visiblePackages: Set<String>,
    selfPkg: String,
): List<UnavailableConfiguredApp> =
    config.apps
        .filter { (pkg, app) -> pkg != selfPkg && pkg !in visiblePackages && app.hasAnyRole }
        .map { (pkg, app) -> UnavailableConfiguredApp(packageName = pkg, roles = configuredAppRoles(app)) }
        .sortedBy { it.packageName }

internal fun removeConfiguredPackages(
    config: CanonicalConfig,
    packages: Set<String>,
    selfPkg: String,
): CanonicalConfig {
    val removable = packages - selfPkg
    if (removable.isEmpty()) return config
    return config.copy(
        apps = config.apps.filterKeys { it !in removable }.toSortedMap(),
        settings =
            config.settings.copy(
                autoHiddenPackages = (config.settings.autoHiddenPackages - removable).toSortedSet(),
                autoHideExcludedPackages = (config.settings.autoHideExcludedPackages - removable).toSortedSet(),
            ),
    )
}

internal fun packageListExportPackages(
    config: CanonicalConfig,
    source: PackageListSource,
    selfPkg: String,
): List<String> =
    config.apps
        .filter { (pkg, app) -> pkg != selfPkg && app.matchesPackageListSource(source) }
        .keys
        .sorted()

internal fun formatPackageListExport(
    config: CanonicalConfig,
    source: PackageListSource,
    format: PackageListFormat,
    selfPkg: String,
): String {
    val packages = packageListExportPackages(config, source, selfPkg)
    val separator =
        when (format) {
            PackageListFormat.Comma -> ","
            PackageListFormat.Lines -> "\n"
        }
    return packages.joinToString(separator)
}

internal fun packageListExportFileName(source: PackageListSource): String = "vpnhide_${source.name.lowercase()}_packages.txt"

private fun configuredAppRoles(app: CanonicalApp): List<ConfiguredAppRole> =
    buildList {
        if (app.java) add(ConfiguredAppRole.Java)
        if (app.native.enabled) add(ConfiguredAppRole.Native)
        if (app.appHiding) add(ConfiguredAppRole.AppHiding)
        if (app.ports) add(ConfiguredAppRole.Ports)
        if (app.hidden) add(ConfiguredAppRole.Hidden)
    }

private fun CanonicalApp.matchesPackageListSource(source: PackageListSource): Boolean =
    when (source) {
        PackageListSource.Java -> java
        PackageListSource.Native -> native.enabled
        PackageListSource.AppHiding -> appHiding
        PackageListSource.Ports -> ports
        PackageListSource.AllProtection -> java || native.enabled || appHiding || ports
    }
