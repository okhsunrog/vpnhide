package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.DiagnosticsCache
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.withContext

/**
 * App-scoped cache for the Dashboard's root-derived facts, and the screen state
 * projected from them.
 *
 * The cached value is [DashboardRootFacts]: everything that costs a root shell
 * or parsing and changes only with the root snapshot. It is refreshed explicitly
 * (pull-to-refresh, Retry, a Save) and by the root dependency. [state] is not
 * cached at all: it is a projection of those facts and the live diagnostic
 * presentation, assembled by [assembleDashboardState] on every change of either,
 * so the protection tiles, the banners and the hero always describe the same
 * instant and nothing has to follow the suite to keep them in step.
 *
 * Refreshing this cache re-reads root and nothing else: it never requests a
 * diagnostic run. The explicit run has one entry point,
 * `retryDiagnosticsAndDashboard`, and the automatic one is owed by the
 * presentation (`owedConfirmation`).
 */
internal object DashboardCache : ContextStateCache<RootProjection<DashboardRootFacts>>(
    traceName = "dashboard_state",
    logTag = LogTags.DASHBOARD,
    source = RootSnapshotCache.dependency,
    timeoutMillis = 120_000,
) {
    /**
     * The screen state, null until the root facts exist and the suite has a first
     * terminal attempt: the Dashboard appears with its first verdict (blocked, failed
     * or measured), as it always has, rather than with tiles that say nothing yet.
     * The previous value stays visible while a root refresh is in flight.
     */
    val state: StateFlow<DashboardState?> by lazy {
        combine(value, DiagnosticsCache.presentation) { facts, presentation ->
            val context = inputs?.context
            if (facts == null || context == null || presentation.lastAttempt == null) return@combine null
            assembleDashboardState(context, facts.value, presentation)
        }.stateIn(ObservationRuntime.scope, SharingStarted.Eagerly, null)
    }

    /**
     * A derivation the user asked for is in flight. A background re-derivation (the
     * startup reconcile, a root dependency after a config phase) is not; the top-bar
     * indicator follows this, [loading] follows every derivation.
     */
    val refreshing: StateFlow<Boolean> by lazy { ProjectedStateFlow(observation) { it.active?.reason == ReadReason.Explicit } }

    override suspend fun load(
        @Suppress("UNUSED_PARAMETER") request: ObservationRequest,
    ): RootProjection<DashboardRootFacts> {
        val (context, selfNeedsRestart) = requireNotNull(inputs)
        val rootSnapshot = RootSnapshotCache.getOrLoad()
        return withContext(Dispatchers.IO) {
            RootProjection(
                rootSnapshot.observationId,
                rootSnapshot.generation,
                deriveDashboardRootFacts(context, selfNeedsRestart, rootSnapshot),
            )
        }
    }
}
