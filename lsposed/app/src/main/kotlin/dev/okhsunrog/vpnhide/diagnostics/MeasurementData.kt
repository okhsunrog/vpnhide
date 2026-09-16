package dev.okhsunrog.vpnhide.diagnostics

import kotlinx.serialization.Serializable

internal data class ProbePlanEntry(
    val id: String,
    val owned: Boolean = true,
)

/** Adapter-provided identities, not arbitrary canonical JSON or secrets. */
internal data class MeasurementContext(
    val subject: String,
    val configuration: String,
    val routing: String,
    val coverage: String,
    val changeEpoch: Long,
    val observationId: Long,
    val observedAt: Long,
)

internal fun sameMeasurementConditions(
    first: MeasurementContext,
    second: MeasurementContext,
): Boolean =
    first.subject == second.subject && first.configuration == second.configuration &&
        first.routing == second.routing && first.coverage == second.coverage && first.changeEpoch == second.changeEpoch

internal data class DiagnosticMeasurement(
    val runId: Long,
    val context: MeasurementContext,
    val plan: List<ProbePlanEntry>,
    val outcomes: Map<String, CheckOutcome>,
    val completed: Boolean,
    val interrupted: Boolean,
    val endedAt: Long,
    val startedAt: Long = context.observedAt,
    val endContext: MeasurementContext? = null,
)

internal enum class MeasurementApplicability { Absent, Changed, Unverified, MatchesLastObservation }

internal fun measurementApplicability(
    measurement: DiagnosticMeasurement?,
    current: MeasurementContext?,
    knownChangeEpoch: Long,
    uncertain: Boolean,
): MeasurementApplicability =
    when {
        measurement == null -> MeasurementApplicability.Absent
        measurement.interrupted || knownChangeEpoch > measurement.context.changeEpoch -> MeasurementApplicability.Changed
        current == null || uncertain -> MeasurementApplicability.Unverified
        !sameMeasurementConditions(measurement.context, current) -> MeasurementApplicability.Changed
        else -> MeasurementApplicability.MatchesLastObservation
    }

internal enum class EvidenceConclusion { OwnedLeak, Insufficient, Partial, NoObservedLeak }

@Serializable
internal data class MeasurementEvidence(
    val hidden: Int,
    val systemBlocked: Int,
    val nothingToLeak: Int,
    val leaks: Int,
    val notMeasured: Int,
    val notRun: Int,
    val uncoveredLeaks: Int,
    val conclusion: EvidenceConclusion,
)

internal fun summarizeMeasurement(measurement: DiagnosticMeasurement): MeasurementEvidence {
    val owned = measurement.plan.filter { it.owned }.map { measurement.outcomes[it.id] }
    val hidden = owned.count { it == CheckOutcome.HiddenByBackend }
    val blocked = owned.count { it == CheckOutcome.HiddenBySelinux }
    val leaks = owned.count { it == CheckOutcome.Leak }
    val notMeasured = owned.count { it is CheckOutcome.NotMeasured }
    val notRun = owned.count { it == null }
    val conclusion =
        when {
            leaks > 0 -> EvidenceConclusion.OwnedLeak
            hidden + blocked == 0 -> EvidenceConclusion.Insufficient
            !measurement.completed || measurement.interrupted || notMeasured + notRun > 0 -> EvidenceConclusion.Partial
            else -> EvidenceConclusion.NoObservedLeak
        }
    return MeasurementEvidence(
        hidden,
        blocked,
        owned.count { it == CheckOutcome.NothingToLeak },
        leaks,
        notMeasured,
        notRun,
        measurement.plan.count { !it.owned && measurement.outcomes[it.id] == CheckOutcome.Leak },
        conclusion,
    )
}

internal fun canPresentCurrentSuccess(
    measurement: DiagnosticMeasurement?,
    applicability: MeasurementApplicability,
    eligibility: DiagnosticEligibility,
): Boolean =
    eligibility == DiagnosticEligibility.Eligible && measurement != null && measurement.completed &&
        applicability == MeasurementApplicability.MatchesLastObservation &&
        summarizeMeasurement(measurement).conclusion == EvidenceConclusion.NoObservedLeak
