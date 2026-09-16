package dev.okhsunrog.vpnhide

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

internal class ObservationReadException(
    val reason: TransitionFailure,
) : RuntimeException(reason.name)

/**
 * The monotonic millisecond base every observation timestamp is expressed in:
 * [ObservationRequest.startedAt], [StaleMark.since] and anything derived from
 * them (how long a re-read has been owed). It is not a wall clock and must never
 * be formatted as a date — `System.currentTimeMillis()` stays the base for a
 * measurement's `observedAt` and the bundle's dates.
 */
internal object ObservationClock {
    fun now(): Long = System.nanoTime() / 1_000_000
}

/** Process-owned reads. A waiter owns neither the worker nor its cancellation. */
internal class ObservationCoordinator<T>(
    private val scope: CoroutineScope,
    private val load: suspend (ObservationRequest) -> T,
    private val ready: () -> Boolean = { true },
    private val timeoutMillis: Long = 60_000,
    private val clock: () -> Long = ObservationClock::now,
    private val deadline: suspend () -> Unit = { delay(timeoutMillis) },
    private val changed: (ObservationState<T>, ObservationState<T>) -> Unit = { _, _ -> },
) {
    private val lock = Any()
    private val mutableState = MutableStateFlow(ObservationState<T>())
    val state = mutableState.asStateFlow()
    private val waiters = mutableSetOf<CompletableDeferred<T>>()
    private val timers = mutableMapOf<Long, Job>()
    private var retryAfterDrain = false

    fun ensure() =
        synchronized(lock) {
            if (ready()) dispatch(ObservationEvent.Ensure(clock()))
        }

    fun invalidate(
        start: Boolean = true,
        reason: ReadReason = ReadReason.Background,
    ) = synchronized(lock) {
        dispatch(ObservationEvent.Invalidate(clock(), start = start && ready() && state.value.attempted, reason = reason))
    }

    fun refresh(reason: ReadReason = ReadReason.Explicit) =
        synchronized(lock) {
            if (state.value.quarantined) retryAfterDrain = true
            if (ready()) dispatch(ObservationEvent.Refresh(clock(), reason = reason))
        }

    /** Follows superseded requests to the requested generation; never returns lastGood as fresh. */
    suspend fun read(
        refresh: Boolean = false,
        notBefore: Long = Long.MIN_VALUE,
        reason: ReadReason = ReadReason.Explicit,
    ): T {
        val waiter = CompletableDeferred<T>()
        synchronized(lock) { admit(waiter, refresh, notBefore, reason) }
        return try {
            withTimeoutOrNull(timeoutMillis * 2) { waiter.await() }
                ?: throw ObservationReadException(TransitionFailure.DeadlineExceeded)
        } finally {
            synchronized(lock) { waiters.remove(waiter) }
        }
    }

    private fun admit(
        waiter: CompletableDeferred<T>,
        refresh: Boolean,
        notBefore: Long,
        reason: ReadReason,
    ) {
        if (!ready()) {
            waiter.completeExceptionally(ObservationReadException(TransitionFailure.InitializationPending))
            return
        }
        if (state.value.quarantined) {
            retryAfterDrain = retryAfterDrain || refresh
            waiter.completeExceptionally(ObservationReadException(TransitionFailure.ResourceUnavailable))
            return
        }
        if (refresh) dispatch(ObservationEvent.Refresh(clock(), notBefore, reason)) else dispatch(ObservationEvent.Ensure(clock()))
        if (state.value.active == null) {
            complete(waiter)
        } else {
            waiters.add(waiter)
        }
    }

    private fun dispatch(event: ObservationEvent<T>) {
        val previous = state.value
        val transition = reduceObservation(previous, event)
        mutableState.value = transition.state
        // Only short synchronous invalidation callbacks run here, never reads or waits.
        runCatching { changed(previous, transition.state) }
        transition.effects.forEach { effect ->
            when (effect) {
                is ObservationEffect.Load -> {
                    start(effect.request)
                }

                is ObservationEffect.Finished -> {
                    timers.remove(effect.id)?.cancel()
                    if (effect.outcome != ObservationCompletion.Superseded) {
                        waiters.toList().forEach(::complete)
                        waiters.clear()
                    }
                }

                else -> {}
            }
        }
    }

    private fun complete(waiter: CompletableDeferred<T>) {
        val view = state.value
        val failure = view.error
        val value = view.lastGood?.takeIf { it.request.generation == view.generation }?.value
        if (failure == null && value != null) {
            waiter.complete(value)
        } else {
            waiter.completeExceptionally(ObservationReadException(failure ?: TransitionFailure.ReadFailed))
        }
    }

    private fun start(request: ObservationRequest) {
        timers[request.id] =
            scope.launch {
                deadline()
                synchronized(lock) {
                    if (state.value.active?.id == request.id) {
                        dispatch(ObservationEvent.Failed(request.id, TransitionFailure.DeadlineExceeded, clock(), quiescent = false))
                    }
                }
            }
        scope.launch {
            val result = runCatching { load(request) }
            synchronized(lock) { finish(request, result) }
        }
    }

    private fun finish(
        request: ObservationRequest,
        result: Result<T>,
    ) {
        if (state.value.quarantineRequestId == request.id) {
            val retry = retryAfterDrain || state.value.generation != request.generation
            retryAfterDrain = false
            dispatch(ObservationEvent.ResourceRecovered(request.id, clock(), retry = retry))
        } else {
            val value = result.getOrNull()
            if (value != null) {
                dispatch(ObservationEvent.Loaded(request.id, value, clock()))
            } else {
                dispatch(ObservationEvent.Failed(request.id, TransitionFailure.ReadFailed, clock()))
            }
        }
    }
}
