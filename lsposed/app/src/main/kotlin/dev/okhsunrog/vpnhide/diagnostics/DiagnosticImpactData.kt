package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.ConfigOperationSpec
import dev.okhsunrog.vpnhide.ConfigPhase
import dev.okhsunrog.vpnhide.Transition
import dev.okhsunrog.vpnhide.TransitionFailure

/**
 * Whether a config operation can change what this process measures. Relevant:
 * this app's own roles and hook selection, the global optional features, and
 * whole replacements (import, reset, removal), which are conservatively relevant.
 * Not relevant: other apps' roles, debug logging, auto-hide bookkeeping, and a
 * forced activation with no write at all (the startup runtime reconcile).
 */
internal fun operationAffectsSelfMeasurement(
    spec: ConfigOperationSpec,
    selfPackage: String,
): Boolean =
    spec.removesCanonical ||
        spec.writes.any { field ->
            val segments = field.segments
            when (segments.first()) {
                "apps" -> segments.size == 1 || segments[1] == selfPackage
                "settings" -> segments.size == 1 || segments[1] == "optionalFeatures"
                else -> false
            }
        }

/**
 * Impact of accepted config operations on diagnostic runs: which relevant
 * operations are still unsettled, the known-change epoch advanced at every
 * mutating dispatch of a relevant operation, and the application readiness that
 * feeds diagnostic eligibility. Pure; the facade routes its effects to the run
 * coordinator.
 */
internal data class DiagnosticImpactState(
    val relevant: Set<Long> = emptySet(),
    val dispatched: Set<Long> = emptySet(),
    val changeEpoch: Long = 0,
    /** Relevant operations that settled unresolved and have not been recovered yet (I13). */
    val unresolved: Set<Long> = emptySet(),
    val failed: Boolean = false,
)

internal sealed interface DiagnosticImpactEvent {
    data class Accepted(
        val id: Long,
        val relevant: Boolean,
    ) : DiagnosticImpactEvent

    data class Dispatched(
        val id: Long,
        val phase: ConfigPhase,
    ) : DiagnosticImpactEvent

    data class Settled(
        val id: Long,
        val failure: TransitionFailure?,
    ) : DiagnosticImpactEvent

    /** Manual recovery resolved an operation that had been delivered as unresolved. */
    data class Recovered(
        val id: Long,
        val failure: TransitionFailure?,
    ) : DiagnosticImpactEvent
}

internal sealed interface DiagnosticImpactEffect {
    data class DelayRuns(
        val id: Long,
    ) : DiagnosticImpactEffect

    data object InterruptRuns : DiagnosticImpactEffect

    data class SettleRuns(
        val id: Long,
        val failure: TransitionFailure?,
    ) : DiagnosticImpactEffect
}

internal fun reduceDiagnosticImpact(
    state: DiagnosticImpactState,
    event: DiagnosticImpactEvent,
): Transition<DiagnosticImpactState, DiagnosticImpactEffect> =
    when (event) {
        is DiagnosticImpactEvent.Accepted -> {
            if (event.relevant) {
                Transition(state.copy(relevant = state.relevant + event.id), listOf(DiagnosticImpactEffect.DelayRuns(event.id)))
            } else {
                Transition(state)
            }
        }

        is DiagnosticImpactEvent.Dispatched -> {
            if (event.id in state.relevant && event.id !in state.dispatched) {
                // The first mutating effect of a relevant operation is the known change;
                // later phases of the same operation belong to that change.
                Transition(
                    state.copy(dispatched = state.dispatched + event.id, changeEpoch = state.changeEpoch + 1),
                    listOf(DiagnosticImpactEffect.InterruptRuns),
                )
            } else {
                Transition(state)
            }
        }

        is DiagnosticImpactEvent.Settled -> {
            if (event.id in state.relevant) {
                // An unresolved outcome stays unresolved until its own recovery; a
                // later settlement of another operation (a queued one paused by the
                // same lane, for instance) neither clears nor downgrades it.
                val unknown = event.failure == TransitionFailure.ApplicationUnknown
                Transition(
                    state.copy(
                        relevant = state.relevant - event.id,
                        dispatched = state.dispatched - event.id,
                        unresolved = if (unknown) state.unresolved + event.id else state.unresolved,
                        failed = event.failure != null && !unknown,
                    ),
                    listOf(DiagnosticImpactEffect.SettleRuns(event.id, event.failure)),
                )
            } else {
                Transition(state)
            }
        }

        is DiagnosticImpactEvent.Recovered -> {
            // Only a relevant unresolved operation shapes readiness; recovering an
            // irrelevant one (debug logging, another app) is not a known change.
            if (event.id in state.unresolved) {
                Transition(state.copy(unresolved = state.unresolved - event.id, failed = event.failure != null))
            } else {
                Transition(state)
            }
        }
    }

/** Precedence for eligibility: an unresolved outcome outranks in-flight work, which outranks a known failure. */
internal fun configReadiness(state: DiagnosticImpactState): ConfigReadiness =
    when {
        state.unresolved.isNotEmpty() -> ConfigReadiness.Unknown
        state.relevant.isNotEmpty() -> ConfigReadiness.Applying
        state.failed -> ConfigReadiness.Failed
        else -> ConfigReadiness.Settled
    }
