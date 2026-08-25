package dev.okhsunrog.vpnhide.statistics

import dev.okhsunrog.vpnhide.LogTags
import dev.okhsunrog.vpnhide.RootSnapshotCache
import dev.okhsunrog.vpnhide.StateCache
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.withContext

internal object StatisticsCache : StateCache<StatisticsState>(
    traceName = "statistics_state",
    logTag = LogTags.STATISTICS,
) {
    val state: StateFlow<StatisticsState?> get() = value

    fun ensureLoaded(scope: CoroutineScope) {
        ensure(scope)
    }

    fun refresh(scope: CoroutineScope) {
        RootSnapshotCache.invalidate()
        forceRefresh(scope)
    }

    override suspend fun load(force: Boolean): StatisticsState {
        val rootSnapshot =
            if (force) RootSnapshotCache.refresh() else RootSnapshotCache.getOrLoad()
        return withContext(Dispatchers.IO) {
            buildStatisticsState(rootSnapshot)
        }
    }
}
