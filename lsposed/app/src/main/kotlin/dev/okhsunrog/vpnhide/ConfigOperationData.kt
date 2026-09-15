package dev.okhsunrog.vpnhide

internal enum class ConfigPhase { Persist, Secret, Native, Ports }

internal enum class PhaseOutcome { NotAttempted, Running, Confirmed, FailedKnown, Unknown }

internal enum class OperationStage { Preparing, Executing, Reconciling, Held }

internal enum class OperationSource { Ui, Bridge, System }

internal data class ConfigOperationSpec(
    val source: OperationSource,
    val writes: Set<ConfigField>,
    val phases: List<ConfigPhase> = listOf(ConfigPhase.Persist, ConfigPhase.Native),
) {
    init {
        require(phases.isNotEmpty())
        require(phases == ConfigPhase.entries.filter { it in phases })
    }
}

internal data class ConfigOperationResult(
    val id: Long,
    val phases: Map<ConfigPhase, PhaseOutcome>,
    val failure: TransitionFailure? = null,
    val conflicts: Set<ConfigField> = emptySet(),
    val draftPending: Boolean = false,
)

internal data class PendingConfigOperation(
    val id: Long,
    val spec: ConfigOperationSpec,
)

internal data class ActiveConfigOperation(
    val request: PendingConfigOperation,
    val stage: OperationStage,
    val ticket: EffectTicket,
    val outcomes: Map<ConfigPhase, PhaseOutcome>,
    val candidate: CanonicalConfig? = null,
    val phase: ConfigPhase? = null,
    val recoveryAttempt: Int = 0,
    val conflicts: Set<ConfigField> = emptySet(),
    val resultDelivered: Boolean = false,
)

/** Construct only after startup has established canonical readability and predecessor quiescence. */
internal data class ConfigOperationState(
    val confirmed: CanonicalConfig,
    val active: ActiveConfigOperation? = null,
    val queue: List<PendingConfigOperation> = emptyList(),
    val drafts: Map<Long, Set<ConfigField>> = emptyMap(),
    val nextId: Long = 1,
    val nextEffect: Long = 1,
)

internal sealed interface ConfigOperationEvent {
    data class Submit(
        val spec: ConfigOperationSpec,
    ) : ConfigOperationEvent

    data class Prepared(
        val ticket: EffectTicket,
        val base: CanonicalConfig,
        val candidate: CanonicalConfig,
    ) : ConfigOperationEvent

    data class PreparationFailed(
        val ticket: EffectTicket,
        val reason: TransitionFailure,
    ) : ConfigOperationEvent

    data class PhaseFinished(
        val ticket: EffectTicket,
        val outcome: PhaseOutcome,
    ) : ConfigOperationEvent

    data class RecoveryFinished(
        val ticket: EffectTicket,
        val evidence: ConfigRecoveryEvidence? = null,
    ) : ConfigOperationEvent

    data class DraftChanged(
        val draftId: Long,
        val fields: Set<ConfigField>,
    ) : ConfigOperationEvent

    data class Cancel(
        val id: Long,
    ) : ConfigOperationEvent

    data object Recheck : ConfigOperationEvent
}

/** Non-null evidence proves all effects quiescent; each attempted phase must be resolved. */
internal data class ConfigRecoveryEvidence(
    val config: CanonicalConfig,
    val outcomes: Map<ConfigPhase, PhaseOutcome>,
) {
    init {
        require(outcomes.values.none { it == PhaseOutcome.Running || it == PhaseOutcome.Unknown })
    }
}

internal sealed interface ConfigOperationEffect {
    data class Accepted(
        val id: Long,
    ) : ConfigOperationEffect

    data class Rejected(
        val reason: TransitionFailure,
    ) : ConfigOperationEffect

    data class Prepare(
        val ticket: EffectTicket,
        val spec: ConfigOperationSpec,
    ) : ConfigOperationEffect

    data class Execute(
        val ticket: EffectTicket,
        val phase: ConfigPhase,
        val config: CanonicalConfig,
    ) : ConfigOperationEffect

    data class Reconcile(
        val ticket: EffectTicket,
    ) : ConfigOperationEffect

    data class Completed(
        val result: ConfigOperationResult,
    ) : ConfigOperationEffect

    data class Recovered(
        val result: ConfigOperationResult,
    ) : ConfigOperationEffect

    data class CancelRejected(
        val id: Long,
    ) : ConfigOperationEffect

    data object RefreshObservations : ConfigOperationEffect
}

internal fun reduceConfigOperation(
    state: ConfigOperationState,
    event: ConfigOperationEvent,
): Transition<ConfigOperationState, ConfigOperationEffect> =
    when (event) {
        is ConfigOperationEvent.Submit -> {
            submitConfigOperation(state, event.spec)
        }

        is ConfigOperationEvent.Prepared -> {
            prepareConfigOperation(state, event)
        }

        is ConfigOperationEvent.PreparationFailed -> {
            if (state.active?.stage == OperationStage.Preparing && state.active.ticket == event.ticket) {
                finishConfigOperation(state, event.reason)
            } else {
                Transition(state)
            }
        }

        is ConfigOperationEvent.PhaseFinished -> {
            completeConfigPhase(state, event)
        }

        is ConfigOperationEvent.RecoveryFinished -> {
            recoverConfigOperation(state, event)
        }

        is ConfigOperationEvent.DraftChanged -> {
            updateOperationDraft(state, event)
        }

        is ConfigOperationEvent.Cancel -> {
            cancelConfigOperation(state, event.id)
        }

        ConfigOperationEvent.Recheck -> {
            if (state.active?.stage == OperationStage.Held) startConfigRecovery(state, manual = true) else Transition(state)
        }
    }

private fun submitConfigOperation(
    state: ConfigOperationState,
    spec: ConfigOperationSpec,
): Transition<ConfigOperationState, ConfigOperationEffect> {
    if (state.active?.stage == OperationStage.Held || state.active?.resultDelivered == true) {
        return Transition(state, listOf(ConfigOperationEffect.Rejected(TransitionFailure.MutationPaused)))
    }
    val request = PendingConfigOperation(state.nextId, spec.copy(writes = configFieldSnapshot(spec.writes), phases = spec.phases.toList()))
    val next = startNextConfigOperation(state.copy(queue = state.queue + request, nextId = state.nextId + 1))
    return next.copy(effects = listOf(ConfigOperationEffect.Accepted(request.id)) + next.effects)
}

internal fun startNextConfigOperation(state: ConfigOperationState): Transition<ConfigOperationState, ConfigOperationEffect> {
    if (state.active != null || state.queue.isEmpty()) return Transition(state)
    val request = state.queue.first()
    val ticket = EffectTicket(request.id, state.nextEffect)
    val active =
        ActiveConfigOperation(
            request,
            OperationStage.Preparing,
            ticket,
            request.spec.phases.associateWith { PhaseOutcome.NotAttempted },
        )
    return Transition(
        state.copy(active = active, queue = state.queue.drop(1), nextEffect = state.nextEffect + 1),
        listOf(ConfigOperationEffect.Prepare(ticket, request.spec)),
    )
}

private fun prepareConfigOperation(
    state: ConfigOperationState,
    event: ConfigOperationEvent.Prepared,
): Transition<ConfigOperationState, ConfigOperationEffect> {
    val active = state.active ?: return Transition(state)
    if (active.stage != OperationStage.Preparing || active.ticket != event.ticket) return Transition(state)
    val conflicts = currentOperationConflicts(state, active.request.spec)
    val next =
        state.copy(
            confirmed = canonicalConfigSnapshot(event.base),
            active = active.copy(candidate = canonicalConfigSnapshot(event.candidate), conflicts = conflicts),
        )
    if (conflicts.isNotEmpty()) return finishConfigOperation(next, TransitionFailure.UiEditConflict)
    return dispatchConfigPhase(
        next,
        active.request.spec.phases
            .first(),
    )
}

internal fun dispatchConfigPhase(
    state: ConfigOperationState,
    phase: ConfigPhase,
): Transition<ConfigOperationState, ConfigOperationEffect> {
    val active = requireNotNull(state.active)
    val ticket = EffectTicket(active.request.id, state.nextEffect)
    val next =
        active.copy(
            stage = OperationStage.Executing,
            ticket = ticket,
            phase = phase,
            outcomes = active.outcomes + (phase to PhaseOutcome.Running),
        )
    return Transition(
        state.copy(active = next, nextEffect = state.nextEffect + 1),
        listOf(ConfigOperationEffect.Execute(ticket, phase, requireNotNull(active.candidate))),
    )
}

private fun completeConfigPhase(
    state: ConfigOperationState,
    event: ConfigOperationEvent.PhaseFinished,
): Transition<ConfigOperationState, ConfigOperationEffect> {
    val active = state.active ?: return Transition(state)
    if (active.stage != OperationStage.Executing || event.ticket != active.ticket) return Transition(state)
    require(event.outcome in setOf(PhaseOutcome.Confirmed, PhaseOutcome.FailedKnown, PhaseOutcome.Unknown))
    val phase = requireNotNull(active.phase)
    val confirmed = if (phase == ConfigPhase.Persist && event.outcome == PhaseOutcome.Confirmed) active.candidate else null
    val next =
        state.copy(
            confirmed = confirmed ?: state.confirmed,
            active =
                active.copy(
                    outcomes =
                        active.outcomes + (phase to event.outcome),
                ),
        )
    return when (event.outcome) {
        PhaseOutcome.Unknown -> {
            startConfigRecovery(next)
        }

        PhaseOutcome.FailedKnown -> {
            finishConfigOperation(next, TransitionFailure.ExecutionFailed)
        }

        else -> {
            val following =
                active.request.spec.phases
                    .dropWhile { it != phase }
                    .drop(1)
                    .firstOrNull()
            if (following == null) finishConfigOperation(next) else dispatchConfigPhase(next, following)
        }
    }
}

private fun currentOperationConflicts(
    state: ConfigOperationState,
    spec: ConfigOperationSpec,
): Set<ConfigField> =
    if (spec.source == OperationSource.Bridge) {
        conflictingFields(
            spec.writes,
            state.drafts.values
                .flatten()
                .toSet(),
        )
    } else {
        emptySet()
    }

private fun updateOperationDraft(
    state: ConfigOperationState,
    event: ConfigOperationEvent.DraftChanged,
): Transition<ConfigOperationState, ConfigOperationEffect> {
    val drafts =
        if (event.fields.isEmpty()) {
            state.drafts - event.draftId
        } else {
            state.drafts +
                (event.draftId to configFieldSnapshot(event.fields))
        }
    val next = state.copy(drafts = drafts)
    val active = state.active ?: return Transition(next)
    if (active.stage == OperationStage.Preparing || active.resultDelivered) return Transition(next)
    return Transition(next.copy(active = active.copy(conflicts = active.conflicts + currentOperationConflicts(next, active.request.spec))))
}

internal fun finishConfigOperation(
    state: ConfigOperationState,
    failure: TransitionFailure? = null,
): Transition<ConfigOperationState, ConfigOperationEffect> {
    val active = requireNotNull(state.active)
    val result = configOperationResult(state, active, failure)
    val completion = if (active.resultDelivered) ConfigOperationEffect.Recovered(result) else ConfigOperationEffect.Completed(result)
    val next = startNextConfigOperation(state.copy(active = null))
    val refresh =
        if (active.outcomes.values.any { it == PhaseOutcome.Confirmed || it == PhaseOutcome.Unknown }) {
            listOf(ConfigOperationEffect.RefreshObservations)
        } else {
            emptyList()
        }
    return next.copy(effects = listOf(completion) + refresh + next.effects)
}

internal fun configOperationResult(
    state: ConfigOperationState,
    active: ActiveConfigOperation,
    failure: TransitionFailure?,
): ConfigOperationResult =
    ConfigOperationResult(
        active.request.id,
        active.outcomes,
        if (active.conflicts.isNotEmpty()) TransitionFailure.UiEditConflict else failure,
        active.conflicts,
        currentOperationConflicts(state, active.request.spec).isNotEmpty(),
    )

private fun cancelConfigOperation(
    state: ConfigOperationState,
    id: Long,
): Transition<ConfigOperationState, ConfigOperationEffect> {
    val queued = state.queue.firstOrNull { it.id == id }
    if (queued != null) {
        val result = ConfigOperationResult(id, queued.spec.phases.associateWith { PhaseOutcome.NotAttempted }, TransitionFailure.Cancelled)
        return Transition(state.copy(queue = state.queue - queued), listOf(ConfigOperationEffect.Completed(result)))
    }
    if (state.active?.request?.id != id) return Transition(state)
    return if (state.active.stage == OperationStage.Preparing) {
        finishConfigOperation(state, TransitionFailure.Cancelled)
    } else {
        Transition(state, listOf(ConfigOperationEffect.CancelRejected(id)))
    }
}
