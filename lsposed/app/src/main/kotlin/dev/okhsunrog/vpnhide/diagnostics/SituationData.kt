package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.ReadReason
import dev.okhsunrog.vpnhide.TransitionFailure
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.transformLatest

/** What a [Situation.Checking] is waiting for — the wording differs, the state does not. */
internal enum class CheckingWhat { VpnState, Suite, ConfigApplying }

/** Nothing can be measured until the user does this. */
internal enum class ActionNeededKind { RestartApp, RestartDevice, ApplicationFailed, ApplicationUnknown }

/** Why no current answer exists, kept distinct from "the VPN is off" and from "a leak was found" (I13). */
internal sealed interface CouldNotCheckCause {
    /** The routing read failed or its source is quarantined. */
    data class RoutingUnknown(
        val cause: TransitionFailure?,
    ) : CouldNotCheckCause

    /** The latest run could not execute; older results may still be listed as history. */
    data class RunFailed(
        val failure: TransitionFailure?,
    ) : CouldNotCheckCause

    /** The latest run was cut short and there is no earlier measurement to fall back on. */
    data class Interrupted(
        val failure: TransitionFailure?,
    ) : CouldNotCheckCause

    /** A probe helper of an earlier run never returned, so no new run can start at all. */
    data object ProbeUnavailable : CouldNotCheckCause
}

/** How well a presented measurement still describes the current conditions. */
internal sealed interface Staleness {
    data object Current : Staleness

    data object Changed : Staleness

    /** A re-read or an automatic re-run is confirming the measurement; [since] null when no grace is tracked (an automatic run has its own deadline). */
    data class Confirming(
        val reason: ReadReason,
        val since: Long?,
    ) : Staleness
}

/**
 * What the user is looking at, decided once for every surface.
 *
 * This is the single classification behind the Dashboard hero and the Diagnostics
 * banner: they map it to words and colour, they never re-derive precedence, so the
 * two cannot drift apart the way `heroDecision` and `diagnosticScreenDecision` did.
 * It is built purely from one [DiagnosticPresentation] plus `now` on the
 * [dev.okhsunrog.vpnhide.ObservationClock] base; per-surface side channels (the
 * results list, the attempt notice, the tiles, the dashboard issue counts) stay
 * separate inputs to those maps rather than fields here.
 */
internal sealed interface Situation {
    /** Inputs not supplied yet; nothing has been observed, not even a failure. */
    data object Initializing : Situation

    /** A read or a run is in flight and no claim is being made; [lastKnown] is what the process last knew. */
    data class Checking(
        val what: CheckingWhat,
        val lastKnown: SelfRouting?,
        val reason: ReadReason,
        val since: Long,
    ) : Situation

    data object VpnOff : Situation

    /** This app is excluded from the tunnel, so there is nothing to hide — expected, not a failure. */
    data object NotMeasurable : Situation

    data class ActionNeeded(
        val kind: ActionNeededKind,
    ) : Situation

    data class CouldNotCheck(
        val cause: CouldNotCheckCause,
    ) : Situation

    data class Measured(
        val evidence: EvidenceConclusion,
        val staleness: Staleness,
    ) : Situation
}

/**
 * How long a re-read may run before it is worth saying so.
 *
 * A user-visible cause is visible immediately: the user asked (Explicit), or an
 * external signal said the fact may have changed (Transition), and claiming the
 * old fact through that would be a lie for as long as the grace lasts. Our own
 * housekeeping (Background: the startup reconcile, a root dependency after a
 * config phase, the foreground-return safety net) is silent for a bounded window
 * instead, so a routine top-up does not flicker the hero. 2 s covers the
 * reconcile's root reload with package inventory (~1.2 s) on a loaded device
 * without hiding a genuinely slow read for long.
 */
internal object SituationGrace {
    private const val BACKGROUND_MILLIS = 2_000L

    fun of(reason: ReadReason): Long =
        when (reason) {
            ReadReason.Background -> BACKGROUND_MILLIS
            ReadReason.Transition, ReadReason.Explicit -> 0L
        }
}

/**
 * Classify the presentation, top to bottom, first match wins: process health
 * (initialization, a quarantined probe) → an action the user owes → the config
 * change being applied → what is known about routing → a run in flight → the
 * latest attempt → the measurement and how stale it is.
 *
 * [now] is on the [dev.okhsunrog.vpnhide.ObservationClock] base, the same base
 * [RoutingKnowledge.Verifying.since] is measured on. [grace] is injected so the
 * scenario tests can pin the boundary without a real clock.
 */
internal fun situation(
    presentation: DiagnosticPresentation,
    now: Long,
    grace: (ReadReason) -> Long = SituationGrace::of,
): Situation {
    conditionSituation(presentation, now)?.let { return it }
    return when (val step = routingStep(presentation.routing, now, grace)) {
        is RoutingStep.Decided -> step.situation
        is RoutingStep.Routed -> measuredSituation(presentation, now, step.fact, step.confirming)
    }
}

/** The last known fact, whether it is current ([RoutingKnowledge.Known]) or history. */
private fun routingFact(routing: RoutingKnowledge): SelfRouting? =
    when (routing) {
        is RoutingKnowledge.Known -> routing.routing
        is RoutingKnowledge.Verifying -> routing.lastKnown
        is RoutingKnowledge.Unknown -> routing.lastKnown
    }

/**
 * Steps 1–4: conditions of this process and of the configuration, which outrank
 * every routing fact because none of them can be measured around. A configuration
 * change being applied is always somebody's action, so it is visible immediately
 * (Explicit) rather than after a grace.
 */
private fun conditionSituation(
    presentation: DiagnosticPresentation,
    now: Long,
): Situation? {
    if (presentation.eligibility == DiagnosticEligibility.Initializing) return Situation.Initializing
    // A quarantined probe cannot measure whatever the conditions are, so it is named
    // before any condition that would otherwise promise a re-check.
    if (presentation.probeUnavailable) return Situation.CouldNotCheck(CouldNotCheckCause.ProbeUnavailable)
    return when (presentation.eligibility) {
        DiagnosticEligibility.RestartApp -> {
            Situation.ActionNeeded(ActionNeededKind.RestartApp)
        }

        DiagnosticEligibility.RestartDevice -> {
            Situation.ActionNeeded(ActionNeededKind.RestartDevice)
        }

        DiagnosticEligibility.ApplicationUnknown -> {
            Situation.ActionNeeded(ActionNeededKind.ApplicationUnknown)
        }

        DiagnosticEligibility.ApplicationFailed -> {
            Situation.ActionNeeded(ActionNeededKind.ApplicationFailed)
        }

        DiagnosticEligibility.Applying -> {
            Situation.Checking(CheckingWhat.ConfigApplying, routingFact(presentation.routing), ReadReason.Explicit, now)
        }

        else -> {
            null
        }
    }
}

/** Step 5's outcome: either the situation is already decided, or this app is routed and the measurement decides. */
private sealed interface RoutingStep {
    data class Decided(
        val situation: Situation,
    ) : RoutingStep

    data class Routed(
        val fact: SelfRouting,
        /** Set when the fact is the last known one, kept through a Background re-read still within its grace. */
        val confirming: Staleness.Confirming?,
    ) : RoutingStep
}

private fun routingStep(
    routing: RoutingKnowledge,
    now: Long,
    grace: (ReadReason) -> Long,
): RoutingStep =
    when (routing) {
        is RoutingKnowledge.Unknown -> {
            RoutingStep.Decided(Situation.CouldNotCheck(CouldNotCheckCause.RoutingUnknown(routing.cause)))
        }

        is RoutingKnowledge.Verifying -> {
            verifyingStep(routing, now, grace)
        }

        is RoutingKnowledge.Known -> {
            knownRoutingStep(routing.routing, null)
        }
    }

/**
 * A re-read in flight. Past its grace — or with nothing to fall back on — the read
 * itself is what the user is looking at; within a Background grace the last known
 * fact still stands and only carries a confirmation mark.
 */
private fun verifyingStep(
    routing: RoutingKnowledge.Verifying,
    now: Long,
    grace: (ReadReason) -> Long,
): RoutingStep {
    val lastKnown = routing.lastKnown
    if (lastKnown == null || now - routing.since >= grace(routing.reason)) {
        return RoutingStep.Decided(Situation.Checking(CheckingWhat.VpnState, lastKnown, routing.reason, routing.since))
    }
    return knownRoutingStep(lastKnown, Staleness.Confirming(routing.reason, routing.since))
}

private fun knownRoutingStep(
    fact: SelfRouting,
    confirming: Staleness.Confirming?,
): RoutingStep =
    when (fact) {
        SelfRouting.VpnOff -> RoutingStep.Decided(Situation.VpnOff)
        SelfRouting.Excluded -> RoutingStep.Decided(Situation.NotMeasurable)
        SelfRouting.Routed -> RoutingStep.Routed(fact, confirming)
    }

/** Steps 6–9, reached only when this app is known to be routed through the tunnel. */
private fun measuredSituation(
    presentation: DiagnosticPresentation,
    now: Long,
    fact: SelfRouting,
    confirming: Staleness.Confirming?,
): Situation {
    activeRunSituation(presentation, now, fact)?.let { return it }
    // A confirmation the owner is about to request is the run it becomes: the stale
    // measurement it will replace is not asked for a manual re-check meanwhile.
    if (presentation.confirmationPending) return Situation.Checking(CheckingWhat.Suite, fact, ReadReason.Background, now)
    failedAttemptSituation(presentation)?.let { return it }
    noMeasurementSituation(presentation, now, fact)?.let { return it }
    val evidence = requireNotNull(presentation.evidence) { "a measurement always carries its evidence summary" }
    return Situation.Measured(evidence.conclusion, staleness(presentation.applicability, confirming))
}

/**
 * Step 6. An automatic confirmation of a measurement that still applies is silent:
 * nothing about the user's world changed, the run coordinator's own deadline bounds
 * it, and flipping to "Checking…" for its duration would be motion without news.
 * Anything else — an explicit re-run, or a run that has something new to say —
 * is the state.
 */
private fun activeRunSituation(
    presentation: DiagnosticPresentation,
    now: Long,
    fact: SelfRouting,
): Situation? {
    if (presentation.activeRunId == null) return null
    val automatic = presentation.activeRunAutomatic == true
    val evidence = presentation.evidence
    if (automatic && evidence != null && presentation.applicability == MeasurementApplicability.MatchesLastObservation) {
        return Situation.Measured(evidence.conclusion, Staleness.Confirming(ReadReason.Background, since = null))
    }
    val reason = if (automatic) ReadReason.Background else ReadReason.Explicit
    return Situation.Checking(CheckingWhat.Suite, fact, reason, now)
}

/**
 * Step 7. A run that never started because of a current condition is not a failed
 * check (I13) — the condition above already named it. A real failure is the
 * situation even when an older measurement exists: the history stays listed by the
 * surfaces, but the state says the last check could not run. An interruption with
 * an earlier measurement falls through to it instead; that measurement's
 * applicability is already Changed, which says the same thing more usefully.
 */
private fun failedAttemptSituation(presentation: DiagnosticPresentation): Situation? {
    val attempt = presentation.lastAttempt ?: return null
    if (attempt.outcome == RunOutcome.Completed || attempt.blocked) return null
    if (attempt.outcome == RunOutcome.Interrupted) {
        if (presentation.measurement != null) return null
        return Situation.CouldNotCheck(CouldNotCheckCause.Interrupted(attempt.failure))
    }
    return Situation.CouldNotCheck(CouldNotCheckCause.RunFailed(attempt.failure))
}

/**
 * Step 8. No measurement to present: either the completed run's evidence was
 * evicted, which is nothing to show and no reason to promise more, or the
 * automatic suite is still owed and the surfaces show its progress.
 */
private fun noMeasurementSituation(
    presentation: DiagnosticPresentation,
    now: Long,
    fact: SelfRouting,
): Situation? {
    if (presentation.measurement != null) return null
    if (presentation.lastAttempt?.outcome == RunOutcome.Completed) {
        return Situation.CouldNotCheck(CouldNotCheckCause.RunFailed(null))
    }
    return Situation.Checking(CheckingWhat.Suite, fact, ReadReason.Background, now)
}

/** Step 9. A re-read still inside its grace is what makes an Unverified measurement presentable. */
private fun staleness(
    applicability: MeasurementApplicability,
    confirming: Staleness.Confirming?,
): Staleness =
    when (applicability) {
        MeasurementApplicability.Changed -> Staleness.Changed
        MeasurementApplicability.Unverified -> confirming ?: Staleness.Confirming(ReadReason.Background, since = null)
        MeasurementApplicability.MatchesLastObservation, MeasurementApplicability.Absent -> confirming ?: Staleness.Current
    }

/**
 * The published classification: one value per presentation, plus a single
 * re-emission at the end of a Background grace so a read that is still in flight
 * stops being silent. It words what the presentation already says and never
 * schedules a run or a read (I16); a newer presentation cancels the pending
 * re-emission, because [transformLatest] cancels the block it came from.
 */
@OptIn(ExperimentalCoroutinesApi::class)
internal fun situationFlow(
    presentations: Flow<DiagnosticPresentation>,
    clock: () -> Long,
    wait: suspend (Long) -> Unit = { delay(it) },
): Flow<Situation> =
    presentations
        .transformLatest { presentation ->
            val now = clock()
            emit(situation(presentation, now))
            val remaining = backgroundGraceRemaining(presentation, now)
            if (remaining > 0) {
                wait(remaining)
                emit(situation(presentation, clock()))
            }
        }.distinctUntilChanged()

/** How long this presentation's classification still depends on an unexpired grace; 0 when it does not. */
private fun backgroundGraceRemaining(
    presentation: DiagnosticPresentation,
    now: Long,
    grace: (ReadReason) -> Long = SituationGrace::of,
): Long {
    val routing = presentation.routing as? RoutingKnowledge.Verifying ?: return 0
    if (routing.lastKnown == null) return 0
    return (routing.since + grace(routing.reason) - now).coerceAtLeast(0)
}
