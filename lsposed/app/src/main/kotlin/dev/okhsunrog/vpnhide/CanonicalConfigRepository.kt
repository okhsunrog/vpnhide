package dev.okhsunrog.vpnhide

import android.content.Context
import dev.okhsunrog.vpnhide.diagnostics.RoutingGateCache
import dev.okhsunrog.vpnhide.picker.TargetsCache
import dev.okhsunrog.vpnhide.statistics.StatisticsCache
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob

/** Runtime channels that must be re-derived after the canonical config changes. */
internal data class CanonicalActivation(
    val native: Boolean = true,
    val ports: Boolean = false,
)

internal data class CanonicalWriteResult(
    val exitCode: Int,
    val output: String,
    val operation: ConfigOperationResult? = null,
) {
    val succeeded: Boolean
        get() = exitCode == 0
}

/**
 * Sole app-side coordinator for canonical JSON writes and runtime activation.
 *
 * The process-owned actor serializes fresh field edits through root receipt recovery.
 * The filesystem write remains atomic for system_server and native readers.
 */
internal object CanonicalConfigRepository {
    internal val processScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val runner = RootProcessRunner()

    @Volatile private var appContext: Context? = null
    private val coordinator =
        ConfigCoordinator(
            ConfigRootIo { appContext?.let { prepareRootMutationTransport(it, runner) } },
            processScope,
            confirmed = { VpnHideLog.enabled = it.debug },
            refresh = { refreshDerivedCaches() },
            manageLogging = true,
        )
    val state = coordinator.view

    suspend fun initialize(context: Context): ConfigCoordinatorMode {
        appContext = context.applicationContext
        return coordinator.initialize()
    }

    fun retry() = coordinator.retry()

    fun draftChanged(
        id: Long,
        fields: Set<ConfigField>,
    ) = coordinator.draftChanged(id, fields)

    suspend fun commit(mutation: CanonicalMutation): CanonicalWriteResult = coordinator.submit(mutation)

    suspend fun reconcile(ports: Boolean = false): CanonicalWriteResult =
        commit(
            CanonicalMutation(
                emptyList(),
                source = OperationSource.System,
                activation = CanonicalActivation(ports = ports),
                forceActivation = true,
            ),
        )

    /**
     * The caches whose value is *derived from the canonical config*, and which a
     * write therefore leaves stale. That is the whole membership rule — add a cache
     * here if and only if its `load` reads the config (directly, or via the root
     * snapshot's config-bearing sections).
     *
     * Deliberately NOT members: `AppListCache` (an `AppSummary` carries no config
     * state — the picker merges target flags in reactively), `UpdateCheckCache` and
     * `DiagnosticsCache` (unrelated to the config), and `SystemServerConfigCache`
     * (lives in the system_server process, unreachable from here — its own
     * `SystemDataFileWatcher` invalidates it).
     *
     * `RootSnapshotCache` is the shared upstream rather than a member; see
     * [refreshDerivedCaches].
     */
    private val derivedCaches: List<StateCache<*>> =
        listOf(TargetsCache, DashboardCache, StatisticsCache, RoutingGateCache)

    /**
     * Reload every config-derived cache in place — swap old→new, so no observer sees
     * a null blank between the write and the reload (the toggle-flicker fix).
     *
     * The root snapshot goes first and alone: the others all derive from it, so they
     * follow with `force = false` to reuse it. Passing `force = true` here would be a
     * silent, invisible cost — each cache would invalidate the snapshot and re-run the
     * whole root shell for itself, once per member. Iterating a list instead of
     * open-coding the calls is what keeps the order and the flag structural rather
     * than a comment someone has to notice.
     *
     * A failure keeps the stale value (the cache records its own error), so one
     * unhappy cache can't abort the rest.
     */
    internal suspend fun refreshDerivedCaches() {
        runCatching { RootSnapshotCache.refresh() }
        derivedCaches.forEach { cache ->
            if (!cache.pristine) runCatching { cache.refreshInPlace(force = false) }
        }
    }
}
