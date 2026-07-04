package dev.okhsunrog.vpnhide

internal enum class HiddenAppsFilter {
    All,
    Automatic,
    Manual,
    Excluded,
}

internal enum class AutoHideReason {
    VpnService,
    NameMatch,
}

internal data class HiddenAppState(
    val packageName: String,
    val manual: Boolean,
    val automatic: Boolean,
    val excluded: Boolean,
    val reasons: List<AutoHideReason>,
    val unavailable: Boolean,
) {
    val hidden: Boolean get() = manual || automatic
}

internal data class HiddenAppsSummary(
    val hidden: Int,
    val automatic: Int,
    val manual: Int,
    val excluded: Int,
)

internal fun autoHideReasons(
    signal: AppAutoHideSignal,
    settings: CanonicalSettings,
    selfPkg: String,
): List<AutoHideReason> {
    if (signal.packageName == selfPkg) return emptyList()
    return buildList {
        if (settings.autoHideVpnServices && signal.declaresVpnService) {
            add(AutoHideReason.VpnService)
        }
        if (settings.autoHideVpnName && signal.nameContainsVpn) {
            add(AutoHideReason.NameMatch)
        }
    }
}

internal fun hiddenAppStates(
    config: CanonicalConfig,
    selfPkg: String,
    signals: Collection<AppAutoHideSignal>,
): List<HiddenAppState> {
    val manualPkgs = manualHiddenPackages(config, selfPkg)
    val signalPackages = signals.mapTo(mutableSetOf()) { it.packageName }
    val reasonsByPackage =
        signals
            .associate { signal -> signal.packageName to autoHideReasons(signal, config.settings, selfPkg) }
            .filterValues { it.isNotEmpty() }
    val excludedPkgs = config.settings.autoHideExcludedPackages - selfPkg
    val automaticPkgs = config.settings.autoHiddenPackages - selfPkg
    val packages = (signalPackages + manualPkgs + automaticPkgs + excludedPkgs - selfPkg).sorted()

    return packages.map { pkg ->
        HiddenAppState(
            packageName = pkg,
            manual = pkg in manualPkgs,
            automatic = pkg in automaticPkgs,
            excluded = pkg in excludedPkgs,
            reasons = reasonsByPackage[pkg].orEmpty(),
            unavailable = pkg !in signalPackages,
        )
    }
}

internal fun hiddenAppsSummary(states: Collection<HiddenAppState>): HiddenAppsSummary =
    HiddenAppsSummary(
        hidden = states.count { it.hidden },
        automatic = states.count { it.automatic },
        manual = states.count { it.manual },
        excluded = states.count { it.excluded },
    )

internal fun filterHiddenAppStates(
    states: List<HiddenAppState>,
    filter: HiddenAppsFilter,
): List<HiddenAppState> =
    when (filter) {
        HiddenAppsFilter.All -> states
        HiddenAppsFilter.Automatic -> states.filter { it.automatic }
        HiddenAppsFilter.Manual -> states.filter { it.manual }
        HiddenAppsFilter.Excluded -> states.filter { it.excluded }
    }

internal fun visibleHiddenAppStates(
    savedStates: List<HiddenAppState>,
    draftStates: List<HiddenAppState>,
    filter: HiddenAppsFilter,
    searchQuery: String,
    labelsByPackage: Map<String, String>,
): List<HiddenAppState> {
    val draftByPackage = draftStates.associateBy { it.packageName }
    val query = searchQuery.trim().lowercase()
    return filterHiddenAppStates(savedStates, filter)
        .filter { state ->
            val label = labelsByPackage[state.packageName]
            query.isEmpty() ||
                state.packageName.lowercase().contains(query) ||
                label?.lowercase()?.contains(query) == true
        }.sortedWith(
            compareBy<HiddenAppState> { !it.hidden }
                .thenBy { labelsByPackage[it.packageName]?.lowercase() ?: it.packageName },
        ).map { state ->
            draftByPackage[state.packageName] ?: state
        }
}

internal fun updateHiddenAppsConfig(
    config: CanonicalConfig,
    selfPkg: String,
    visiblePackages: Set<String>,
    selectedManualHiddenPackages: Set<String>,
    excludedPackages: Set<String>,
    signals: Collection<AppAutoHideSignal>,
): CanonicalConfig {
    val withExclusions =
        config.copy(
            settings =
                config.settings.copy(
                    autoHideExcludedPackages = (excludedPackages - selfPkg).toSortedSet(),
                ),
        )
    return updateManualHiddenPackages(
        config = withExclusions,
        selfPkg = selfPkg,
        visiblePackages = visiblePackages,
        selectedManualHiddenPackages = selectedManualHiddenPackages,
        signals = signals,
    )
}
