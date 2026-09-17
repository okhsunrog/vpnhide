package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.ObservationClock
import dev.okhsunrog.vpnhide.ReadReason
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
    /** The last routing fact this process had, kept across a re-read; null when it never had one. */
    val lastKnownRouting: SelfRouting?,
    /** Present only while a routing re-read is owed or in flight. */
    val routingRead: RoutingReadInfo?,
)

/** A routing re-read in flight at assembly time: why it runs, and how long it has been owed. */
@Serializable
internal data class RoutingReadInfo(
    val reason: ReadReason,
    val pendingMs: Long,
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

/** [now] is the coordinator's monotonic base ([ObservationClock]), the base a pending read's `since` is in. */
internal fun diagnosticSummary(
    presentation: DiagnosticPresentation,
    now: Long = ObservationClock.now(),
): DiagnosticSummaryInfo =
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
        lastKnownRouting =
            when (val routing = presentation.routing) {
                is RoutingKnowledge.Known -> routing.routing
                is RoutingKnowledge.Verifying -> routing.lastKnown
                is RoutingKnowledge.Unknown -> routing.lastKnown
            },
        routingRead =
            (presentation.routing as? RoutingKnowledge.Verifying)?.let {
                RoutingReadInfo(it.reason, (now - it.since).coerceAtLeast(0))
            },
    )
