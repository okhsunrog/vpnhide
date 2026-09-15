package dev.okhsunrog.vpnhide

internal data class ObservationRequest(
    val id: Long,
    val generation: Long,
    val startedAt: Long,
)

internal data class ObservedValue<T>(
    val value: T,
    val request: ObservationRequest,
    val finishedAt: Long,
)

internal enum class ObservationCompletion { Published, Failed, Superseded }

/** Load state for the existing StateCache facade; no jobs or global store live here. */
internal data class ObservationState<T>(
    val lastGood: ObservedValue<T>? = null,
    val active: ObservationRequest? = null,
    val generation: Long = 0,
    val nextId: Long = 1,
    val error: TransitionFailure? = null,
    val attempted: Boolean = false,
    val quarantined: Boolean = false,
    val quarantineRequestId: Long? = null,
)

internal sealed interface ObservationEvent<out T> {
    data class Ensure(
        val now: Long,
    ) : ObservationEvent<Nothing>

    data class Refresh(
        val now: Long,
        val notBefore: Long = Long.MIN_VALUE,
    ) : ObservationEvent<Nothing>

    data class Invalidate(
        val now: Long,
    ) : ObservationEvent<Nothing>

    data class Loaded<T>(
        val id: Long,
        val value: T,
        val now: Long,
    ) : ObservationEvent<T>

    data class Failed(
        val id: Long,
        val reason: TransitionFailure,
        val now: Long,
        val quiescent: Boolean = true,
    ) : ObservationEvent<Nothing>

    data class ResourceRecovered(
        val id: Long,
        val now: Long,
    ) : ObservationEvent<Nothing>
}

internal sealed interface ObservationEffect {
    data class Load(
        val request: ObservationRequest,
    ) : ObservationEffect

    data class Join(
        val id: Long,
    ) : ObservationEffect

    data class AwaitGeneration(
        val generation: Long,
    ) : ObservationEffect

    data class Finished(
        val id: Long,
        val outcome: ObservationCompletion,
    ) : ObservationEffect

    data class Unavailable(
        val reason: TransitionFailure,
    ) : ObservationEffect
}

internal fun <T> reduceObservation(
    state: ObservationState<T>,
    event: ObservationEvent<T>,
): Transition<ObservationState<T>, ObservationEffect> =
    when (event) {
        is ObservationEvent.Ensure -> {
            if (state.attempted) Transition(state) else startObservation(state, event.now)
        }

        is ObservationEvent.Refresh -> {
            refreshObservation(state, event)
        }

        is ObservationEvent.Invalidate -> {
            invalidateObservation(state, event.now)
        }

        is ObservationEvent.Loaded -> {
            finishObservation(state, event.id, event.now, event.value, null, true)
        }

        is ObservationEvent.Failed -> {
            finishObservation(state, event.id, event.now, null, event.reason, event.quiescent)
        }

        is ObservationEvent.ResourceRecovered -> {
            if (state.quarantined && state.quarantineRequestId == event.id) {
                startObservation(state.copy(quarantined = false, quarantineRequestId = null), event.now)
            } else {
                Transition(state)
            }
        }
    }

private fun <T> startObservation(
    state: ObservationState<T>,
    now: Long,
): Transition<ObservationState<T>, ObservationEffect> {
    if (state.quarantined) return Transition(state, listOf(ObservationEffect.Unavailable(TransitionFailure.ResourceUnavailable)))
    val request = ObservationRequest(state.nextId, state.generation, now)
    return Transition(
        state.copy(active = request, nextId = state.nextId + 1, attempted = true),
        listOf(ObservationEffect.Load(request)),
    )
}

private fun <T> refreshObservation(
    state: ObservationState<T>,
    event: ObservationEvent.Refresh,
): Transition<ObservationState<T>, ObservationEffect> {
    val active = state.active ?: return startObservation(state, event.now)
    if (active.generation == state.generation && active.startedAt >= event.notBefore) {
        return Transition(state, listOf(ObservationEffect.Join(active.id)))
    }
    // A successor is already requested: do not produce unbounded generations for equivalent requests.
    val next = if (active.generation != state.generation) state else state.copy(generation = state.generation + 1)
    return Transition(next, listOf(ObservationEffect.AwaitGeneration(next.generation)))
}

private fun <T> invalidateObservation(
    state: ObservationState<T>,
    now: Long,
): Transition<ObservationState<T>, ObservationEffect> {
    val next = state.copy(generation = state.generation + 1)
    return if (state.active == null) startObservation(next, now) else Transition(next)
}

private fun <T> finishObservation(
    state: ObservationState<T>,
    id: Long,
    now: Long,
    value: T?,
    error: TransitionFailure?,
    quiescent: Boolean,
): Transition<ObservationState<T>, ObservationEffect> {
    val active = state.active ?: return Transition(state)
    if (active.id != id) return Transition(state)
    if (!quiescent) {
        return Transition(
            state.copy(active = null, quarantined = true, quarantineRequestId = id, error = TransitionFailure.ResourceUnavailable),
            listOf(ObservationEffect.Finished(id, ObservationCompletion.Failed)),
        )
    }
    if (active.generation != state.generation) {
        val next = startObservation(state.copy(active = null), now)
        return next.copy(effects = listOf(ObservationEffect.Finished(id, ObservationCompletion.Superseded)) + next.effects)
    }
    val observed: ObservedValue<T>? = if (error == null) ObservedValue(requireNotNull(value), active, now) else state.lastGood
    return Transition(
        state.copy(active = null, lastGood = observed, error = error),
        listOf(ObservationEffect.Finished(id, if (error == null) ObservationCompletion.Published else ObservationCompletion.Failed)),
    )
}
