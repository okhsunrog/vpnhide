package dev.okhsunrog.vpnhide

// Pure logic for the unified app picker's save path — data in, data out, no
// Android deps, unit-tested (see AppPickerDataTest), per lsposed/AGENTS.md.

/**
 * The hidden-package set to persist on Save. This preserves manual hidden
 * packages and auto-detected VPN apps, and always includes [selfPkg] (managed
 * invisibly, never shown in the picker).
 *
 * Hidden and app-hiding observer roles can coexist. The package-visibility hook
 * avoids self-crashes by never hiding a package from callers with the same
 * appId; other observers still cannot see it.
 */
internal fun resolveHiddenPackages(
    existing: Set<String>,
    selfPkg: String,
): List<String> = (existing + selfPkg).distinct().sorted()

internal data class AppAutoHideSignal(
    val packageName: String,
    val declaresVpnService: Boolean = false,
    val nameContainsVpn: Boolean = false,
)

internal fun resolveAutoHiddenPackages(
    signals: Collection<AppAutoHideSignal>,
    settings: CanonicalSettings,
    selfPkg: String,
): Set<String> =
    signals
        .asSequence()
        .filter { it.packageName != selfPkg }
        .filter {
            (settings.autoHideVpnServices && it.declaresVpnService) ||
                (settings.autoHideVpnName && it.nameContainsVpn)
        }.filter { it.packageName !in settings.autoHideExcludedPackages }
        .mapTo(sortedSetOf()) { it.packageName }

internal data class AppRoleSelection(
    val packageName: String,
    val java: Boolean = false,
    val javaHooks: List<String>? = null,
    val native: Boolean = false,
    val nativeOverrides: NativeHookOverrides = NativeHookOverrides(),
    val appHiding: Boolean = false,
    val ports: Boolean = false,
    val portPolicy: PortPolicy? = null,
)

internal fun resolveNativeHookSelection(
    hookNames: List<String>,
    selectedHookNames: Set<String>,
): List<String>? = resolveHookSelection(hookNames, selectedHookNames)

internal fun resolveHookSelection(
    hookNames: List<String>,
    selectedHookNames: Set<String>,
): List<String>? {
    val ordered = hookNames.filter { it in selectedHookNames }
    return when {
        ordered.isEmpty() -> emptyList()
        ordered.size == hookNames.size -> null
        else -> ordered
    }
}

internal fun buildCanonicalConfigForAppPickerSave(
    debug: Boolean,
    selfPkg: String,
    selections: Collection<AppRoleSelection>,
    snapshot: TargetsSnapshot?,
    autoHideSignals: Collection<AppAutoHideSignal> = emptyList(),
): CanonicalConfig {
    val base = canonicalBaseForSave(debug, snapshot)
    val visiblePkgs = selections.mapTo(mutableSetOf()) { it.packageName }

    fun preserved(predicate: (CanonicalApp) -> Boolean): Set<String> =
        base.apps
            .filter { (pkg, app) -> pkg !in visiblePkgs && predicate(app) }
            .keys

    val javaPkgs = preserved { it.java } + selections.selectedPkgs { it.java } + selfPkg
    val nativePkgs = preserved { it.native.enabled } + selections.selectedPkgs { it.native } + selfPkg
    val selectedJavaHooks = selections.selectedJavaHooks()
    val selectedNativeRoles = selections.selectedNativeRoles()
    val selectedPortPolicies = selections.selectedPortPolicies()
    val observerPkgs = preserved { it.appHiding } + selections.selectedPkgs { it.appHiding }
    val portsPkgs = preserved { it.ports } + selections.selectedPkgs { it.ports }
    val manualHiddenPkgs = base.apps.filterValues { it.hidden }.keys - base.settings.autoHiddenPackages
    val hiddenPkgs =
        resolveHiddenPackages(
            existing = manualHiddenPkgs,
            selfPkg = selfPkg,
        )

    val canonicalWithJavaHooks =
        buildCanonicalConfig(
            debug = debug,
            javaPkgs = javaPkgs,
            nativePkgs = nativePkgs,
            hiddenPkgs = hiddenPkgs,
            observerPkgs = observerPkgs,
            portsPkgs = portsPkgs,
            existing = base,
        ).withJavaHooks(selectedJavaHooks)
    val canonical =
        canonicalWithJavaHooks
            .withNativeRoles(selectedNativeRoles)
            .withPortPolicies(selectedPortPolicies)
    return applyAutoHiddenPackages(
        config = canonical,
        selfPkg = selfPkg,
        signals = autoHideSignals,
    )
}

internal fun applyAutoHiddenPackages(
    config: CanonicalConfig,
    selfPkg: String,
    signals: Collection<AppAutoHideSignal>,
): CanonicalConfig {
    val observerPkgs = config.apps.filterValues { it.appHiding }.keys
    val manualHiddenPkgs = config.apps.filterValues { it.hidden }.keys - config.settings.autoHiddenPackages
    val autoHiddenPkgs = resolveAutoHiddenPackages(signals, config.settings, selfPkg)
    val effectiveAutoHiddenPkgs = autoHiddenPkgs
    val hiddenPkgs =
        resolveHiddenPackages(
            existing = manualHiddenPkgs + effectiveAutoHiddenPkgs,
            selfPkg = selfPkg,
        )
    return buildCanonicalConfig(
        debug = config.debug,
        javaPkgs = config.apps.filterValues { it.java }.keys,
        nativePkgs = config.apps.filterValues { it.native.enabled }.keys,
        hiddenPkgs = hiddenPkgs,
        observerPkgs = observerPkgs,
        portsPkgs = config.apps.filterValues { it.ports }.keys,
        existing = config.copy(settings = config.settings.copy(autoHiddenPackages = effectiveAutoHiddenPkgs.toSortedSet())),
    )
}

/**
 * Whether re-materializing the auto-hidden set against fresh [signals] would
 * change the set persisted for [config] — i.e. whether the startup / Refresh
 * reconcile needs to write. The reconcile write-guard: a newly-installed VPN
 * app that isn't in `autoHiddenPackages` yet flips this true; an unchanged set
 * keeps it false so the reconcile is a no-op.
 */
internal fun autoHiddenPackagesNeedReconcile(
    config: CanonicalConfig,
    selfPkg: String,
    signals: Collection<AppAutoHideSignal>,
): Boolean =
    applyAutoHiddenPackages(config, selfPkg, signals).settings.autoHiddenPackages !=
        config.settings.autoHiddenPackages

internal fun manualHiddenPackages(
    config: CanonicalConfig,
    selfPkg: String,
): Set<String> = config.apps.filterValues { it.hidden }.keys - config.settings.autoHiddenPackages - selfPkg

internal fun updateManualHiddenPackages(
    config: CanonicalConfig,
    selfPkg: String,
    visiblePackages: Set<String>,
    selectedManualHiddenPackages: Set<String>,
    signals: Collection<AppAutoHideSignal>,
): CanonicalConfig {
    val observerPkgs = config.apps.filterValues { it.appHiding }.keys
    val existingManualHidden = manualHiddenPackages(config, selfPkg)
    val nextManualHidden = (existingManualHidden - visiblePackages) + selectedManualHiddenPackages
    val manualConfig =
        buildCanonicalConfig(
            debug = config.debug,
            javaPkgs = config.apps.filterValues { it.java }.keys,
            nativePkgs = config.apps.filterValues { it.native.enabled }.keys,
            hiddenPkgs = resolveHiddenPackages(nextManualHidden, selfPkg),
            observerPkgs = observerPkgs,
            portsPkgs = config.apps.filterValues { it.ports }.keys,
            existing = config,
        )
    return applyAutoHiddenPackages(
        config = manualConfig,
        selfPkg = selfPkg,
        signals = signals,
    )
}

private fun canonicalBaseForSave(
    debug: Boolean,
    snapshot: TargetsSnapshot?,
): CanonicalConfig =
    when {
        snapshot?.canonicalConfig != null -> {
            snapshot.canonicalConfig.copy(
                debug = debug,
                debugSwitch = snapshot.canonicalConfig.debugSwitch,
            )
        }

        snapshot != null -> {
            buildCanonicalConfigFromTargetsSnapshot(snapshot, debug = debug)
        }

        else -> {
            CanonicalConfig(debug = debug)
        }
    }

private fun Collection<AppRoleSelection>.selectedPkgs(predicate: (AppRoleSelection) -> Boolean): Set<String> =
    filter(predicate).mapTo(mutableSetOf()) { it.packageName }

private fun Collection<AppRoleSelection>.selectedNativeRoles(): Map<String, NativeRole> =
    filter { it.native }
        .associate { selection ->
            selection.packageName to NativeRole(enabled = true, overrides = selection.nativeOverrides)
        }

private fun Collection<AppRoleSelection>.selectedJavaHooks(): Map<String, List<String>?> =
    filter { it.java }
        .associate { selection ->
            selection.packageName to selection.javaHooks?.takeIf { it.isNotEmpty() }
        }

private fun Collection<AppRoleSelection>.selectedPortPolicies(): Map<String, PortPolicy?> =
    filter { it.ports }
        .associate { selection ->
            selection.packageName to normalizePortPolicy(selection.portPolicy)
        }

private fun CanonicalConfig.withJavaHooks(hooks: Map<String, List<String>?>): CanonicalConfig {
    if (hooks.isEmpty()) return this
    val updated =
        apps
            .mapValues { (pkg, app) ->
                if (pkg in hooks) app.copy(javaHooks = hooks[pkg]) else app
            }.toSortedMap()
    return copy(apps = updated)
}

private fun CanonicalConfig.withNativeRoles(roles: Map<String, NativeRole>): CanonicalConfig {
    if (roles.isEmpty()) return this
    val updated =
        apps
            .mapValues { (pkg, app) ->
                roles[pkg]?.let { app.copy(native = it) } ?: app
            }.toSortedMap()
    return copy(apps = updated)
}

private fun CanonicalConfig.withPortPolicies(policies: Map<String, PortPolicy?>): CanonicalConfig {
    if (policies.isEmpty()) return this
    val updated =
        apps
            .mapValues { (pkg, app) ->
                if (pkg in policies) app.copy(portPolicy = policies[pkg]) else app
            }.toSortedMap()
    return copy(apps = updated)
}
