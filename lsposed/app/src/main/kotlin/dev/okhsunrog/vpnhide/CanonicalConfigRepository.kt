package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.RoutingGateCache
import dev.okhsunrog.vpnhide.picker.TargetsCache
import dev.okhsunrog.vpnhide.statistics.StatisticsCache
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/** Runtime channels that must be re-derived after the canonical config changes. */
internal data class CanonicalActivation(
    val native: Boolean = true,
    val ports: Boolean = false,
)

internal data class CanonicalWriteResult(
    val exitCode: Int,
    val output: String,
) {
    val succeeded: Boolean
        get() = exitCode == 0
}

/**
 * Build the one root transaction used for canonical-config persistence.
 *
 * Every step is joined with `&&`: activators must never run against stale
 * state when the atomic write (or a coupled secret write) failed.
 */
internal fun buildCanonicalPersistenceCommand(
    config: CanonicalConfig,
    coupledCommands: List<String> = emptyList(),
    activation: CanonicalActivation = CanonicalActivation(),
): String =
    buildList {
        add(buildCanonicalConfigWriteCommand(config))
        addAll(coupledCommands)
        if (activation.native) add(ConfigChannels.nativeActivatorCommand())
        if (activation.ports) add(ConfigChannels.portsActivatorCommand())
    }.joinToString(" && ")

/**
 * Sole app-side coordinator for canonical JSON writes and runtime activation.
 *
 * The monitor prevents two background UI operations from interleaving root
 * writes in this process. The filesystem write itself remains atomic for
 * system_server and native readers.
 */
internal object CanonicalConfigRepository {
    private val writeMutex = Mutex()

    suspend fun commit(
        config: CanonicalConfig,
        coupledCommands: List<String> = emptyList(),
        activation: CanonicalActivation = CanonicalActivation(),
        timeoutSec: Long = SU_DEFAULT_TIMEOUT_SEC,
    ): CanonicalWriteResult =
        writeMutex.withLock {
            val command = buildCanonicalPersistenceCommand(config, coupledCommands, activation)
            val (exit, output) = withContext(Dispatchers.IO) { suExec(command, timeoutSec) }
            if (exit == 0) refreshDerivedCaches()
            CanonicalWriteResult(exit, output)
        }

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
