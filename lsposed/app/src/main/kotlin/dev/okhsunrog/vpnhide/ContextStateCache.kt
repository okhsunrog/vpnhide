package dev.okhsunrog.vpnhide

import android.content.Context

internal data class ContextObservationInputs(
    val context: Context,
    val selfNeedsRestart: Boolean,
)

/** Snapshot application-only inputs before a load; changed readiness invalidates older work. */
internal abstract class ContextStateCache<T>(
    traceName: String,
    logTag: String,
    source: ObservationDependency,
    timeoutMillis: Long = 60_000,
) : StateCache<T>(traceName, logTag, source, timeoutMillis) {
    @Volatile protected var inputs: ContextObservationInputs? = null
        private set
    override val ready: Boolean get() = inputs != null

    fun ensureLoaded(
        context: Context,
        selfNeedsRestart: Boolean,
    ) {
        updateInputs(context, selfNeedsRestart)
        ensure()
    }

    fun refresh(
        context: Context,
        selfNeedsRestart: Boolean,
    ) {
        updateInputs(context, selfNeedsRestart)
        beforeRefresh(requireNotNull(inputs))
        forceRefresh()
    }

    /** Explicit refresh only; dependency invalidation must not trigger other effect owners. */
    protected open fun beforeRefresh(inputs: ContextObservationInputs) = Unit

    private fun updateInputs(
        context: Context,
        selfNeedsRestart: Boolean,
    ) {
        val next = ContextObservationInputs(context.applicationContext, selfNeedsRestart)
        val changed = inputs != null && inputs != next
        inputs = next
        if (changed) invalidate()
    }
}
