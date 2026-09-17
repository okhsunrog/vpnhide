package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.DiagnosticsCache
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.mapNotNull
import kotlinx.coroutines.launch
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
 *
 * The protection tiles are folded from the diagnostic presentation at derivation
 * time, while the hero renders the live Situation. So that the two never come
 * from different instants, the cache follows the suite: a terminal attempt newer
 * than the one the tiles were derived from re-derives them in the background,
 * without requesting another run.
 */
internal object DashboardCache : ContextStateCache<RootProjection<DashboardState>>(
    traceName = "dashboard_state",
    logTag = LogTags.DASHBOARD,
    source = RootSnapshotCache.dependency,
    timeoutMillis = 120_000,
) {
    val state: StateFlow<DashboardState?> = ProjectedStateFlow(value) { it?.value }

    /**
     * A derivation the user asked for is in flight. A background re-derivation (the
     * startup reconcile, a root dependency after a config phase) is not; the top-bar
     * indicator follows this, [loading] follows every derivation.
     */
    val refreshing: StateFlow<Boolean> by lazy { ProjectedStateFlow(observation) { it.active?.reason == ReadReason.Explicit } }

    override fun beforeRefresh(inputs: ContextObservationInputs) {
        DiagnosticsCache.retry(inputs.context, inputs.selfNeedsRestart)
    }

    override suspend fun load(
        @Suppress("UNUSED_PARAMETER") request: ObservationRequest,
    ): RootProjection<DashboardState> {
        follower
        val (context, selfNeedsRestart) = requireNotNull(inputs)
        // Join or read the terminal attempt, then render the shared presentation as
        // the screens do; a blocked or failed attempt is observed, never retried here.
        val diagnostics = DiagnosticsCache.awaitTerminal(context, selfNeedsRestart)
        val rootSnapshot =
            RootSnapshotCache.getOrLoad()
        return withContext(Dispatchers.IO) {
            RootProjection(
                rootSnapshot.observationId,
                rootSnapshot.generation,
                loadDashboardState(context, selfNeedsRestart, rootSnapshot, diagnostics),
            )
        }
    }

    private val follower by lazy { ObservationRuntime.scope.launch { followDiagnostics() } }

    /**
     * Re-derive after a terminal attempt the tiles have not seen, from the root
     * snapshot already in hand (no root re-read, no run request). A derivation in
     * flight is left to render it first; a failed derivation is not retried from
     * here (that stays with the user's Retry), and a derivation that already
     * rendered the attempt is left alone.
     */
    private suspend fun followDiagnostics() {
        DiagnosticsCache.presentation
            .mapNotNull { it.lastAttempt?.id }
            .distinctUntilChanged()
            .collect { attempt ->
                observation.first { it.active == null }
                val rendered = state.value?.diagnosticsAttemptId ?: return@collect
                if (rendered < attempt) refreshInPlace(force = false, reason = ReadReason.Background)
            }
    }
}
