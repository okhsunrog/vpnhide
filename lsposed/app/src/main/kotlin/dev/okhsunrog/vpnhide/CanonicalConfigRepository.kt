package dev.okhsunrog.vpnhide

import android.content.Context
import dev.okhsunrog.vpnhide.core.CorePaths
import dev.okhsunrog.vpnhide.core.CoreRoot
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
 * Sole app-side coordinator for config edits and runtime activation.
 *
 * The process-owned actor serializes fresh field edits; every config write, native apply and
 * Ports apply is a typed command to the root core ([CoreConfigIo] over the in-process core-ffi client). Core
 * is the only writer of the canonical config, `/proc/vpnhide_ctl`, Zygisk targets and Ports
 * rules. Editor drafts and local preferences stay in Kotlin.
 */
internal object CanonicalConfigRepository {
    internal val processScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    @Volatile private var appContext: Context? = null

    private val coordinator =
        ConfigCoordinator(
            CoreConfigIo(
                prepareCore = { appContext?.let { FfiCoreConfigClient(CorePaths.socket(it)) } },
            ),
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
        CoreRoot.attach(context)
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
