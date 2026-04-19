package dev.okhsunrog.vpnhide

import android.content.Context
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.graphics.drawable.Drawable
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Common per-installed-app fields used by every Protection screen
 * (Tun targets, App hiding, Ports). The three screens merge this with
 * their own per-screen toggle state at render time.
 */
internal data class AppSummary(
    val packageName: String,
    val label: String,
    val icon: Drawable?,
    val isSystem: Boolean,
)

/**
 * App-scoped cache for the installed-app list. Loaded asynchronously
 * at startup; Protection screens subscribe to `apps` and render
 * instantly on tab switch.
 *
 * [refreshCounter] increments on every refresh — screens that maintain
 * their own per-screen state (targets.txt / observer files etc.) key
 * their reload `LaunchedEffect` on it, so the TopBar refresh button
 * rehydrates *everything*, not just the package+icon cache.
 */
internal object AppListCache {
    private val _apps = MutableStateFlow<List<AppSummary>?>(null)
    val apps: StateFlow<List<AppSummary>?> = _apps.asStateFlow()

    private val _refreshCounter = MutableStateFlow(0)
    val refreshCounter: StateFlow<Int> = _refreshCounter.asStateFlow()

    private val _loading = MutableStateFlow(false)
    val loading: StateFlow<Boolean> = _loading.asStateFlow()

    private var inflight: Job? = null

    /** Kick off an initial load if not already loaded or loading. */
    fun ensureLoaded(
        scope: CoroutineScope,
        context: Context,
    ) {
        if (_apps.value != null || inflight?.isActive == true) return
        inflight = scope.launch { reload(context.applicationContext) }
    }

    /** Force a reload and bump the refresh counter so screens re-read
     * their per-screen state (targets.txt / observer files etc.) too.
     */
    fun refresh(
        scope: CoroutineScope,
        context: Context,
    ) {
        inflight?.cancel()
        inflight = scope.launch { reload(context.applicationContext) }
    }

    private suspend fun reload(appContext: Context) {
        _loading.value = true
        try {
            val loaded =
                withContext(Dispatchers.IO) {
                    val pm = appContext.packageManager
                    pm
                        .getInstalledApplications(0)
                        .map { info ->
                            AppSummary(
                                packageName = info.packageName,
                                label = info.loadLabel(pm).toString(),
                                icon = runCatching { pm.getApplicationIcon(info) }.getOrNull(),
                                isSystem = (info.flags and ApplicationInfo.FLAG_SYSTEM) != 0,
                            )
                        }.sortedBy { it.label.lowercase() }
                }
            _apps.value = loaded
            _refreshCounter.value = _refreshCounter.value + 1
        } finally {
            _loading.value = false
        }
    }
}
