package dev.okhsunrog.vpnhide.diagnostics

/**
 * What the Diagnostics screen shows above the per-check list: the wording set for
 * the one [Situation], as the Dashboard hero has its own in
 * [dev.okhsunrog.vpnhide.HeroVisual]. No precedence lives here — the screen and
 * the hero read the same classification, so a blocked condition, an execution
 * failure, a detected leak and a changed measurement cannot mean one thing on one
 * surface and another on the other (I13).
 */
internal enum class DiagnosticBanner {
    /** Nothing to show yet: inputs not supplied, or a run with no evidence so far. */
    Progress,
    RestartApp,
    VpnOff,
    SelfExcluded,

    /** A relevant configuration change is still being applied; the check waits for it. */
    Applying,

    /** The last relevant configuration change stayed unresolved; manual recheck first. */
    ApplicationUnknown,

    /** The last relevant configuration change failed; repair it, then re-check. */
    ApplicationFailed,

    /** Routing could not be determined; explicit, never an endless spinner (T17). */
    RoutingUnknown,

    /**
     * The probe resource is quarantined: a helper of an earlier run never returned,
     * so no new check can start until it does. Says so instead of offering a
     * Re-check button that is silently rejected.
     */
    ProbeUnavailable,

    /** The latest attempt could not execute; an older measurement may still be listed as history. */
    Failed,

    /** The latest attempt was interrupted by a change and there is no complete measurement. */
    Interrupted,

    /** A complete measurement that describes the current conditions and carries hiding evidence. */
    Ready,

    /** A complete measurement taken under conditions that have since changed. */
    ResultsChanged,

    /** A complete measurement whose applicability is being re-established by a read in flight. */
    ResultsUnverified,

    /** A complete measurement that measured nothing attributable, so it proves nothing. */
    InsufficientEvidence,
}

/** A notice about the latest attempt shown beside an older complete measurement. */
internal enum class DiagnosticAttemptNotice { Failed, Interrupted }

internal data class DiagnosticScreenDecision(
    val banner: DiagnosticBanner,
    val attemptNotice: DiagnosticAttemptNotice? = null,
    /** The check results to list, if any: the measurement's, or the active run's partial evidence. */
    val results: CheckResults? = null,
    /** False while the listed results are an active run's partial evidence. */
    val complete: Boolean = true,
    /** The layers the listed measurement was taken against; null for partial evidence, whose report uses the live layers. */
    val coverage: MeasurementCoverage? = null,
    /** A condition's prompt is shown while a re-read runs: its button is busy, not tappable. */
    val checking: Boolean = false,
)

/**
 * The banner is the [situation]; the side channels — which results to list, whether
 * they are complete, the layers they were measured against and the notice about the
 * latest attempt — come from the [presentation], which the Situation deliberately
 * does not carry.
 */
internal fun diagnosticScreenDecision(
    situation: Situation,
    presentation: DiagnosticPresentation,
): DiagnosticScreenDecision {
    val decision = situationBanner(situation, presentation)
    val history = presentation.measurementResults
    if (history == null || decision.results !== history) return decision
    // A retained measurement is attributed with the backend it was measured against (§6),
    // and carries the notice about an attempt that did not complete after it.
    return decision.copy(
        attemptNotice = historyNotice(decision.banner, presentation.lastAttempt),
        coverage = presentation.measurement?.context?.coverageLayers,
    )
}

private fun situationBanner(
    situation: Situation,
    presentation: DiagnosticPresentation,
): DiagnosticScreenDecision =
    when (situation) {
        Situation.Initializing -> {
            DiagnosticScreenDecision(DiagnosticBanner.Progress)
        }

        is Situation.Checking -> {
            checkingDecision(situation, presentation)
        }

        Situation.VpnOff -> {
            DiagnosticScreenDecision(DiagnosticBanner.VpnOff)
        }

        Situation.NotMeasurable -> {
            DiagnosticScreenDecision(DiagnosticBanner.SelfExcluded)
        }

        is Situation.ActionNeeded -> {
            actionNeededDecision(situation.kind, presentation)
        }

        is Situation.CouldNotCheck -> {
            couldNotCheckDecision(situation.cause, presentation)
        }

        is Situation.Measured -> {
            DiagnosticScreenDecision(measuredBanner(situation), results = presentation.measurementResults)
        }
    }

/**
 * A read or a run in flight. The prompts (VPN off, self excluded) replace the list
 * as they always have, now with a busy button instead of a bare spinner over a
 * known condition; everything else keeps the history listed so it stays visible
 * while the banner says why it cannot be confirmed right now (T17).
 */
private fun checkingDecision(
    checking: Situation.Checking,
    presentation: DiagnosticPresentation,
): DiagnosticScreenDecision =
    when (checking.what) {
        CheckingWhat.ConfigApplying -> {
            DiagnosticScreenDecision(DiagnosticBanner.Applying, results = presentation.measurementResults)
        }

        CheckingWhat.Suite -> {
            activeRunDecision(presentation)
        }

        CheckingWhat.VpnState -> {
            routingReadDecision(checking.lastKnown, presentation)
        }
    }

/** A suite in flight shows its partial evidence as soon as it has any; a draining run has nothing new to add. */
private fun activeRunDecision(presentation: DiagnosticPresentation): DiagnosticScreenDecision {
    val stage = presentation.activeStage
    val partial = presentation.activeResults
    if (partial == null || stage == null || stage == DiagnosticStage.Draining) {
        return DiagnosticScreenDecision(DiagnosticBanner.Progress)
    }
    return DiagnosticScreenDecision(DiagnosticBanner.Progress, results = partial, complete = false)
}

private fun routingReadDecision(
    lastKnown: SelfRouting?,
    presentation: DiagnosticPresentation,
): DiagnosticScreenDecision =
    when (lastKnown) {
        SelfRouting.VpnOff -> {
            DiagnosticScreenDecision(DiagnosticBanner.VpnOff, checking = true)
        }

        SelfRouting.Excluded -> {
            DiagnosticScreenDecision(DiagnosticBanner.SelfExcluded, checking = true)
        }

        SelfRouting.Routed, null -> {
            if (presentation.measurement != null) {
                DiagnosticScreenDecision(DiagnosticBanner.ResultsUnverified, results = presentation.measurementResults)
            } else {
                DiagnosticScreenDecision(DiagnosticBanner.Progress)
            }
        }
    }

private fun actionNeededDecision(
    kind: ActionNeededKind,
    presentation: DiagnosticPresentation,
): DiagnosticScreenDecision =
    when (kind) {
        ActionNeededKind.RestartApp, ActionNeededKind.RestartDevice -> {
            DiagnosticScreenDecision(DiagnosticBanner.RestartApp)
        }

        ActionNeededKind.ApplicationUnknown -> {
            DiagnosticScreenDecision(DiagnosticBanner.ApplicationUnknown, results = presentation.measurementResults)
        }

        ActionNeededKind.ApplicationFailed -> {
            DiagnosticScreenDecision(DiagnosticBanner.ApplicationFailed, results = presentation.measurementResults)
        }
    }

/** No current answer; an older measurement is still worth listing under every cause but an interruption, which never has one. */
private fun couldNotCheckDecision(
    cause: CouldNotCheckCause,
    presentation: DiagnosticPresentation,
): DiagnosticScreenDecision =
    when (cause) {
        is CouldNotCheckCause.RoutingUnknown -> {
            DiagnosticScreenDecision(DiagnosticBanner.RoutingUnknown, results = presentation.measurementResults)
        }

        CouldNotCheckCause.ProbeUnavailable -> {
            DiagnosticScreenDecision(DiagnosticBanner.ProbeUnavailable, results = presentation.measurementResults)
        }

        is CouldNotCheckCause.RunFailed -> {
            DiagnosticScreenDecision(DiagnosticBanner.Failed, results = presentation.measurementResults)
        }

        is CouldNotCheckCause.Interrupted -> {
            DiagnosticScreenDecision(DiagnosticBanner.Interrupted)
        }
    }

/** Sufficiency before staleness: results that prove nothing are never presented as ready. */
private fun measuredBanner(measured: Situation.Measured): DiagnosticBanner =
    when {
        measured.evidence == EvidenceConclusion.Insufficient -> DiagnosticBanner.InsufficientEvidence

        measured.staleness == Staleness.Changed -> DiagnosticBanner.ResultsChanged

        // Confirming renders like Current: a re-read inside its grace is silent by design.
        else -> DiagnosticBanner.Ready
    }

/**
 * The notice beside listed history. A run that never started because of a condition
 * is not a failed check (I13), and a banner that already says the last attempt
 * failed or was interrupted does not need the same sentence twice.
 */
private fun historyNotice(
    banner: DiagnosticBanner,
    attempt: DiagnosticAttempt?,
): DiagnosticAttemptNotice? {
    if (banner == DiagnosticBanner.Failed || banner == DiagnosticBanner.Interrupted) return null
    if (attempt == null || attempt.outcome == RunOutcome.Completed || attempt.blocked) return null
    return when (attempt.outcome) {
        RunOutcome.Interrupted -> DiagnosticAttemptNotice.Interrupted
        else -> DiagnosticAttemptNotice.Failed
    }
}
