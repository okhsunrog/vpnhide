package dev.okhsunrog.vpnhide

import android.content.Context
import android.net.ConnectivityManager
import dev.okhsunrog.vpnhide.debug.LogcatRecorder
import dev.okhsunrog.vpnhide.debug.StateContentOptions
import dev.okhsunrog.vpnhide.debug.VpnHideState
import dev.okhsunrog.vpnhide.debug.buildLsposedConfigText
import dev.okhsunrog.vpnhide.debug.buildVpnHideState
import dev.okhsunrog.vpnhide.debug.captureBootLsposedLogcat
import dev.okhsunrog.vpnhide.debug.exportDebug
import dev.okhsunrog.vpnhide.debug.isoNow
import dev.okhsunrog.vpnhide.debug.setDebugLoggingEnabled
import dev.okhsunrog.vpnhide.diagnostics.AppProbeStats
import dev.okhsunrog.vpnhide.diagnostics.CaptureDiff
import dev.okhsunrog.vpnhide.diagnostics.DetectionMethod
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticGate
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticsCache
import dev.okhsunrog.vpnhide.diagnostics.MethodSurface
import dev.okhsunrog.vpnhide.diagnostics.buildAppProbeStats
import dev.okhsunrog.vpnhide.diagnostics.buildHookDiagnosticsText
import dev.okhsunrog.vpnhide.diagnostics.diffCapture
import dev.okhsunrog.vpnhide.diagnostics.snapshotCounters
import dev.okhsunrog.vpnhide.generated.HookIds
import dev.okhsunrog.vpnhide.picker.AppListCache
import dev.okhsunrog.vpnhide.picker.AppSummary
import dev.okhsunrog.vpnhide.picker.TargetsSnapshot
import dev.okhsunrog.vpnhide.picker.applyAutoHiddenPackages
import dev.okhsunrog.vpnhide.picker.labelWithUsers
import dev.okhsunrog.vpnhide.picker.parseTargetsSnapshot
import dev.okhsunrog.vpnhide.picker.resolveHookSelection
import dev.okhsunrog.vpnhide.picker.resolveNativeHookSelection
import dev.okhsunrog.vpnhide.picker.toAutoHideSignal
import dev.okhsunrog.vpnhide.picker.updateManualHiddenPackages
import dev.okhsunrog.vpnhide.settings.removeConfiguredPackages
import dev.okhsunrog.vpnhide.statistics.BackendStatistics
import dev.okhsunrog.vpnhide.statistics.StatisticsRow
import dev.okhsunrog.vpnhide.statistics.StatisticsState
import dev.okhsunrog.vpnhide.statistics.buildStatisticsState
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File
import java.util.zip.ZipFile

/**
 * Transport-independent control surface used by the debug host bridge.
 *
 * Every mutating function writes the existing canonical JSON and then runs the
 * same activators used by the UI save/import paths. The Settings toggle only
 * gates the debug transport; these functions stay plain Kotlin entry points.
 */
internal object AgentControl {
    /**
     * The one canonical read for a Claude Code debugging session: the live dashboard
     * plus the full diagnostics report, per-module liveness, root-shell self-diagnosis,
     * the canonical config and a hook-counter snapshot — everything you would otherwise
     * gather from screenshots and a manual diagnostics export. [includeForensics] adds
     * the heavy blobs (dmesg / boot logcat / lsposed config / hook report / raw sections).
     */
    suspend fun getState(
        context: Context,
        refresh: Boolean? = null,
        options: StateContentOptions = StateContentOptions(),
    ): VpnHideState =
        withAppContext(context) { context ->
            val rootSnapshot = rootSnapshot(refresh == true)
            val dashboard = loadDashboardState(context, selfNeedsRestart = false, rootSnapshot = rootSnapshot)
            val statistics = buildStatisticsState(rootSnapshot).toAgentStatisticsState(selfPackage = context.packageName)
            val config = AgentBridgeJson.parseToJsonElement(canonicalConfigJson(currentCanonicalConfig(refresh = false)))

            // awaitTerminal carries the blocked-gate/failed reason instead of racing
            // on state.value. selfNeedsRestart=false is safe: the cache's sticky flag
            // keeps a UI-set NEEDS_RESTART from being cleared here.
            val terminal = DiagnosticsCache.awaitTerminal(context, selfNeedsRestart = false)
            val gate =
                when (terminal) {
                    is DiagnosticsCache.State.Blocked -> terminal.gate
                    is DiagnosticsCache.State.Ready -> DiagnosticGate.ROUTED
                    else -> null
                }
            val checkResults = (terminal as? DiagnosticsCache.State.Ready)?.results

            // Forensic blobs (shell snapshot, dmesg, boot logcat, lsposed config,
            // hook counters, raw sections) only on request — they run extra root
            // shells and bloat the live payload. Same options as the file export.
            val shellSnapshot = if (options.forensics) collectDebugShellSnapshot() else null
            buildVpnHideState(
                context = context,
                captureKind = "agent_bridge",
                generatedAt = isoNow(),
                selfNeedsRestart = false,
                rootSnapshot = rootSnapshot,
                shellSnapshot = shellSnapshot,
                gate = gate,
                checkResults = checkResults,
                dmesg = if (options.forensics) suExec("dmesg 2>/dev/null").second else "",
                logcat = "",
                bootLsposedLogcat = if (options.forensics) captureBootLsposedLogcat() else "",
                lsposedConfigDb = if (options.forensics) buildLsposedConfigText(context) else "",
                hookReport = shellSnapshot?.let { buildHookDiagnosticsText(context, it) },
                debugCapture = null,
                errors = emptyList(),
                dashboard = dashboard,
                config = config,
                statistics = statistics,
                options = options,
            )
        }

    /**
     * Create a separate opt-in ZIP with active boot/init_boot/vendor_boot kernel images.
     */
    suspend fun exportKernelImages(context: Context): AgentDebugZipExport =
        withAppContext(context) { context ->
            val connectivityManager =
                context.getSystemService(ConnectivityManager::class.java)
                    ?: error("ConnectivityManager unavailable")
            val file =
                exportDebug(
                    cm = connectivityManager,
                    context = context,
                    selfNeedsRestart = false,
                    options = StateContentOptions(forensics = true),
                    attachKernelImage = true,
                ) ?: error("Kernel image export failed")
            file.toAgentDebugZipExport()
        }

    /**
     * Start the same full-system-logcat recording session as Detailed diagnostics.
     */
    suspend fun startFullSystemLogcat(
        context: Context,
        selfNeedsRestart: Boolean? = null,
    ): AgentMutationResult =
        withAppContext(context) { context ->
            val file =
                LogcatRecorder.start(
                    context = context,
                    selfNeedsRestart = selfNeedsRestart == true,
                ) ?: error("Full system logcat recording failed to start")
            AgentMutationResult(
                ok = true,
                message = "Full system logcat recording started: ${file.absolutePath}",
            )
        }

    /**
     * Stop full-system-logcat recording and return the diagnostic ZIP metadata.
     */
    suspend fun stopFullSystemLogcat(context: Context): AgentDebugZipExport =
        withAppContext(context) { context ->
            val file = LogcatRecorder.stop(context) ?: error("No full system logcat recording is active")
            file.toAgentDebugZipExport()
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
     * List installed apps in the same shape used by the Apps picker.
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
            val protection = buildProtectionState(targetsSnapshot(refresh = false))
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
     * Import canonical JSON and immediately activate native/ports runtime state.
     *
     * @param json Canonical config JSON (the `/data/system/vpnhide_config.json` format).
     */
    suspend fun importCanonicalConfig(
        context: Context,
        json: String,
    ): AgentMutationResult =
        withAppContext(context) { context ->
            val canonical =
                parseImportedCanonicalConfig(json, context.packageName)
                    ?: throw IllegalArgumentException("Invalid canonical JSON")
            applyCanonicalConfig(context = context, canonical = canonical)
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
            val base = currentCanonicalConfig(refresh = true)
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
            val base = currentCanonicalConfig(refresh = true)
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
            // Derive base from the snapshot already fetched (not a second cache
            // read), so this read-modify-write can't depend on cache ordering.
            val base = snapshot.canonicalConfig ?: buildCanonicalConfigFromTargetsSnapshot(snapshot)
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
            val base = currentCanonicalConfig(refresh = true)
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
            val base = currentCanonicalConfig(refresh = true)
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
            val base = currentCanonicalConfig(refresh = true)
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
            val base = currentCanonicalConfig(refresh = true)
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
            setDebugLoggingEnabled(enabled)
            AgentMutationResult(ok = true, message = "Debug logging updated", changed = true)
        }

    /**
     * Re-run native and ports activators for the current canonical config.
     */
    suspend fun activateConfig(context: Context): AgentMutationResult =
        withAppContext(context) { context ->
            runActivation(changed = false)
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

private suspend fun currentCanonicalConfig(refresh: Boolean): CanonicalConfig {
    val snapshot = targetsSnapshot(refresh)
    return snapshot.canonicalConfig
        ?: buildCanonicalConfigFromTargetsSnapshot(snapshot)
}

private suspend fun applyCanonicalConfig(
    context: Context,
    canonical: CanonicalConfig,
    targetRestartRecommended: Boolean = true,
): AgentMutationResult {
    val next = canonicalConfigWithSelfTarget(canonical, context.packageName)
    val write =
        CanonicalConfigRepository.commit(
            next,
            activation = CanonicalActivation(native = true, ports = true),
        )
    val result = write.toAgentMutationResult(changed = true)
    return result.copy(targetRestartRecommended = result.ok && targetRestartRecommended)
}

private fun runActivation(changed: Boolean): AgentMutationResult = runActivationCommand(changed = changed)

private fun runActivationCommand(changed: Boolean): AgentMutationResult {
    val parts =
        listOf(
            ConfigChannels.reconcileCommand(),
            ConfigChannels.portsActivatorCommand(),
        )
    val (exit, output) = suExec(parts.joinToString(" && "))
    return CanonicalWriteResult(exit, output).toAgentMutationResult(changed)
}

private fun CanonicalWriteResult.toAgentMutationResult(changed: Boolean): AgentMutationResult =
    if (succeeded) {
        AgentMutationResult(ok = true, message = "Activation completed", changed = changed)
    } else {
        AgentMutationResult(
            ok = false,
            message = "Root command failed with exit=$exitCode: ${output.trim()}",
            changed = false,
        )
    }

private fun buildProtectionState(snapshot: TargetsSnapshot): AgentProtectionState {
    val canonical =
        snapshot.canonicalConfig
            ?: buildCanonicalConfigFromTargetsSnapshot(snapshot)
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

internal fun StatisticsState.toAgentStatisticsState(selfPackage: String? = null): AgentStatisticsState {
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

private fun File.toAgentDebugZipExport(): AgentDebugZipExport =
    AgentDebugZipExport(
        path = absolutePath,
        sizeBytes = length(),
        entries =
            ZipFile(this).use { zip ->
                zip
                    .entries()
                    .asSequence()
                    .map { it.name }
                    .sorted()
                    .toList()
            },
    )

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
        status.statusError == HookIds.StatusError.OK -> "ok"
        status.statusError == HookIds.StatusError.PARTIAL_HOOKS -> "partial"
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
