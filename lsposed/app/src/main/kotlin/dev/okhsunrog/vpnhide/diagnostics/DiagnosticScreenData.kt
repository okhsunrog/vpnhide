package dev.okhsunrog.vpnhide.diagnostics

/**
 * What the Diagnostics screen shows above the per-check list. Decided purely from
 * [DiagnosticPresentation] (classify here, word it in the screen), in one fixed
 * precedence: current conditions first, then the run in flight, then the latest
 * attempt, then the applicability and sufficiency of the latest complete
 * measurement. Blocked conditions, an execution failure, a detected leak and a
 * changed measurement therefore never collapse into one another (I13).
 */
internal enum class DiagnosticBanner {
    /** Eligibility not known yet, or a run started and has no evidence to show. */
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

    /** The latest attempt failed and there is no complete measurement to fall back on. */
    Failed,

    /** The latest attempt was interrupted by a change and there is no complete measurement. */
    Interrupted,

    /** A complete measurement that matches the last observation and carries hiding evidence. */
    Ready,

    /** A complete measurement taken under conditions that have since changed. */
    ResultsChanged,

    /** A complete measurement whose applicability is being re-established. */
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
)

internal fun diagnosticScreenDecision(presentation: DiagnosticPresentation): DiagnosticScreenDecision {
    val decision = blockedBanner(presentation) ?: runOrHistory(presentation)
    // A retained measurement is attributed with the backend it was measured against (§6).
    val retained = decision.results != null && decision.results === presentation.measurementResults
    return if (retained) decision.copy(coverage = presentation.measurement?.context?.coverageLayers) else decision
}

private fun runOrHistory(presentation: DiagnosticPresentation): DiagnosticScreenDecision {
    // A current condition still outranks it — it explains the same "cannot measure
    // now" with more detail — but a quarantined probe outranks the history itself,
    // which would otherwise carry a Re-check button that does nothing.
    if (presentation.probeUnavailable) {
        return DiagnosticScreenDecision(DiagnosticBanner.ProbeUnavailable, results = presentation.measurementResults)
    }
    val active = presentation.activeStage
    if (active != null && presentation.activeResults != null && active != DiagnosticStage.Draining) {
        return DiagnosticScreenDecision(DiagnosticBanner.Progress, results = presentation.activeResults, complete = false)
    }
    if (active != null) return DiagnosticScreenDecision(DiagnosticBanner.Progress)
    val attempt = presentation.lastAttempt
    val measurement = presentation.measurement
    val notice = attempt?.takeIf { it.outcome != RunOutcome.Completed }?.let(::attemptNotice)
    if (measurement == null) {
        return DiagnosticScreenDecision(
            when (notice) {
                DiagnosticAttemptNotice.Interrupted -> DiagnosticBanner.Interrupted
                DiagnosticAttemptNotice.Failed -> DiagnosticBanner.Failed
                null -> DiagnosticBanner.Progress
            },
        )
    }
    return DiagnosticScreenDecision(measurementBanner(presentation), notice, presentation.measurementResults)
}

/**
 * Current conditions outrank everything. The existing prompts (restart, VPN off,
 * self excluded) replace the list as before; the explicit condition banners and a
 * Checking observation keep the last complete measurement listed, so history stays
 * visible while the banner says why it cannot be confirmed right now (T17).
 */
private fun blockedBanner(presentation: DiagnosticPresentation): DiagnosticScreenDecision? =
    when (presentation.eligibility) {
        DiagnosticEligibility.Initializing -> {
            DiagnosticScreenDecision(DiagnosticBanner.Progress)
        }

        DiagnosticEligibility.Checking -> {
            if (presentation.measurement != null && presentation.activeStage == null) {
                DiagnosticScreenDecision(DiagnosticBanner.ResultsUnverified, results = presentation.measurementResults)
            } else {
                DiagnosticScreenDecision(DiagnosticBanner.Progress)
            }
        }

        DiagnosticEligibility.RestartApp, DiagnosticEligibility.RestartDevice -> {
            DiagnosticScreenDecision(DiagnosticBanner.RestartApp)
        }

        DiagnosticEligibility.VpnOff -> {
            DiagnosticScreenDecision(DiagnosticBanner.VpnOff)
        }

        DiagnosticEligibility.SelfExcluded -> {
            DiagnosticScreenDecision(DiagnosticBanner.SelfExcluded)
        }

        DiagnosticEligibility.Applying -> {
            DiagnosticScreenDecision(DiagnosticBanner.Applying, results = presentation.measurementResults)
        }

        DiagnosticEligibility.ApplicationUnknown -> {
            DiagnosticScreenDecision(DiagnosticBanner.ApplicationUnknown, results = presentation.measurementResults)
        }

        DiagnosticEligibility.ApplicationFailed -> {
            DiagnosticScreenDecision(DiagnosticBanner.ApplicationFailed, results = presentation.measurementResults)
        }

        DiagnosticEligibility.Unknown -> {
            DiagnosticScreenDecision(DiagnosticBanner.RoutingUnknown, results = presentation.measurementResults)
        }

        DiagnosticEligibility.Eligible -> {
            null
        }
    }

private fun attemptNotice(attempt: DiagnosticAttempt): DiagnosticAttemptNotice =
    when (attempt.outcome) {
        RunOutcome.Interrupted -> DiagnosticAttemptNotice.Interrupted
        else -> DiagnosticAttemptNotice.Failed
    }

/** Sufficiency before applicability: results that prove nothing are never presented as ready. */
private fun measurementBanner(presentation: DiagnosticPresentation): DiagnosticBanner =
    when {
        presentation.evidence?.conclusion == EvidenceConclusion.Insufficient -> DiagnosticBanner.InsufficientEvidence
        presentation.applicability == MeasurementApplicability.Changed -> DiagnosticBanner.ResultsChanged
        presentation.applicability == MeasurementApplicability.Unverified -> DiagnosticBanner.ResultsUnverified
        else -> DiagnosticBanner.Ready
    }
