package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.DiagnosticsCache
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
internal object DashboardCache : ContextStateCache<RootProjection<DashboardState>>(
    traceName = "dashboard_state",
    logTag = LogTags.DASHBOARD,
    source = RootSnapshotCache.dependency,
    timeoutMillis = 120_000,
) {
    val state: StateFlow<DashboardState?> = ProjectedStateFlow(value) { it?.value }

    override fun beforeRefresh(inputs: ContextObservationInputs) {
        DiagnosticsCache.retry(ObservationRuntime.scope, inputs.context, inputs.selfNeedsRestart)
    }

    override suspend fun load(
        @Suppress("UNUSED_PARAMETER") request: ObservationRequest,
    ): RootProjection<DashboardState> {
        val (context, selfNeedsRestart) = requireNotNull(inputs)
        // Join or read the terminal attempt; a Blocked/Failed one is observed, never retried here.
        val diagnosticObservation = DiagnosticsCache.awaitTerminal(context, selfNeedsRestart)
        val rootSnapshot =
            RootSnapshotCache.getOrLoad()
        return withContext(Dispatchers.IO) {
            RootProjection(
                rootSnapshot.observationId,
                rootSnapshot.generation,
                loadDashboardState(context, selfNeedsRestart, rootSnapshot, diagnosticObservation),
            )
        }
    }
}
