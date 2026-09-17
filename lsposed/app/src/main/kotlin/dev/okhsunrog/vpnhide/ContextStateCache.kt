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
    source: ObservationDependency? = null,
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

    /** [reason] is Explicit for a user action; a screen reacting to a network event passes Transition. */
    fun refresh(
        context: Context,
        selfNeedsRestart: Boolean,
        reason: ReadReason = ReadReason.Explicit,
        runBeforeRefresh: Boolean = true,
    ) {
        updateInputs(context, selfNeedsRestart)
        if (runBeforeRefresh) beforeRefresh(requireNotNull(inputs))
        forceRefresh(reason)
    }

    /**
     * Refresh using the inputs a screen already supplied, for a process-side trigger
     * that has no Context of its own (the foreground VPN-state poller). A no-op until
     * startup or a screen has seeded the inputs at least once.
     */
    fun refreshRetained(reason: ReadReason) {
        val current = inputs ?: return
        beforeRefresh(current)
        forceRefresh(reason)
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
