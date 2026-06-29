package dev.okhsunrog.vpnhide

import android.content.Context
import dev.okhsunrog.vpnhide.generated.HookIds
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * Transport-independent control surface used by the debug host bridge.
 *
 * Every mutating function writes the existing canonical JSON and then runs the
 * same activators used by the UI save/import paths. The Settings toggle only
 * gates the debug transport; these functions stay plain Kotlin entry points.
 */
internal object AgentControl {
    /**
     * Return the current dashboard state. Set refresh to true to force a fresh root snapshot.
     *
     * @param refresh Force a fresh root snapshot instead of reusing the app cache.
     */
    suspend fun getDashboardState(
        context: Context,
        refresh: Boolean? = null,
    ): AgentDashboardState =
        withAppContext(context) { context ->
            val rootSnapshot = rootSnapshot(refresh == true)
            val dashboard = loadDashboardState(context, selfNeedsRestart = false, rootSnapshot = rootSnapshot)
            dashboard.toAgentDashboardState()
        }

    /**
     * Run the full diagnostics suite and return every check shown in Detailed diagnostics.
     */
    suspend fun runFullDiagnostics(context: Context): AgentDiagnosticsReport =
        withAppContext(context) { context ->
            val results = DiagnosticsCache.awaitFullResults(context)
            if (results == null) {
                AgentDiagnosticsReport(
                    state = "vpn_off",
                    score = AgentCheckScore(0, 0),
                    nativeChecks = emptyList(),
                    javaChecks = emptyList(),
                )
            } else {
                results.toAgentDiagnosticsReport()
            }
        }

    /**
     * Return the Statistics tab state. Set refresh to true to force a fresh root snapshot.
     *
     * @param refresh Force a fresh root snapshot instead of reusing the app cache.
     */
    suspend fun getStatisticsState(
        context: Context,
        refresh: Boolean? = null,
    ): AgentStatisticsState =
        withAppContext(context) { context ->
            buildStatisticsState(rootSnapshot(refresh == true)).toAgentStatisticsState(selfPackage = context.packageName)
        }

    /**
     * Take a cumulative-counter baseline for the Statistics capture-session flow.
     *
     * @param refresh Force a fresh root snapshot before taking the baseline.
     */
    suspend fun getStatisticsCaptureBaseline(
        context: Context,
        refresh: Boolean? = null,
    ): AgentStatisticsCaptureBaseline =
        withAppContext(context) {
            snapshotCounters(buildStatisticsState(rootSnapshot(refresh == true))).toAgentStatisticsCaptureBaseline()
        }

    /**
     * Return probes that happened since a baseline from getStatisticsCaptureBaseline.
     *
     * @param baseline Baseline returned by getStatisticsCaptureBaseline.
     * @param refresh Force a fresh root snapshot before diffing.
     */
    suspend fun getStatisticsCaptureDiff(
        context: Context,
        baseline: AgentStatisticsCaptureBaseline,
        refresh: Boolean? = null,
    ): AgentStatisticsCaptureDiff =
        withAppContext(context) { context ->
            diffCapture(
                baseline = baseline.toCounterMap(),
                current = buildStatisticsState(rootSnapshot(refresh == true)),
                selfPackage = context.packageName,
            ).toAgentStatisticsCaptureDiff()
        }

    /**
     * Return the Protection tab canonical config and configured package summary.
     *
     * @param refresh Force a fresh root snapshot instead of reusing the app cache.
     */
    suspend fun getProtectionState(
        context: Context,
        refresh: Boolean? = null,
    ): AgentProtectionState =
        withAppContext(context) { context ->
            val snapshot = targetsSnapshot(refresh == true)
            buildProtectionState(context, snapshot)
        }

    /**
     * List installed apps in the same shape used by the Protection picker.
     *
     * @param includeSystem Include system packages.
     * @param configuredOnly Return only packages currently present in canonical config.
     * @param refresh Force a package-manager/root package-list refresh.
     */
    suspend fun listInstalledApps(
        context: Context,
        includeSystem: Boolean? = null,
        configuredOnly: Boolean? = null,
        refresh: Boolean? = null,
    ): List<AgentInstalledApp> =
        withAppContext(context) { context ->
            val apps = AppListCache.loadForAgent(context, force = refresh == true)
            val userNames = AppListCache.userNames.value
            val protection = buildProtectionState(context, targetsSnapshot(refresh = false))
            val rolesByPackage = protection.configuredApps.associateBy { it.packageName }
            apps
                .asSequence()
                .filter { includeSystem == true || !it.isSystem }
                .filter { configuredOnly != true || it.packageName in rolesByPackage }
                .map { app ->
                    app.toAgentInstalledApp(
                        userNames = userNames,
                        configured = rolesByPackage[app.packageName],
                    )
                }.sortedBy { it.label.lowercase() }
                .toList()
        }

    /**
     * Export the canonical JSON config used by Settings backup/export.
     */
    suspend fun exportCanonicalConfig(context: Context): String =
        withAppContext(context) { context ->
            canonicalConfigJson(currentCanonicalConfig(context, refresh = false))
        }

    /**
     * Import canonical JSON and immediately activate native/ports runtime state.
     *
     * @param json Canonical JSON as produced by exportCanonicalConfig.
     */
    suspend fun importCanonicalConfig(
        context: Context,
        json: String,
    ): AgentMutationResult =
        withAppContext(context) { context ->
            val canonical =
                parseImportedCanonicalConfig(json, context.packageName)
                    ?: throw IllegalArgumentException("Invalid canonical JSON")
            applyCanonicalConfig(
                context = context,
                canonical = canonical,
                updateDebugPreference = true,
            )
        }

    /**
     * Set high-level protection roles for one package. Null arguments leave the role unchanged.
     *
     * @param packageName Package name to update.
     * @param java Enable/disable Java role, or null to keep it unchanged.
     * @param native Enable/disable Native role, or null to keep it unchanged.
     * @param appHiding Enable/disable Apps observer role, or null to keep it unchanged.
     * @param ports Enable/disable Ports observer role, or null to keep it unchanged.
     * @param hidden Mark package hidden from app-hiding observers, or null to keep it unchanged.
     */
    suspend fun setAppProtection(
        context: Context,
        packageName: String,
        java: Boolean? = null,
        native: Boolean? = null,
        appHiding: Boolean? = null,
        ports: Boolean? = null,
        hidden: Boolean? = null,
    ): AgentMutationResult =
        withAppContext(context) { context ->
            val pkg = requirePackageName(packageName)
            val base = currentCanonicalConfig(context, refresh = true)
            val current = base.apps[pkg] ?: CanonicalApp()
            val nextPorts = ports ?: current.ports
            val next =
                current.copy(
                    java = java ?: current.java,
                    javaHooks = current.javaHooks.takeIf { java != false },
                    native =
                        when (native) {
                            true -> current.native.takeIf { it.enabled } ?: NativeRole.All
                            false -> NativeRole.Disabled
                            null -> current.native
                        },
                    appHiding = appHiding ?: current.appHiding,
                    ports = nextPorts,
                    portPolicy = current.portPolicy.takeIf { nextPorts },
                    hidden = hidden ?: current.hidden,
                )
            applyCanonicalConfig(context, base.withApp(pkg, next))
        }

    /**
     * Set exact Java hook selection for one package. Empty hookIds disables Java for that package.
     *
     * @param packageName Package name to update.
     * @param hookIds Hook names from the Java hook registry.
     */
    suspend fun setJavaHooks(
        context: Context,
        packageName: String,
        hookIds: List<String>,
    ): AgentMutationResult =
        withAppContext(context) { context ->
            val pkg = requirePackageName(packageName)
            val hooks = resolveHookIds(hookIds, LsposedJavaHookEntries)
            val base = currentCanonicalConfig(context, refresh = true)
            val current = base.apps[pkg] ?: CanonicalApp()
            val selected = resolveHookSelection(LsposedJavaHookEntries.map { it.hookName }, hooks.toSet())
            val next =
                if (selected.isNullOrEmpty() && hooks.isEmpty()) {
                    current.copy(java = false, javaHooks = null)
                } else {
                    current.copy(java = true, javaHooks = selected)
                }
            applyCanonicalConfig(context, base.withApp(pkg, next))
        }

    /**
     * Set exact native hook selection for one package and native family.
     *
     * @param packageName Package name to update.
     * @param family kernel, zygisk, or null for the currently active/displayed native family.
     * @param hookIds Hook names from that native family's hook registry. Empty disables Native.
     */
    suspend fun setNativeHooks(
        context: Context,
        packageName: String,
        family: String? = null,
        hookIds: List<String>,
    ): AgentMutationResult =
        withAppContext(context) { context ->
            val pkg = requirePackageName(packageName)
            val snapshot = targetsSnapshot(refresh = true)
            val hookFamily = family?.let(::parseNativeHookFamily) ?: snapshot.nativeHookFamily
            val entries = nativeHookEntriesFor(hookFamily)
            val hooks = resolveHookIds(hookIds, entries)
            val base = currentCanonicalConfig(context, refresh = false)
            val current = base.apps[pkg] ?: CanonicalApp()
            val selected = resolveNativeHookSelection(entries.map { it.hookName }, hooks.toSet())
            val next =
                if (selected.isNullOrEmpty() && hooks.isEmpty()) {
                    current.copy(native = NativeRole.Disabled)
                } else {
                    current.copy(
                        native =
                            NativeRole(
                                enabled = true,
                                overrides = current.native.overrides.withHooksFor(hookFamily, selected),
                            ),
                    )
                }
            applyCanonicalConfig(context, base.withApp(pkg, next))
        }

    /**
     * Set localhost port policy for one package. Use mode all to block every localhost port.
     *
     * @param packageName Package name to update.
     * @param mode all, preset, or custom.
     * @param preset Preset id such as common_proxy when mode is preset.
     * @param rules Materialized port rules for custom or preset mode.
     */
    suspend fun setPortPolicy(
        context: Context,
        packageName: String,
        mode: String,
        preset: String? = null,
        rules: List<AgentPortRule>? = null,
    ): AgentMutationResult =
        withAppContext(context) { context ->
            val pkg = requirePackageName(packageName)
            val policy = parseAgentPortPolicy(mode, preset, rules.orEmpty())
            val base = currentCanonicalConfig(context, refresh = true)
            val current = base.apps[pkg] ?: CanonicalApp()
            val next = current.copy(ports = true, portPolicy = policy)
            applyCanonicalConfig(context, base.withApp(pkg, next), targetRestartRecommended = true)
        }

    /**
     * Update app-hiding auto-detection heuristics.
     *
     * @param autoHideVpnServices Auto-hide packages declaring VpnService, or null to keep unchanged.
     * @param autoHideVpnName Auto-hide non-system apps with VPN in label, or null to keep unchanged.
     */
    suspend fun setAutoHideSettings(
        context: Context,
        autoHideVpnServices: Boolean? = null,
        autoHideVpnName: Boolean? = null,
    ): AgentMutationResult =
        withAppContext(context) { context ->
            val apps = AppListCache.loadForAgent(context, force = true)
            val base = currentCanonicalConfig(context, refresh = true)
            val nextSettings =
                base.settings.copy(
                    autoHideVpnServices = autoHideVpnServices ?: base.settings.autoHideVpnServices,
                    autoHideVpnName = autoHideVpnName ?: base.settings.autoHideVpnName,
                )
            val next =
                applyAutoHiddenPackages(
                    config = base.copy(settings = nextSettings),
                    selfPkg = context.packageName,
                    signals = apps.map(AppSummary::toAutoHideSignal),
                )
            applyCanonicalConfig(context, next)
        }

    /**
     * Replace manual hidden-package selection. Auto-hidden packages are preserved.
     *
     * @param packageNames Manual hidden packages to keep selected.
     */
    suspend fun setManualHiddenPackages(
        context: Context,
        packageNames: List<String>,
    ): AgentMutationResult =
        withAppContext(context) { context ->
            val apps = AppListCache.loadForAgent(context, force = true)
            val base = currentCanonicalConfig(context, refresh = true)
            val visiblePackages = apps.mapTo(mutableSetOf()) { it.packageName }
            val selected = packageNames.map(::requirePackageName).toSortedSet()
            val next =
                updateManualHiddenPackages(
                    config = base,
                    selfPkg = context.packageName,
                    visiblePackages = visiblePackages,
                    selectedManualHiddenPackages = selected,
                    signals = apps.map(AppSummary::toAutoHideSignal),
                )
            applyCanonicalConfig(context, next)
        }

    /**
     * Remove stale configured packages from canonical config.
     *
     * @param packageNames Package names to remove.
     */
    suspend fun removeConfiguredPackages(
        context: Context,
        packageNames: List<String>,
    ): AgentMutationResult =
        withAppContext(context) { context ->
            val packages = packageNames.map(::requirePackageName).toSortedSet()
            if (packages.isEmpty()) return@withAppContext AgentMutationResult(ok = true, message = "No packages to remove")
            val base = currentCanonicalConfig(context, refresh = true)
            applyCanonicalConfig(
                context = context,
                canonical = removeConfiguredPackages(base, packages, context.packageName),
            )
        }

    /**
     * Toggle VPN Hide debug logging and propagate it to runtime sinks.
     *
     * @param enabled New debug logging state.
     */
    suspend fun setDebugLogging(
        context: Context,
        enabled: Boolean,
    ): AgentMutationResult =
        withAppContext(context) { context ->
            setDebugLoggingEnabled(context, enabled)
            AgentMutationResult(ok = true, message = "Debug logging updated", changed = true)
        }

    /**
     * Re-run native and ports activators for the current canonical config.
     */
    suspend fun activateConfig(context: Context): AgentMutationResult =
        withAppContext(context) { context ->
            runActivation(context, changed = false)
        }
}

private suspend fun <T> withAppContext(
    context: Context,
    block: suspend (Context) -> T,
): T =
    withContext(Dispatchers.IO) {
        block(context.applicationContext)
    }

private suspend fun rootSnapshot(refresh: Boolean): RootSnapshot =
    if (refresh) RootSnapshotCache.refresh() else RootSnapshotCache.getOrLoad()

private suspend fun targetsSnapshot(refresh: Boolean): TargetsSnapshot = parseTargetsSnapshot(rootSnapshot(refresh))

private suspend fun currentCanonicalConfig(
    context: Context,
    refresh: Boolean,
): CanonicalConfig {
    val snapshot = targetsSnapshot(refresh)
    return snapshot.canonicalConfig
        ?: buildCanonicalConfigFromTargetsSnapshot(snapshot, debug = isEnabledInPrefs(context))
}

private fun applyCanonicalConfig(
    context: Context,
    canonical: CanonicalConfig,
    updateDebugPreference: Boolean = false,
    targetRestartRecommended: Boolean = true,
): AgentMutationResult {
    val next = canonicalConfigWithSelfTarget(canonical, context.packageName)
    val result = runActivationCommand(buildCanonicalConfigWriteCommand(next), changed = true)
    if (result.ok) {
        if (updateDebugPreference) storeDebugLoggingPreference(context, next.debug)
        RootSnapshotCache.invalidate()
        TargetsCache.invalidate()
        DashboardCache.invalidate()
        StatisticsCache.invalidate()
    }
    return result.copy(targetRestartRecommended = result.ok && targetRestartRecommended)
}

private fun runActivation(
    @Suppress("UNUSED_PARAMETER") context: Context,
    changed: Boolean,
): AgentMutationResult = runActivationCommand(prefix = null, changed = changed)

private fun runActivationCommand(
    prefix: String?,
    changed: Boolean,
): AgentMutationResult {
    val parts =
        listOfNotNull(
            prefix,
            ConfigChannels.reconcileCommand(),
            "if [ -x $PORTS_ACTIVATOR ]; then $PORTS_ACTIVATOR; fi",
        )
    val (exit, output) = suExec(parts.joinToString(" ; "))
    return if (exit == 0) {
        AgentMutationResult(ok = true, message = "Activation completed", changed = changed)
    } else {
        AgentMutationResult(
            ok = false,
            message = "Root command failed with exit=$exit: ${output.trim()}",
            changed = false,
        )
    }
}

private fun buildProtectionState(
    context: Context,
    snapshot: TargetsSnapshot,
): AgentProtectionState {
    val canonical =
        snapshot.canonicalConfig
            ?: buildCanonicalConfigFromTargetsSnapshot(snapshot, debug = isEnabledInPrefs(context))
    return AgentProtectionState(
        canonicalConfigJson = canonicalConfigJson(canonical),
        activeNativeBackend = snapshot.activeNativeBackendId?.name,
        nativeHookFamily = snapshot.nativeHookFamily.name.lowercase(),
        configuredApps =
            canonical.apps
                .toSortedMap()
                .map { (pkg, app) -> app.toAgentConfiguredApp(pkg) },
        settings = canonical.settings.toAgentCanonicalSettings(),
    )
}

private fun DashboardState.toAgentDashboardState(): AgentDashboardState {
    val errorCount = messages.count { it.severity == DashboardMessageSeverity.ERROR }
    val warningCount = messages.count { it.severity == DashboardMessageSeverity.WARNING }
    val infoCount = messages.count { it.severity == DashboardMessageSeverity.INFO }
    return AgentDashboardState(
        heroStatus = computeHeroStatus(this, errorCount, warningCount).name,
        activeModuleCount = activeModuleCount(this),
        totalModuleCount = 3,
        errorCount = errorCount,
        warningCount = warningCount,
        infoCount = infoCount,
        activeNativeBackend = nativeBackend.id?.name,
        modules =
            listOf(
                lsposed.toAgentModuleState(),
                nativeBackend.toAgentModuleState(),
                ports.toAgentModuleState(id = "ports", layer = "ports", backend = "Ports"),
            ),
        protection = protection.toAgentProtectionSummary(),
        messages = messages.map { AgentDashboardMessage(it.severity.name.lowercase(), it.text) },
    )
}

private fun LsposedState.toAgentModuleState(): AgentModuleState =
    when (this) {
        LsposedState.NotInstalled -> {
            AgentModuleState(id = "lsposed", layer = "java", backend = "LSPosed", state = "not_installed")
        }

        is LsposedState.InstalledInactive -> {
            AgentModuleState(
                id = "lsposed",
                layer = "java",
                backend = "LSPosed",
                state = "installed_inactive",
                version = version,
            )
        }

        is LsposedState.NeedsReboot -> {
            AgentModuleState(
                id = "lsposed",
                layer = "java",
                backend = "LSPosed",
                state = "needs_reboot",
                version = version,
            )
        }

        is LsposedState.Active -> {
            AgentModuleState(
                id = "lsposed",
                layer = "java",
                backend = "LSPosed",
                state = "active",
                version = version,
                targetCount = targetCount,
            )
        }
    }

private fun DisplayNativeBackend.toAgentModuleState(): AgentModuleState =
    state.toAgentModuleState(
        id = id?.name?.lowercase() ?: "native",
        layer = "native",
        backend =
            when (id) {
                NativeBackendId.Kmod -> "Kmod"
                NativeBackendId.Kpm -> "KPM"
                NativeBackendId.Zygisk -> "Zygisk"
                null -> "Native"
            },
    )

private fun ModuleState.toAgentModuleState(
    id: String,
    layer: String,
    backend: String,
): AgentModuleState =
    when (this) {
        ModuleState.NotInstalled -> {
            AgentModuleState(id = id, layer = layer, backend = backend, state = "not_installed")
        }

        is ModuleState.Installed -> {
            AgentModuleState(
                id = id,
                layer = layer,
                backend = backend,
                state = if (active) "active" else "installed_inactive",
                version = version,
                targetCount = targetCount,
                reason = brokenReason?.name,
            )
        }
    }

private fun ProtectionCheck.toAgentProtectionSummary(): AgentProtectionSummary =
    when (this) {
        ProtectionCheck.NoVpn -> {
            AgentProtectionSummary(state = "vpn_off")
        }

        ProtectionCheck.NeedsRestart -> {
            AgentProtectionSummary(state = "needs_restart")
        }

        is ProtectionCheck.Checked -> {
            AgentProtectionSummary(
                state = "checked",
                native = native.toAgentStatus(),
                java = java.toAgentStatus(),
                nativePassed = (native as? NativeResult.Fail)?.passed,
                nativeFailed = (native as? NativeResult.Fail)?.failed,
                javaFailed = (java as? JavaResult.Fail)?.failedChecks,
            )
        }
    }

private fun NativeResult.toAgentStatus(): String =
    when (this) {
        NativeResult.Ok -> "ok"
        NativeResult.NoModule -> "no_module"
        is NativeResult.Fail -> "fail"
    }

private fun JavaResult.toAgentStatus(): String =
    when (this) {
        JavaResult.Ok -> "ok"
        JavaResult.HooksInactive -> "hooks_inactive"
        is JavaResult.Fail -> "fail"
    }

private fun CheckResults.toAgentDiagnosticsReport(): AgentDiagnosticsReport {
    val score = all.score()
    return AgentDiagnosticsReport(
        state = "ready",
        score = AgentCheckScore(score.passed, score.total),
        nativeChecks = nativeAll.map(CheckResult::toAgentCheckResult),
        javaChecks = java.map(CheckResult::toAgentCheckResult),
    )
}

private fun CheckResult.toAgentCheckResult(): AgentCheckResult =
    AgentCheckResult(
        name = name,
        status =
            when (passed) {
                true -> "pass"
                false -> "fail"
                null -> "info"
            },
        detail = detail,
    )

private fun StatisticsState.toAgentStatisticsState(selfPackage: String? = null): AgentStatisticsState {
    val apps = buildAppProbeStats(this, selfPackage)
    return AgentStatisticsState(
        hasAnyData = hasAnyData,
        activeBackendCount = activeBackendCount,
        totalRows = totalRows,
        totalCount = totalCount.toString(),
        appCount = apps.size,
        methodCount = apps.flatMap { it.byMethod.keys }.toSet().size,
        backends = backends.map(BackendStatistics::toAgentBackendStatistics),
        apps = apps.map(AppProbeStats::toAgentAppProbeStats),
    )
}

private fun BackendStatistics.toAgentBackendStatistics(): AgentBackendStatistics =
    AgentBackendStatistics(
        backend = backend.name,
        status = statisticsStatusName(),
        hookedCount = hookedCount,
        totalCount = totalCount.toString(),
        unavailableReason = unavailableReason?.name,
        rows = rows.map(StatisticsRow::toAgentStatisticRow),
    )

private fun BackendStatistics.statisticsStatusName(): String =
    when {
        unavailableReason != null -> "unavailable"

        status == null -> "no_data"

        status.error ==
            HookIds.StatusError.OK.code
                .toLong()
        -> "ok"

        status.error ==
            HookIds.StatusError.PARTIAL_HOOKS.code
                .toLong()
        -> "partial"

        else -> "error"
    }

private fun StatisticsRow.toAgentStatisticRow(): AgentStatisticRow =
    AgentStatisticRow(
        uid = uid,
        packageNames = packageNames,
        hookId = hookId,
        hookName = hook?.hookName,
        count = count.toULong().toString(),
    )

private fun AppProbeStats.toAgentAppProbeStats(): AgentAppProbeStats =
    AgentAppProbeStats(
        uid = uid,
        packageNames = packageNames,
        totalCount = total.toString(),
        surfaces = surfaces.sortedBy(MethodSurface::name).map(MethodSurface::agentName),
        methods =
            byMethod.entries
                .sortedByDescending { it.value }
                .map { (method, count) ->
                    AgentDetectionMethodStats(
                        method = method.name,
                        surface = method.surface.agentName(),
                        count = count.toString(),
                    )
                },
        hooks =
            byHook.entries
                .sortedByDescending { it.value }
                .map { (hook, count) ->
                    val method = DetectionMethod.of(hook)
                    AgentDetectionHookStats(
                        hookId = hook.id.toLong(),
                        hookName = hook.hookName,
                        hookNote = hook.note,
                        method = method.name,
                        surface = method.surface.agentName(),
                        count = count.toString(),
                    )
                },
    )

private fun MethodSurface.agentName(): String = name.lowercase()

private fun Map<Pair<Long, Long>, Long>.toAgentStatisticsCaptureBaseline(): AgentStatisticsCaptureBaseline =
    AgentStatisticsCaptureBaseline(
        counters =
            entries
                .sortedWith(compareBy<Map.Entry<Pair<Long, Long>, Long>> { it.key.first }.thenBy { it.key.second })
                .map { (key, count) ->
                    AgentStatisticCounterSnapshot(
                        uid = key.first,
                        hookId = key.second,
                        count = count,
                    )
                },
    )

private fun AgentStatisticsCaptureBaseline.toCounterMap(): Map<Pair<Long, Long>, Long> =
    counters.associate { (it.uid to it.hookId) to it.count }

private fun CaptureDiff.toAgentStatisticsCaptureDiff(): AgentStatisticsCaptureDiff =
    AgentStatisticsCaptureDiff(
        backendReset = backendReset,
        apps = apps.map(AppProbeStats::toAgentAppProbeStats),
    )

private fun CanonicalApp.toAgentConfiguredApp(packageName: String): AgentConfiguredApp =
    AgentConfiguredApp(
        packageName = packageName,
        java = java,
        javaHooks = javaHooks,
        native = native.enabled,
        nativeHooks =
            AgentNativeHookOverrides(
                kernel = native.overrides.kernel,
                zygisk = native.overrides.zygisk,
            ),
        appHiding = appHiding,
        ports = ports,
        portPolicy = portPolicy.toAgentPortPolicy(),
        hidden = hidden,
    )

private fun CanonicalSettings.toAgentCanonicalSettings(): AgentCanonicalSettings =
    AgentCanonicalSettings(
        rememberSuperkey = rememberSuperkey,
        autoHideVpnServices = autoHideVpnServices,
        autoHideVpnName = autoHideVpnName,
        autoHiddenPackages = autoHiddenPackages.toList(),
    )

private fun PortPolicy?.toAgentPortPolicy(): AgentPortPolicy? {
    if (this == null) return null
    return AgentPortPolicy(
        mode = mode.jsonName,
        preset = preset,
        rules = rules.map(PortRule::toAgentPortRule),
    )
}

private fun PortRule.toAgentPortRule(): AgentPortRule =
    AgentPortRule(
        protocol = protocol.jsonName,
        start = start,
        end = end,
    )

private fun AppSummary.toAgentInstalledApp(
    userNames: Map<Int, String>,
    configured: AgentConfiguredApp?,
): AgentInstalledApp =
    AgentInstalledApp(
        packageName = packageName,
        label = labelWithUsers(label, userIds, userNames),
        system = isSystem,
        userIds = userIds.map(Int::toString),
        declaresVpnService = declaresVpnService,
        nameContainsVpn = nameContainsVpn,
        roles = configured?.roles().orEmpty(),
    )

private fun AgentConfiguredApp.roles(): List<String> =
    buildList {
        if (java) add("java")
        if (native) add("native")
        if (appHiding) add("app_hiding")
        if (ports) add("ports")
        if (hidden) add("hidden")
    }

private fun CanonicalConfig.withApp(
    packageName: String,
    app: CanonicalApp,
): CanonicalConfig {
    val nextApps =
        if (app.hasAnyRole) {
            apps + (packageName to app)
        } else {
            apps - packageName
        }
    return copy(apps = nextApps.toSortedMap())
}
