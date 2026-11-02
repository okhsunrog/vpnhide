package dev.okhsunrog.vpnhide.statistics

import dev.okhsunrog.vpnhide.CoreSessions
import dev.okhsunrog.vpnhide.LogTags
import dev.okhsunrog.vpnhide.ObservationRequest
import dev.okhsunrog.vpnhide.ObservationRuntime
import dev.okhsunrog.vpnhide.ProjectedStateFlow
import dev.okhsunrog.vpnhide.RootProjection
import dev.okhsunrog.vpnhide.RootSnapshotCache
import dev.okhsunrog.vpnhide.StateCache
import dev.okhsunrog.vpnhide.nativeStatusSnapshot
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.withContext

internal object StatisticsCache : StateCache<RootProjection<CounterTelemetry>>(
    traceName = "statistics_state",
    logTag = LogTags.STATISTICS,
    source = RootSnapshotCache.dependency,
) {
    val state: StateFlow<StatisticsState?> =
        combine(value, CoreSessions.updates) { cached, core -> cached?.value?.let { qualifyStatistics(it, core.nativeStatusSnapshot()) } }
            .stateIn(ObservationRuntime.scope, SharingStarted.Eagerly, null)

    fun ensureLoaded() {
        ensure()
    }

    fun refresh() {
        forceRefresh()
    }

    override suspend fun load(
        @Suppress("UNUSED_PARAMETER") request: ObservationRequest,
    ): RootProjection<CounterTelemetry> {
        val rootSnapshot =
            RootSnapshotCache.getOrLoad()
        return withContext(Dispatchers.IO) {
            RootProjection(rootSnapshot.observationId, rootSnapshot.generation, counterTelemetry(rootSnapshot))
        }
    }
}
