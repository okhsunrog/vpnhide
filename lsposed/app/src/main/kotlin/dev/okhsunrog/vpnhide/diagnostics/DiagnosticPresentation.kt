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
 *
 * [routing] is the same routing observation [eligibility] is decided from, kept
 * as knowledge instead of as an admission verdict: it carries the last known fact
 * through a re-read and says why that re-read runs. `eligibility` stays the
 * decision (the run coordinator and the bundle read it); a surface that wants to
 * keep saying "VPN is off" while the fact is being re-read reads [routing].
 */
internal data class DiagnosticPresentation(
    val eligibility: DiagnosticEligibility,
    val routing: RoutingKnowledge,
    val activeRunId: Long?,
    val activeStage: DiagnosticStage?,
    val activeResults: CheckResults?,
    val lastAttempt: DiagnosticAttempt?,
    val measurement: DiagnosticMeasurement?,
    val measurementResults: CheckResults?,
    val applicability: MeasurementApplicability,
    val evidence: MeasurementEvidence?,
    val currentSuccess: Boolean,
    val probeUnavailable: Boolean,
)

/**
 * The legacy gate the Dashboard tiles and the bridge's `gate`/`report` are built
 * under: `ROUTED` only for a complete measurement whose evidence is retained,
 * otherwise the blocking eligibility of a latest attempt that never started, or
 * null when nothing measurable is known (no attempt, a failed or interrupted one,
 * a condition with no gate vocabulary). Read [applicability] next to it: a `ROUTED`
 * gate describes the measurement, not necessarily the current conditions.
 */
internal fun DiagnosticPresentation.reportGate(): DiagnosticGate? =
    when {
        measurement != null && measurementResults != null -> DiagnosticGate.ROUTED
        else -> lastAttempt?.eligibility?.blockedGate()
    }

/**
 * [current] is the context observation built from the current routing
 * observation, root snapshot, config and impact state, or null when no routing
 * observation exists yet. [routing] is that same routing observation as
 * knowledge; anything but [RoutingKnowledge.Known] means the observation is not
 * current (loading, invalidated, failed or quarantined), so the measurement is
 * Unverified until a consistent reobservation restores it.
 */
internal fun diagnosticPresentation(
    view: DiagnosticRunView,
    current: DiagnosticContextObservation?,
    changeEpoch: Long,
    routing: RoutingKnowledge,
): DiagnosticPresentation {
    val core = view.core
    val measurement = core.lastComplete
    val applicability = measurementApplicability(measurement, current?.context, changeEpoch, routing !is RoutingKnowledge.Known)
    val eligibility = current?.eligibility ?: DiagnosticEligibility.Checking
    return DiagnosticPresentation(
        eligibility = eligibility,
        routing = routing,
        activeRunId = core.active?.id,
        activeStage = core.active?.stage,
        activeResults = view.activeResults,
        lastAttempt = core.lastAttempt,
        measurement = measurement,
        measurementResults = measurement?.let { view.attemptResults[it.runId] },
        applicability = applicability,
        evidence = measurement?.let(::summarizeMeasurement),
        currentSuccess = canPresentCurrentSuccess(measurement, applicability, eligibility),
        probeUnavailable = core.quarantined,
    )
}
