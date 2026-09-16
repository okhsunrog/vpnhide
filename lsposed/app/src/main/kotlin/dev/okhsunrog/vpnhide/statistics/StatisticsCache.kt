package dev.okhsunrog.vpnhide.statistics

import dev.okhsunrog.vpnhide.LogTags
import dev.okhsunrog.vpnhide.ObservationRequest
import dev.okhsunrog.vpnhide.ProjectedStateFlow
import dev.okhsunrog.vpnhide.RootProjection
import dev.okhsunrog.vpnhide.RootSnapshotCache
import dev.okhsunrog.vpnhide.StateCache
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.withContext

internal object StatisticsCache : StateCache<RootProjection<StatisticsState>>(
    traceName = "statistics_state",
    logTag = LogTags.STATISTICS,
    source = RootSnapshotCache.dependency,
) {
    val state: StateFlow<StatisticsState?> = ProjectedStateFlow(value) { it?.value }

    fun ensureLoaded(scope: CoroutineScope) {
        ensure(scope)
    }

    fun refresh(scope: CoroutineScope) {
        forceRefresh(scope)
    }

    override suspend fun load(
        @Suppress("UNUSED_PARAMETER") request: ObservationRequest,
    ): RootProjection<StatisticsState> {
        val rootSnapshot =
            RootSnapshotCache.getOrLoad()
        return withContext(Dispatchers.IO) {
            RootProjection(rootSnapshot.observationId, rootSnapshot.generation, buildStatisticsState(rootSnapshot))
        }
    }
}
