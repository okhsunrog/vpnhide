package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.startup.StartupTrace
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.StateFlow

internal interface ObservationDependency {
    fun subscribe(invalidate: () -> Unit)

    fun refresh()
}

internal object ObservationRuntime {
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
}

/** One process-owned observation; compatibility flows are projections, never independent stores. */
internal abstract class StateCache<T>(
    private val traceName: String,
    private val logTag: String,
    private val source: ObservationDependency? = null,
    timeoutMillis: Long = 60_000,
) {
    private val coordinator by lazy {
        ObservationCoordinator(
            scope = ObservationRuntime.scope,
            timeoutMillis = timeoutMillis,
            ready = { ready },
            load = { request ->
                StartupTrace.mark("${traceName}_start")
                try {
                    load(request).also { StartupTrace.mark("${traceName}_done") }
                } catch (error: Exception) {
                    StartupTrace.mark("${traceName}_failed")
                    VpnHideLog.w(logTag, "$traceName read failed: ${error.javaClass.simpleName}")
                    throw error
                }
            },
            changed = ::observationChanged,
        ).also { owner -> source?.subscribe { owner.invalidate() } }
    }

    val observation: StateFlow<ObservationState<T>> get() = coordinator.state
    val value: StateFlow<T?> by lazy { ProjectedStateFlow(observation) { it.lastGood?.value } }
    val loading: StateFlow<Boolean> by lazy { ProjectedStateFlow(observation) { it.active != null } }
    val error: StateFlow<String?> by lazy { ProjectedStateFlow(observation) { if (it.active == null) it.error?.name else null } }
    val current: StateFlow<T?> by lazy { ProjectedStateFlow(observation, ::currentObservationValue) }
    val pristine: Boolean get() = !observation.value.attempted
    protected open val ready: Boolean get() = true

    protected abstract suspend fun load(request: ObservationRequest): T

    protected open fun observationChanged(
        previous: ObservationState<T>,
        next: ObservationState<T>,
    ) = Unit

    // Loads run on the process-owned ObservationRuntime scope; leaving a screen only detaches its collectors.
    protected fun ensure() = coordinator.ensure()

    protected fun forceRefresh() {
        if (!ready) return
        source?.refresh()
        coordinator.refresh()
    }

    suspend fun refreshInPlace(force: Boolean = true) {
        if (!ready) return
        if (force) source?.refresh()
        try {
            coordinator.read(refresh = true)
        } catch (error: CancellationException) {
            throw error
        } catch (_: ObservationReadException) {
            // Failure is published in observation; callers of the legacy Unit facade inspect that state.
        }
    }

    protected suspend fun awaitValue(
        refresh: Boolean = false,
        notBefore: Long = Long.MIN_VALUE,
    ): T = coordinator.read(refresh, notBefore)

    protected fun requestRefresh() = coordinator.refresh()

    fun markStale() = coordinator.invalidate(start = false)

    open fun invalidate() = coordinator.invalidate()
}
