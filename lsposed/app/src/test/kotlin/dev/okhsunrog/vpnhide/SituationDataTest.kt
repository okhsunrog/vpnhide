package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.ActionNeededKind
import dev.okhsunrog.vpnhide.diagnostics.CheckOutcome
import dev.okhsunrog.vpnhide.diagnostics.CheckingWhat
import dev.okhsunrog.vpnhide.diagnostics.CouldNotCheckCause
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticAttempt
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticEligibility
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticMeasurement
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticPresentation
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticStage
import dev.okhsunrog.vpnhide.diagnostics.EvidenceConclusion
import dev.okhsunrog.vpnhide.diagnostics.MeasurementApplicability
import dev.okhsunrog.vpnhide.diagnostics.MeasurementContext
import dev.okhsunrog.vpnhide.diagnostics.NATIVE_CHECKS
import dev.okhsunrog.vpnhide.diagnostics.ProbePlanEntry
import dev.okhsunrog.vpnhide.diagnostics.RoutingKnowledge
import dev.okhsunrog.vpnhide.diagnostics.RunOutcome
import dev.okhsunrog.vpnhide.diagnostics.SelfRouting
import dev.okhsunrog.vpnhide.diagnostics.Situation
import dev.okhsunrog.vpnhide.diagnostics.Staleness
import dev.okhsunrog.vpnhide.diagnostics.situation
import dev.okhsunrog.vpnhide.diagnostics.situationFlow
import dev.okhsunrog.vpnhide.diagnostics.summarizeMeasurement
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.test.currentTime
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Test

/** One fixture of the design review's scenario table (docs/notes/ui-state-presentation-review.md §2). */
private data class Row(
    val name: String,
    val presentation: DiagnosticPresentation,
    val expected: Situation,
    val now: Long = SituationDataTest.NOW,
)

// TestScope.currentTime is the virtual clock the grace re-emission is pinned against.
@OptIn(ExperimentalCoroutinesApi::class)
class SituationDataTest {
    @Test
    fun `the scenario table classifies every row as one whole situation`() {
        (conditionRows() + routingRows() + runRows() + measurementRows()).forEach { row ->
            assertEquals(row.name, row.expected, situation(row.presentation, row.now))
        }
    }

    /** Steps 1–4: what this process and the configuration are doing outranks every routing fact. */
    private fun conditionRows(): List<Row> =
        listOf(
            Row(
                "cold start before any input is supplied",
                presentation(DiagnosticEligibility.Initializing, RoutingKnowledge.Verifying(null, ReadReason.Background, 0)),
                Situation.Initializing,
            ),
            Row(
                "a quarantined probe outranks even a known VPN-off",
                presentation(DiagnosticEligibility.VpnOff, off, measurement(), probeUnavailable = true),
                Situation.CouldNotCheck(CouldNotCheckCause.ProbeUnavailable),
            ),
            Row(
                "restart pending after the app added itself as a target",
                presentation(DiagnosticEligibility.RestartApp, routed, measurement()),
                Situation.ActionNeeded(ActionNeededKind.RestartApp),
            ),
            Row(
                "a device restart is owed",
                presentation(DiagnosticEligibility.RestartDevice, routed),
                Situation.ActionNeeded(ActionNeededKind.RestartDevice),
            ),
            Row(
                "the last relevant configuration change failed",
                presentation(DiagnosticEligibility.ApplicationFailed, routed, measurement()),
                Situation.ActionNeeded(ActionNeededKind.ApplicationFailed),
            ),
            Row(
                "own-roles save still applying is a wait, not a problem",
                presentation(DiagnosticEligibility.Applying, routed, measurement()),
                Situation.Checking(CheckingWhat.ConfigApplying, SelfRouting.Routed, ReadReason.Explicit, NOW),
            ),
        )

    /** Step 5: the routing knowledge, with the Background grace that keeps a routine top-up silent. */
    private fun routingRows(): List<Row> =
        listOf(
            Row("cold start with the VPN off", presentation(DiagnosticEligibility.VpnOff, off), Situation.VpnOff),
            Row(
                "this app is excluded from the tunnel",
                presentation(DiagnosticEligibility.SelfExcluded, RoutingKnowledge.Known(SelfRouting.Excluded, 5)),
                Situation.NotMeasurable,
            ),
            Row(
                "a failed routing read keeps its last fact as history only",
                presentation(
                    DiagnosticEligibility.Unknown,
                    RoutingKnowledge.Unknown(TransitionFailure.ReadFailed, SelfRouting.Routed),
                    measurement(),
                ),
                Situation.CouldNotCheck(CouldNotCheckCause.RoutingUnknown(TransitionFailure.ReadFailed)),
            ),
            Row(
                "a network handover re-read is visible immediately",
                checking(RoutingKnowledge.Verifying(SelfRouting.Routed, ReadReason.Transition, NOW), measurement()),
                Situation.Checking(CheckingWhat.VpnState, SelfRouting.Routed, ReadReason.Transition, NOW),
            ),
            Row(
                "the VPN went off: the new fact replaces the old measurement's claim",
                presentation(DiagnosticEligibility.VpnOff, off, measurement(), MeasurementApplicability.Changed),
                Situation.VpnOff,
            ),
            Row(
                "the VPN comes back: the re-read keeps the VPN-off fact while it runs",
                checking(RoutingKnowledge.Verifying(SelfRouting.VpnOff, ReadReason.Transition, NOW), measurement()),
                Situation.Checking(CheckingWhat.VpnState, SelfRouting.VpnOff, ReadReason.Transition, NOW),
            ),
            Row(
                "Retry with the VPN off never claims the old measurement",
                checking(RoutingKnowledge.Verifying(SelfRouting.VpnOff, ReadReason.Explicit, NOW - 5), measurement()),
                Situation.Checking(CheckingWhat.VpnState, SelfRouting.VpnOff, ReadReason.Explicit, NOW - 5),
            ),
            Row(
                "startup reconcile within its grace is silent",
                checking(RoutingKnowledge.Verifying(SelfRouting.Routed, ReadReason.Background, NOW - 500), measurement()),
                Situation.Measured(EvidenceConclusion.NoObservedLeak, Staleness.Confirming(ReadReason.Background, NOW - 500)),
            ),
            Row(
                "the same reconcile past its grace says so",
                checking(RoutingKnowledge.Verifying(SelfRouting.Routed, ReadReason.Background, NOW - 2_000), measurement()),
                Situation.Checking(CheckingWhat.VpnState, SelfRouting.Routed, ReadReason.Background, NOW - 2_000),
            ),
            Row(
                "a first read has nothing to stay silent about",
                checking(RoutingKnowledge.Verifying(null, ReadReason.Background, NOW - 10), null),
                Situation.Checking(CheckingWhat.VpnState, null, ReadReason.Background, NOW - 10),
            ),
        )

    /** Step 6 and step 8: a run in flight, and the suite the process still owes. */
    private fun runRows(): List<Row> =
        listOf(
            Row(
                "Retry with the VPN on is an explicit re-run, not a green claim",
                presentation(routing = routed, measurement = measurement(), activeRunId = 5, activeRunAutomatic = false),
                Situation.Checking(CheckingWhat.Suite, SelfRouting.Routed, ReadReason.Explicit, NOW),
            ),
            Row(
                "an automatic confirmation of a still-applicable measurement is silent",
                presentation(routing = routed, measurement = measurement(), activeRunId = 5, activeRunAutomatic = true),
                Situation.Measured(EvidenceConclusion.NoObservedLeak, Staleness.Confirming(ReadReason.Background, null)),
            ),
            Row(
                "an automatic run after a change has something to say",
                presentation(
                    routing = routed,
                    measurement = measurement(),
                    applicability = MeasurementApplicability.Changed,
                    activeRunId = 5,
                    activeRunAutomatic = true,
                ),
                Situation.Checking(CheckingWhat.Suite, SelfRouting.Routed, ReadReason.Background, NOW),
            ),
            Row(
                "nothing measured and nothing attempted: the suite is owed",
                presentation(routing = routed),
                Situation.Checking(CheckingWhat.Suite, SelfRouting.Routed, ReadReason.Background, NOW),
            ),
            Row(
                "a confirmation about to be requested is the run it becomes, not a stale result to re-check",
                presentation(
                    routing = routed,
                    measurement = measurement(),
                    applicability = MeasurementApplicability.Changed,
                    confirmationPending = true,
                ),
                Situation.Checking(CheckingWhat.Suite, SelfRouting.Routed, ReadReason.Background, NOW),
            ),
            Row(
                "a completed attempt whose evidence was evicted has nothing to show",
                presentation(routing = routed, lastAttempt = DiagnosticAttempt(1, RunOutcome.Completed)),
                Situation.CouldNotCheck(CouldNotCheckCause.RunFailed(null)),
            ),
        )

    /** Steps 7 and 9: the latest attempt, then the measurement and how stale it is. */
    private fun measurementRows(): List<Row> =
        listOf(
            Row(
                "root revoked: the latest check could not run, whatever history exists",
                presentation(
                    routing = routed,
                    measurement = measurement(),
                    lastAttempt = DiagnosticAttempt(2, RunOutcome.Failed, TransitionFailure.ExecutionFailed),
                ),
                Situation.CouldNotCheck(CouldNotCheckCause.RunFailed(TransitionFailure.ExecutionFailed)),
            ),
            Row(
                "an interruption beside a measurement is told by that measurement",
                presentation(
                    routing = routed,
                    measurement = measurement(),
                    applicability = MeasurementApplicability.Changed,
                    lastAttempt = DiagnosticAttempt(2, RunOutcome.Interrupted, TransitionFailure.ContextChanged),
                ),
                Situation.Measured(EvidenceConclusion.NoObservedLeak, Staleness.Changed),
            ),
            Row(
                "an interruption with nothing to fall back on is its own state",
                presentation(
                    routing = routed,
                    lastAttempt = DiagnosticAttempt(2, RunOutcome.Interrupted, TransitionFailure.ContextChanged),
                ),
                Situation.CouldNotCheck(CouldNotCheckCause.Interrupted(TransitionFailure.ContextChanged)),
            ),
            Row(
                "a blocked latest attempt is never worded as a failure",
                presentation(
                    routing = routed,
                    measurement = measurement(),
                    lastAttempt = DiagnosticAttempt(2, RunOutcome.NotStarted, eligibility = DiagnosticEligibility.VpnOff),
                ),
                Situation.Measured(EvidenceConclusion.NoObservedLeak, Staleness.Current),
            ),
            Row(
                "cold start with the VPN on, once the suite has measured",
                presentation(routing = routed, measurement = measurement()),
                Situation.Measured(EvidenceConclusion.NoObservedLeak, Staleness.Current),
            ),
            Row(
                "a measurement taken under conditions that have since changed",
                presentation(routing = routed, measurement = measurement(), applicability = MeasurementApplicability.Changed),
                Situation.Measured(EvidenceConclusion.NoObservedLeak, Staleness.Changed),
            ),
            Row(
                "a measurement that attributed nothing proves nothing",
                presentation(routing = routed, measurement = measurement(CheckOutcome.NothingToLeak)),
                Situation.Measured(EvidenceConclusion.Insufficient, Staleness.Current),
            ),
        )

    @Test
    fun `a background re-read is silent within its grace and speaks once it elapses`() =
        runTest {
            val verifying = checking(RoutingKnowledge.Verifying(SelfRouting.Routed, ReadReason.Background, since = 0), measurement())
            val emitted = situationFlow(flowOf(verifying), clock = { currentTime }).toList()
            assertEquals(
                listOf(
                    Situation.Measured(EvidenceConclusion.NoObservedLeak, Staleness.Confirming(ReadReason.Background, 0)),
                    Situation.Checking(CheckingWhat.VpnState, SelfRouting.Routed, ReadReason.Background, 0),
                ),
                emitted,
            )
            assertEquals(2_000L, currentTime)
        }

    @Test
    fun `an explicit re-read is the state immediately and is never re-emitted`() =
        runTest {
            val verifying = checking(RoutingKnowledge.Verifying(SelfRouting.Routed, ReadReason.Explicit, since = 0), measurement())
            val emitted = situationFlow(flowOf(verifying), clock = { currentTime }).toList()
            assertEquals(listOf(Situation.Checking(CheckingWhat.VpnState, SelfRouting.Routed, ReadReason.Explicit, 0)), emitted)
            assertEquals(0L, currentTime)
        }

    @Test
    fun `a newer presentation cancels the pending grace re-emission`() =
        runTest {
            val verifying = checking(RoutingKnowledge.Verifying(SelfRouting.Routed, ReadReason.Background, since = 0), measurement())
            val settled = presentation(routing = routed, measurement = measurement())
            val source =
                flow {
                    emit(verifying)
                    delay(500)
                    emit(settled)
                }
            val emitted = situationFlow(source, clock = { currentTime }).toList()
            assertEquals(
                listOf(
                    Situation.Measured(EvidenceConclusion.NoObservedLeak, Staleness.Confirming(ReadReason.Background, 0)),
                    Situation.Measured(EvidenceConclusion.NoObservedLeak, Staleness.Current),
                ),
                emitted,
            )
            assertEquals(500L, currentTime)
        }

    private val routed = RoutingKnowledge.Known(SelfRouting.Routed, 5)
    private val off = RoutingKnowledge.Known(SelfRouting.VpnOff, 5)
    private val plan = NATIVE_CHECKS.map { ProbePlanEntry(it.id) }
    private val context = MeasurementContext("pid:1;uid:10", "self=x", "vpn=tun0;self=ROUTED", "backend=Kmod", 0, 1, 10)

    private fun measurement(outcome: CheckOutcome = CheckOutcome.HiddenByBackend): DiagnosticMeasurement =
        DiagnosticMeasurement(1, context, plan, plan.associate { it.id to outcome }, completed = true, interrupted = false, endedAt = 20)

    /** A routing re-read in flight: the eligibility that goes with it, and a measurement it cannot confirm. */
    private fun checking(
        routing: RoutingKnowledge,
        measurement: DiagnosticMeasurement?,
    ): DiagnosticPresentation =
        presentation(
            DiagnosticEligibility.Checking,
            routing,
            measurement,
            if (measurement == null) MeasurementApplicability.Absent else MeasurementApplicability.Unverified,
        )

    private fun presentation(
        eligibility: DiagnosticEligibility = DiagnosticEligibility.Eligible,
        routing: RoutingKnowledge = routed,
        measurement: DiagnosticMeasurement? = null,
        applicability: MeasurementApplicability =
            if (measurement == null) MeasurementApplicability.Absent else MeasurementApplicability.MatchesLastObservation,
        lastAttempt: DiagnosticAttempt? = measurement?.let { DiagnosticAttempt(it.runId, RunOutcome.Completed, measurement = it) },
        activeRunId: Long? = null,
        activeRunAutomatic: Boolean? = null,
        probeUnavailable: Boolean = false,
        confirmationPending: Boolean = false,
    ): DiagnosticPresentation =
        DiagnosticPresentation(
            eligibility = eligibility,
            routing = routing,
            activeRunId = activeRunId,
            activeRunAutomatic = activeRunAutomatic,
            activeStage = activeRunId?.let { DiagnosticStage.Core },
            activeResults = null,
            lastAttempt = lastAttempt,
            measurement = measurement,
            measurementResults = null,
            applicability = applicability,
            evidence = measurement?.let(::summarizeMeasurement),
            currentSuccess = false,
            probeUnavailable = probeUnavailable,
            confirmationPending = confirmationPending,
        )

    companion object {
        /** Well past every `since` in the table, so a grace boundary is expressed as `NOW - n`. */
        const val NOW = 10_000L
    }
}
