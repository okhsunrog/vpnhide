package dev.okhsunrog.vpnhide.startup

import android.content.Context
import dev.okhsunrog.vpnhide.BuildConfig
import dev.okhsunrog.vpnhide.CanonicalConfig
import dev.okhsunrog.vpnhide.CanonicalConfigRepository
import dev.okhsunrog.vpnhide.DashboardCache
import dev.okhsunrog.vpnhide.DashboardState
import dev.okhsunrog.vpnhide.LogTags
import dev.okhsunrog.vpnhide.PackageInventorySeed
import dev.okhsunrog.vpnhide.RootSnapshot
import dev.okhsunrog.vpnhide.RootSnapshotCache
import dev.okhsunrog.vpnhide.SelfTargetFailureKind
import dev.okhsunrog.vpnhide.SelfTargetPreparation
import dev.okhsunrog.vpnhide.UpdateCheckCache
import dev.okhsunrog.vpnhide.VpnHideLog
import dev.okhsunrog.vpnhide.canonicalConfigForStartupDebugReconcile
import dev.okhsunrog.vpnhide.cleanupStaleZygiskStatus
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticsCache
import dev.okhsunrog.vpnhide.diagnostics.RoutingGateCache
import dev.okhsunrog.vpnhide.ensureSelfInTargets
import dev.okhsunrog.vpnhide.next
import dev.okhsunrog.vpnhide.picker.AppAutoHideSignal
import dev.okhsunrog.vpnhide.picker.AppListCache
import dev.okhsunrog.vpnhide.picker.TargetsCache
import dev.okhsunrog.vpnhide.picker.parseTargetsSnapshot
import dev.okhsunrog.vpnhide.picker.toAutoHideSignal
import dev.okhsunrog.vpnhide.reconcileAutoHiddenPackages
import dev.okhsunrog.vpnhide.runRuntimeConfigReconcile
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

internal sealed interface StartupSelfTargetState {
    data object Preparing : StartupSelfTargetState

    data class Ready(
        val selfNeedsRestart: Boolean,
    ) : StartupSelfTargetState

    data class Failed(
        val kind: SelfTargetFailureKind,
        val detail: String,
    ) : StartupSelfTargetState
}

internal class StartupCoordinator(
    private val appContext: Context,
    private val appVersionName: String = BuildConfig.VERSION_NAME,
    private val prepareSelfTargetsCommand: suspend (String) -> SelfTargetPreparation = ::ensureSelfInTargets,
    private val cleanupZygiskStatus: (Context, String?) -> Unit = ::cleanupStaleZygiskStatus,
    private val seedRootSnapshotInventory: (PackageInventorySeed?) -> Unit = RootSnapshotCache::seedPackageInventory,
    private val markStartupEvent: (String) -> Unit = StartupTrace::mark,
    private val reconcileRuntimeConfig: () -> Unit = { runRuntimeConfigReconcile() },
    private val reconcileAutoHidden: suspend (CanonicalConfig, List<AppAutoHideSignal>) -> Unit =
        { config, signals -> reconcileAutoHiddenPackages(appContext, config, signals) },
    private val loadCanonicalConfig: suspend () -> CanonicalConfig? =
        { parseTargetsSnapshot(RootSnapshotCache.getOrLoad()).canonicalConfig },
) {
    private val _selfTargetState = MutableStateFlow<StartupSelfTargetState>(StartupSelfTargetState.Preparing)
    val selfTargetState: StateFlow<StartupSelfTargetState> = _selfTargetState.asStateFlow()

    // The runtime channels carry a `vpnhide 2 config` snapshot derived by the
    // activator from canonical JSON. A once-per-session reconcile is enough
    // (Save / the debug toggle re-run the activator on their own afterwards).
    private val reconcileStarted =
        java.util.concurrent.atomic
            .AtomicBoolean(false)

    // The auto-hide reconcile observer is a single session-long collector; guard
    // against starting a second one if ensureInitialCaches re-runs.
    private val autoHideReconcileStarted =
        java.util.concurrent.atomic
            .AtomicBoolean(false)

    suspend fun prepareSelfTargets() {
        _selfTargetState.value = StartupSelfTargetState.Preparing
        markStartupEvent("self_targets_start")
        val preparation =
            withContext(Dispatchers.IO) {
                val next = prepareSelfTargetsCommand(appContext.packageName)
                if (next.rootAvailable) {
                    val inventory =
                        if (next.pmPackages != null && next.pmUsers != null) {
                            PackageInventorySeed(next.pmPackages, next.pmUsers)
                        } else {
                            null
                        }
                    seedRootSnapshotInventory(inventory)
                    cleanupZygiskStatus(appContext, next.currentBootId)
                }
                next
            }
        markStartupEvent("self_targets_done")
        if (preparation.rootAvailable) {
            _selfTargetState.value = StartupSelfTargetState.Ready(preparation.selfNeedsRestart)
        } else {
            markStartupEvent("self_targets_failed")
            _selfTargetState.value =
                StartupSelfTargetState.Failed(
                    kind = preparation.failureKind,
                    detail = preparation.error ?: "root preparation failed",
                )
        }
    }

    fun retrySelfTargets(scope: CoroutineScope) {
        scope.launch { prepareSelfTargets() }
    }

    fun ensureInitialCaches(
        scope: CoroutineScope,
        selfNeedsRestart: Boolean,
    ) {
        AppListCache.ensureLoaded(scope, appContext)
        DashboardCache.ensureLoaded(scope, appContext, selfNeedsRestart)
        // Seed the shared routing gate as early as selfNeedsRestart is known — this is
        // also the earliest point the process-scoped VPN transport watcher (Phase 3)
        // can find inputs to refresh against.
        RoutingGateCache.ensureLoaded(scope, appContext, selfNeedsRestart)
        // The cache parks at Blocked(NEEDS_RESTART) itself when selfNeedsRestart — this
        // is also the first run() call, so it stamps the process-constant flag.
        DiagnosticsCache.run(scope, appContext, selfNeedsRestart)
        startAutoHideReconcile(scope)
    }

    /**
     * Once per session, watch the installed-app list and re-materialize the
     * auto-hidden VPN-app set whenever it (re)loads — at cold start and after a
     * Hiding-tab Refresh (which force-reloads [AppListCache]). This keeps a
     * newly-installed VPN app hidden from observers without the user having to
     * open the picker and Save. The write itself is idempotent: it only touches
     * disk when the auto-hidden set actually changed.
     *
     * Self-target preparation runs first and always writes the canonical config,
     * so by the time the app list emits, the config is non-null — a null read
     * means no root, and the reconcile is simply skipped.
     */
    private fun startAutoHideReconcile(scope: CoroutineScope) {
        if (!autoHideReconcileStarted.compareAndSet(false, true)) return
        scope.launch(Dispatchers.IO) {
            AppListCache.apps.filterNotNull().collect { apps ->
                val config = loadCanonicalConfig() ?: return@collect
                reconcileAutoHidden(config, apps.map { it.toAutoHideSignal() })
            }
        }
    }

    fun ensureProtectionCacheAfterRootSnapshot(
        scope: CoroutineScope,
        selfNeedsRestart: Boolean?,
        rootSnapshot: RootSnapshot?,
    ) {
        if (selfNeedsRestart != null && rootSnapshot != null) {
            TargetsCache.ensureLoaded(scope, appContext)
            if (reconcileStarted.compareAndSet(false, true)) {
                scope.launch(Dispatchers.IO) { reconcileRuntimeConfigNow(rootSnapshot) }
            }
        }
    }

    private suspend fun reconcileRuntimeConfigNow(rootSnapshot: RootSnapshot) {
        val canonicalConfig = parseTargetsSnapshot(rootSnapshot).canonicalConfig
        val reconciled = canonicalConfig?.let { canonicalConfigForStartupDebugReconcile(it) }
        // A capture interrupted mid-flight left effective `debug` out of sync with
        // the user's `debugSwitch`: write the healed config and run the activator
        // once. Otherwise nothing needs writing — just re-run the activator to pick
        // up the current file state. Never both (the write command already runs it).
        if (reconciled == null) {
            reconcileRuntimeConfig()
            return
        }
        val result = CanonicalConfigRepository.commit(reconciled)
        if (!result.succeeded) {
            VpnHideLog.w(
                LogTags.STARTUP,
                "startup debug reconcile failed (exit=${result.exitCode}): ${result.output.trim()}",
            )
            return
        }
    }

    fun ensureUpdateFresh(scope: CoroutineScope) {
        UpdateCheckCache.ensureFresh(scope, appVersionName)
    }

    fun refreshDashboard(
        scope: CoroutineScope,
        selfNeedsRestart: Boolean,
    ) {
        DashboardCache.refresh(scope, appContext, selfNeedsRestart)
        UpdateCheckCache.refresh(scope, appVersionName)
    }

    fun refreshProtection(scope: CoroutineScope) {
        AppListCache.refresh(scope, appContext)
        TargetsCache.refresh(scope, appContext)
    }

    fun isUiReady(
        dashboardState: DashboardState?,
        dashboardError: String?,
    ): Boolean =
        when (selfTargetState.value) {
            is StartupSelfTargetState.Failed -> true
            StartupSelfTargetState.Preparing -> false
            is StartupSelfTargetState.Ready -> dashboardState != null || dashboardError != null
        }
}
