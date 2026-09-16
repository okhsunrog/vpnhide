package dev.okhsunrog.vpnhide

import android.content.Context
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticImpactObserver
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
            invalidateObservations = RootSnapshotCache::invalidate,
            observer = DiagnosticImpactObserver { requireNotNull(appContext).packageName },
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

    /** Root invalidation synchronously marks registered dependents obsolete; loading is independent of writes. */
    internal suspend fun refreshDerivedCaches() {
        RootSnapshotCache.getOrLoad()
    }
}
