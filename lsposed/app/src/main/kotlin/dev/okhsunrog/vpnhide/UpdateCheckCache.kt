package dev.okhsunrog.vpnhide

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * In-memory cache for the GitHub update-check result. Two jobs:
 *
 * 1. Avoid re-hitting the GitHub API on every Dashboard tab switch —
 *    previously each composition re-ran `checkForUpdate`, so toggling
 *    Dashboard ↔ Protection a few times burned through the 60/h
 *    anonymous API rate limit.
 * 2. Refresh quietly when the app comes back to the foreground after
 *    sitting in the background long enough (see [STALE_MS]). Users who
 *    keep the app minimized for a day still see a fresh state when
 *    they come back.
 *
 * No WorkManager, no background alarms, no notifications — this is
 * purely reactive to app lifecycle. If the app is actually killed the
 * cache dies with it and the next launch re-checks as usual.
 */
internal object UpdateCheckCache {
    /** 6 hours. Arbitrary — roughly matches how often a release cadence
     * is meaningful. Not configurable on purpose; if you want newer,
     * tap Refresh.
     */
    private const val STALE_MS = 6L * 60 * 60 * 1000

    private val _info = MutableStateFlow<UpdateInfo?>(null)
    val info: StateFlow<UpdateInfo?> = _info.asStateFlow()

    // `null` = never checked this session. Timestamps are wall-clock
    // millis; drift doesn't matter — all we need is "was X hours ago".
    private var lastCheckMs: Long? = null
    private var inflight: Job? = null

    fun ensureFresh(
        scope: CoroutineScope,
        currentVersion: String,
    ) {
        val last = lastCheckMs
        val fresh = last != null && System.currentTimeMillis() - last < STALE_MS
        if (fresh || inflight?.isActive == true) return
        inflight = scope.launch { run(currentVersion) }
    }

    fun refresh(
        scope: CoroutineScope,
        currentVersion: String,
    ) {
        inflight?.cancel()
        inflight = scope.launch { run(currentVersion) }
    }

    private suspend fun run(currentVersion: String) {
        val result = withContext(Dispatchers.IO) { checkForUpdate(currentVersion) }
        _info.value = result
        lastCheckMs = System.currentTimeMillis()
    }
}
