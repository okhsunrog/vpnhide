package dev.okhsunrog.vpnhide.startup

import android.content.Context
import dev.okhsunrog.vpnhide.BuildConfig
import dev.okhsunrog.vpnhide.CanonicalConfigRepository
import dev.okhsunrog.vpnhide.ConfigCoordinatorMode
import dev.okhsunrog.vpnhide.DashboardCache
import dev.okhsunrog.vpnhide.DashboardState
import dev.okhsunrog.vpnhide.PackageInventorySeed
import dev.okhsunrog.vpnhide.RootSnapshot
import dev.okhsunrog.vpnhide.RootSnapshotCache
import dev.okhsunrog.vpnhide.SelfTargetFailureKind
import dev.okhsunrog.vpnhide.SelfTargetPreparation
import dev.okhsunrog.vpnhide.UpdateCheckCache
import dev.okhsunrog.vpnhide.cleanupStaleZygiskStatus
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticsCache
import dev.okhsunrog.vpnhide.diagnostics.RoutingGateCache
import dev.okhsunrog.vpnhide.ensureSelfInTargets
import dev.okhsunrog.vpnhide.next
import dev.okhsunrog.vpnhide.picker.AppAutoHideSignal
import dev.okhsunrog.vpnhide.picker.AppListCache
import dev.okhsunrog.vpnhide.picker.TargetsCache
import dev.okhsunrog.vpnhide.picker.toAutoHideSignal
import dev.okhsunrog.vpnhide.reconcileAutoHiddenPackages
import dev.okhsunrog.vpnhide.runRuntimeConfigReconcile
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
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
    private val initializeConfig: suspend () -> Unit = { CanonicalConfigRepository.initialize(appContext) },
    private val prepareSelfTargetsCommand: suspend (String) -> SelfTargetPreparation = ::ensureSelfInTargets,
    private val cleanupZygiskStatus: (Context, String?) -> Unit = ::cleanupStaleZygiskStatus,
    private val seedRootSnapshotInventory: (PackageInventorySeed?) -> Unit = RootSnapshotCache::seedPackageInventory,
    private val seedRootSnapshot: (Map<String, String>) -> Unit = RootSnapshotCache::seedSnapshot,
    private val markStartupEvent: (String) -> Unit = StartupTrace::mark,
    private val reconcileRuntimeConfig: suspend () -> Unit = { runRuntimeConfigReconcile() },
    private val reconcileAutoHidden: suspend (List<AppAutoHideSignal>) -> Unit =
        { signals -> reconcileAutoHiddenPackages(appContext, signals) },
) {
    private val owner = CanonicalConfigRepository.processScope
    private val prepareMutex = Mutex()
    private var prepareTask: Deferred<Unit>? = null

    companion object {
        @Volatile private var instance: StartupCoordinator? = null

        fun forProcess(context: Context): StartupCoordinator =
            synchronized(this) {
                instance ?: StartupCoordinator(context.applicationContext).also { coordinator ->
                    instance = coordinator
                    coordinator.observeAvailability()
                }
            }
    }

    private fun observeAvailability() {
        owner.launch {
            CanonicalConfigRepository.state.map { it.mode }.distinctUntilChanged().collect { mode ->
                if (mode == ConfigCoordinatorMode.Open) prepareSelfTargets(force = true)
            }
        }
    }

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

    suspend fun prepareSelfTargets(force: Boolean = false) {
        val task =
            prepareMutex.withLock {
                if (force && prepareTask?.isCompleted == true) prepareTask = null
                prepareTask ?: owner.async { prepareSelfTargetsOnce() }.also { prepareTask = it }
            }
        task.await()
    }

    private suspend fun prepareSelfTargetsOnce() {
        _selfTargetState.value = StartupSelfTargetState.Preparing
        markStartupEvent("self_targets_start")
        initializeConfig()
        markStartupEvent("config_init_done")
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
                    // A read that wrote nothing seeds the whole snapshot (no second
                    // root shell); after a write only the package inventory survives.
                    val sections = next.sections
                    if (sections != null) seedRootSnapshot(sections) else seedRootSnapshotInventory(inventory)
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
        scope.launch { prepareSelfTargets(force = true) }
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
        startAutoHideReconcile()
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
    private fun startAutoHideReconcile() {
        if (!autoHideReconcileStarted.compareAndSet(false, true)) return
        owner.launch(Dispatchers.IO) {
            AppListCache.apps.filterNotNull().collect { apps ->
                reconcileAutoHidden(apps.map { it.toAutoHideSignal() })
            }
        }
    }

    /**
     * Protection parses the shared root snapshot from memory as soon as it exists.
     * The runtime reconcile (a forced native activation) waits for [dashboardReady]:
     * its completion invalidates every root observation, and running it while the
     * first Dashboard derivation and the diagnostic suite are still in flight put a
     * second root snapshot plus a repeated derivation on the cold-start critical path
     * (measured at roughly 1.4 s on Pixel 8 Pro). After first paint the same
     * refresh happens in the background.
     */
    fun ensureProtectionCacheAfterRootSnapshot(
        scope: CoroutineScope,
        selfNeedsRestart: Boolean?,
        rootSnapshot: RootSnapshot?,
        dashboardReady: Boolean,
    ) {
        if (selfNeedsRestart != null && rootSnapshot != null) {
            TargetsCache.ensureLoaded(scope, appContext)
            if (dashboardReady && reconcileStarted.compareAndSet(false, true)) {
                owner.launch(Dispatchers.IO) { reconcileRuntimeConfigNow() }
            }
        }
    }

    private suspend fun reconcileRuntimeConfigNow() {
        reconcileRuntimeConfig()
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
