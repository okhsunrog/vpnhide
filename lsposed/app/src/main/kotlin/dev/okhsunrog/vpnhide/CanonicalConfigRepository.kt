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

    // Reload each derived cache in place — swap old→new, so no observer sees a null blank
    // between the write and the reload (the toggle-flicker fix). Root first: the others
    // derive from it and reuse it via force=false. Failures keep the stale value.
    internal suspend fun refreshDerivedCaches() {
        runCatching { RootSnapshotCache.refresh() }
        runCatching { TargetsCache.refreshInPlace(force = false) }
        runCatching { DashboardCache.refreshInPlace(force = false) }
        runCatching { StatisticsCache.refreshInPlace(force = false) }
        // Tolerates RoutingGateCache not being initialized yet (load() throws on a
        // null appContext before any screen has called ensureLoaded/refresh) — a
        // config write racing app startup should not crash the write itself.
        runCatching { RoutingGateCache.refreshInPlace(force = false) }
    }
}
