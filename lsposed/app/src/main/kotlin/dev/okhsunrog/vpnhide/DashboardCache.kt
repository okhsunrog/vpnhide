package dev.okhsunrog.vpnhide

import android.content.Context
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.withContext

/**
 * App-scoped cache for the Dashboard's computed state. Previously
 * `DashboardScreen` ran `loadDashboardState()` in its own
 * `LaunchedEffect(Unit)` on every composition — which means every
 * tab switch re-ran all the module-prop / target / kprobes / SELinux
 * checks via `suExec`. Cache them once at startup; refresh them
 * explicitly on user action or after a Save.
 *
 * The Dashboard screen reads [state] and shows the previous value
 * while a refresh is in flight so tab switches feel instant even when
 * data changes underneath.
 */
internal object DashboardCache : StateCache<DashboardState>(
    traceName = "dashboard_state",
    logTag = LogTags.DASHBOARD,
) {
    val state: StateFlow<DashboardState?> get() = value

    @Volatile private var appContext: Context? = null

    @Volatile private var selfNeedsRestart: Boolean = false

    fun ensureLoaded(
        scope: CoroutineScope,
        context: Context,
        selfNeedsRestart: Boolean,
    ) {
        this.appContext = context.applicationContext
        this.selfNeedsRestart = selfNeedsRestart
        ensure(scope)
    }

    fun refresh(
        scope: CoroutineScope,
        context: Context,
        selfNeedsRestart: Boolean,
    ) {
        this.appContext = context.applicationContext
        this.selfNeedsRestart = selfNeedsRestart
        RootSnapshotCache.invalidate()
        forceRefresh(scope)
    }

    override suspend fun load(force: Boolean): DashboardState {
        val context = requireNotNull(appContext) { "DashboardCache.load before ensureLoaded/refresh" }
        val rootSnapshot =
            if (force) RootSnapshotCache.refresh() else RootSnapshotCache.getOrLoad()
        return withContext(Dispatchers.IO) {
            loadDashboardState(context, selfNeedsRestart, rootSnapshot)
        }
    }
}
