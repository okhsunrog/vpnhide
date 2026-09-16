package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.TransitionFailure
import kotlinx.serialization.Serializable

/**
 * The presentation projection as the bridge and the bundle carry it: the same
 * eligibility, run identities, applicability and evidence sufficiency the
 * screens render, so an agent read or a bug report can never disagree with what
 * the user saw (T28). Additive next to the legacy `gate`/`report` fields, which
 * keep their meaning; the dates are the measurement's own, not the payload's.
 */
@Serializable
internal data class DiagnosticSummaryInfo(
    val eligibility: DiagnosticEligibility,
    val activeRunId: Long?,
    val activeStage: DiagnosticStage?,
    val lastAttempt: DiagnosticAttemptInfo?,
    val measurement: DiagnosticMeasurementInfo?,
    val applicability: MeasurementApplicability,
    val evidence: MeasurementEvidence?,
    val currentSuccess: Boolean,
    /** The probe helper did not return from an earlier run, so no new run can start. */
    val probeUnavailable: Boolean,
)

@Serializable
internal data class DiagnosticAttemptInfo(
    val runId: Long,
    val outcome: RunOutcome,
    val failure: TransitionFailure?,
    val eligibility: DiagnosticEligibility?,
)

@Serializable
internal data class DiagnosticMeasurementInfo(
    val runId: Long,
    val startedAt: Long,
    val endedAt: Long,
    val completed: Boolean,
    val interrupted: Boolean,
    val observationId: Long,
)

internal fun diagnosticSummary(presentation: DiagnosticPresentation): DiagnosticSummaryInfo =
    DiagnosticSummaryInfo(
        eligibility = presentation.eligibility,
        activeRunId = presentation.activeRunId,
        activeStage = presentation.activeStage,
        lastAttempt =
            presentation.lastAttempt?.let {
                DiagnosticAttemptInfo(it.id, it.outcome, it.failure, it.eligibility)
            },
        measurement =
            presentation.measurement?.let {
                DiagnosticMeasurementInfo(it.runId, it.startedAt, it.endedAt, it.completed, it.interrupted, it.context.observationId)
            },
        applicability = presentation.applicability,
        evidence = presentation.evidence,
        currentSuccess = presentation.currentSuccess,
        probeUnavailable = presentation.probeUnavailable,
    )
