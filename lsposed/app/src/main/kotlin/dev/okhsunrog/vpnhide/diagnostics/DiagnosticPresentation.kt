package dev.okhsunrog.vpnhide.diagnostics

/**
 * One immutable projection of the diagnostic domain for every consumer
 * (screens, bridge, bundle): the shared eligibility decision, the active run,
 * the latest attempt, the latest complete measurement with its applicability to
 * the current observations, and the evidence summary. It is derived purely from
 * the run view, the current context observation and the impact state, so no
 * consumer combines flows of different vintages or recomputes a verdict.
 *
 * [currentSuccess] is the only positive claim: completed execution, sufficient
 * evidence, an applicable measurement and eligible current conditions. Everything
 * else is history with its qualifier ([applicability], [lastAttempt]).
 *
 * [probeUnavailable] is a property of this process, not of any measurement: the
 * probe helper of an earlier run never returned within its drain deadline, so the
 * resource stays quarantined (§7) and every new request is rejected until that
 * helper returns. The retained results are as good as they were; what is gone is
 * the ability to take a new one, and the user has to be told that (§1).
 */
internal data class DiagnosticPresentation(
    val eligibility: DiagnosticEligibility,
    val activeRunId: Long?,
    val activeStage: DiagnosticStage?,
    val activeResults: CheckResults?,
    val lastAttempt: DiagnosticAttempt?,
    val lastAttemptResults: CheckResults?,
    val measurement: DiagnosticMeasurement?,
    val measurementResults: CheckResults?,
    val applicability: MeasurementApplicability,
    val evidence: MeasurementEvidence?,
    val currentSuccess: Boolean,
    val probeUnavailable: Boolean,
) {
    /** The latest attempt did not complete while an older complete measurement still exists. */
    val staleFailureVisible: Boolean
        get() = lastAttempt != null && lastAttempt.outcome != RunOutcome.Completed && measurement != null
}

/**
 * [current] is the context observation built from the current routing
 * observation, root snapshot, config and impact state, or null when no routing
 * observation exists yet. [uncertain] is true while the routing observation is
 * not current (loading, invalidated, failed or quarantined): the measurement is
 * then Unverified until a consistent reobservation restores it.
 */
internal fun diagnosticPresentation(
    view: DiagnosticRunView,
    current: DiagnosticContextObservation?,
    changeEpoch: Long,
    uncertain: Boolean,
): DiagnosticPresentation {
    val core = view.core
    val measurement = core.lastComplete
    val applicability = measurementApplicability(measurement, current?.context, changeEpoch, uncertain)
    val eligibility = current?.eligibility ?: DiagnosticEligibility.Checking
    return DiagnosticPresentation(
        eligibility = eligibility,
        activeRunId = core.active?.id,
        activeStage = core.active?.stage,
        activeResults = view.activeResults,
        lastAttempt = core.lastAttempt,
        lastAttemptResults = core.lastAttempt?.let { view.attemptResults[it.id] },
        measurement = measurement,
        measurementResults = measurement?.let { view.attemptResults[it.runId] },
        applicability = applicability,
        evidence = measurement?.let(::summarizeMeasurement),
        currentSuccess = canPresentCurrentSuccess(measurement, applicability, eligibility),
        probeUnavailable = core.quarantined,
    )
}
