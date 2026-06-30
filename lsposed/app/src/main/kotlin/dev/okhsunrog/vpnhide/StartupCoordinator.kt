package dev.okhsunrog.vpnhide

import android.content.Context
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
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
    private val prepareSelfTargetsCommand: (String) -> SelfTargetPreparation = ::ensureSelfInTargets,
    private val cleanupZygiskStatus: (Context, String?) -> Unit = ::cleanupStaleZygiskStatus,
    private val seedRootSnapshotPackages: (String?) -> Unit = RootSnapshotCache::seedPmPackages,
    private val markStartupEvent: (String) -> Unit = StartupTrace::mark,
    private val reconcileRuntimeConfig: (RootSnapshot) -> Unit = { runRuntimeConfigReconcile(appContext, it) },
) {
    private val _selfTargetState = MutableStateFlow<StartupSelfTargetState>(StartupSelfTargetState.Preparing)
    val selfTargetState: StateFlow<StartupSelfTargetState> = _selfTargetState.asStateFlow()

    // The runtime channels carry a `vpnhide 1 config` snapshot derived by the
    // activator from canonical JSON. A once-per-session reconcile is enough
    // (Save / the debug toggle re-run the activator on their own afterwards).
    private val reconcileStarted =
        java.util.concurrent.atomic
            .AtomicBoolean(false)

    suspend fun prepareSelfTargets() {
        _selfTargetState.value = StartupSelfTargetState.Preparing
        markStartupEvent("self_targets_start")
        val preparation =
            withContext(Dispatchers.IO) {
                val next = prepareSelfTargetsCommand(appContext.packageName)
                if (next.rootAvailable) {
                    seedRootSnapshotPackages(next.pmPackages)
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
        if (!selfNeedsRestart) DiagnosticsCache.run(scope, appContext)
    }

    fun ensureProtectionCacheAfterRootSnapshot(
        scope: CoroutineScope,
        selfNeedsRestart: Boolean?,
        rootSnapshot: RootSnapshot?,
    ) {
        if (selfNeedsRestart != null && rootSnapshot != null) {
            TargetsCache.ensureLoaded(scope, appContext)
            if (reconcileStarted.compareAndSet(false, true)) {
                scope.launch(Dispatchers.IO) { reconcileRuntimeConfig(rootSnapshot) }
            }
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
