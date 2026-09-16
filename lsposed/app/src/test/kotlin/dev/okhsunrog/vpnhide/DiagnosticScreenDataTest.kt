package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.ActionNeededKind
import dev.okhsunrog.vpnhide.diagnostics.CheckOutcome
import dev.okhsunrog.vpnhide.diagnostics.CheckResult
import dev.okhsunrog.vpnhide.diagnostics.CheckResults
import dev.okhsunrog.vpnhide.diagnostics.CheckingWhat
import dev.okhsunrog.vpnhide.diagnostics.CouldNotCheckCause
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticAttempt
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticAttemptNotice
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticBanner
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticEligibility
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticMeasurement
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticPresentation
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticScreenDecision
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticStage
import dev.okhsunrog.vpnhide.diagnostics.EvidenceConclusion
import dev.okhsunrog.vpnhide.diagnostics.MeasurementApplicability
import dev.okhsunrog.vpnhide.diagnostics.MeasurementContext
import dev.okhsunrog.vpnhide.diagnostics.MeasurementCoverage
import dev.okhsunrog.vpnhide.diagnostics.NATIVE_CHECKS
import dev.okhsunrog.vpnhide.diagnostics.NotMeasuredReason
import dev.okhsunrog.vpnhide.diagnostics.ProbePlanEntry
import dev.okhsunrog.vpnhide.diagnostics.RoutingKnowledge
import dev.okhsunrog.vpnhide.diagnostics.RunOutcome
import dev.okhsunrog.vpnhide.diagnostics.SelfRouting
import dev.okhsunrog.vpnhide.diagnostics.Situation
import dev.okhsunrog.vpnhide.diagnostics.Staleness
import dev.okhsunrog.vpnhide.diagnostics.diagnosticScreenDecision
import dev.okhsunrog.vpnhide.diagnostics.summarizeMeasurement
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * The banner is a map over [Situation] — the same classification the Dashboard hero
 * renders — and the presentation only supplies this screen's side channels. Every row
 * asserts the whole decision, so a results list, a completeness flag or a coverage
 * that quietly changes is a failure.
 */
class DiagnosticScreenDataTest {
    @Test
    fun `initialization and a suite in flight show progress and any partial evidence`() {
        assertEquals(progress(), decide(Situation.Initializing, history))
        // A run with nothing to show yet, and a draining one that has nothing left to add.
        assertEquals(progress(), decide(checkingSuite(), blank))
        val draining = blank.copy(activeRunId = 2, activeStage = DiagnosticStage.Draining, activeResults = partial)
        assertEquals(progress(), decide(checkingSuite(), draining))
        // Partial evidence is listed as incomplete and attributed with the live layers, not the measurement's.
        val running = history.copy(activeRunId = 2, activeStage = DiagnosticStage.Slow, activeResults = partial)
        assertEquals(
            DiagnosticScreenDecision(DiagnosticBanner.Progress, results = partial, complete = false),
            decide(checkingSuite(), running),
        )
    }

    @Test
    fun `a routing re-read keeps the last known condition with a busy button`() {
        assertEquals(
            DiagnosticScreenDecision(DiagnosticBanner.VpnOff, checking = true),
            decide(checkingVpn(SelfRouting.VpnOff), history),
        )
        assertEquals(
            DiagnosticScreenDecision(DiagnosticBanner.SelfExcluded, checking = true),
            decide(checkingVpn(SelfRouting.Excluded), history),
        )
        // Routed, or nothing known at all: the history stays listed as unverified, or there is nothing to list.
        assertEquals(listing(DiagnosticBanner.ResultsUnverified), decide(checkingVpn(SelfRouting.Routed), history))
        assertEquals(progress(), decide(checkingVpn(null), blank))
    }

    @Test
    fun `a configuration change being applied keeps the history listed`() {
        val applying = Situation.Checking(CheckingWhat.ConfigApplying, SelfRouting.Routed, ReadReason.Explicit, since = 0)
        assertEquals(listing(DiagnosticBanner.Applying), decide(applying, history))
        assertEquals(DiagnosticScreenDecision(DiagnosticBanner.Applying), decide(applying, blank))
    }

    @Test
    fun `a known condition replaces the list, a failed application keeps it`() {
        assertEquals(DiagnosticScreenDecision(DiagnosticBanner.VpnOff), decide(Situation.VpnOff, history))
        assertEquals(DiagnosticScreenDecision(DiagnosticBanner.SelfExcluded), decide(Situation.NotMeasurable, history))
        for (kind in listOf(ActionNeededKind.RestartApp, ActionNeededKind.RestartDevice)) {
            assertEquals("$kind", DiagnosticScreenDecision(DiagnosticBanner.RestartApp), decide(Situation.ActionNeeded(kind), history))
        }
        assertEquals(
            listing(DiagnosticBanner.ApplicationUnknown),
            decide(Situation.ActionNeeded(ActionNeededKind.ApplicationUnknown), history),
        )
        assertEquals(
            listing(DiagnosticBanner.ApplicationFailed),
            decide(Situation.ActionNeeded(ActionNeededKind.ApplicationFailed), history),
        )
    }

    @Test
    fun `a check that could not run keeps the history and never repeats itself in a notice`() {
        val routing = Situation.CouldNotCheck(CouldNotCheckCause.RoutingUnknown(TransitionFailure.ReadFailed))
        assertEquals(listing(DiagnosticBanner.RoutingUnknown), decide(routing, history))
        val quarantined = Situation.CouldNotCheck(CouldNotCheckCause.ProbeUnavailable)
        assertEquals(listing(DiagnosticBanner.ProbeUnavailable), decide(quarantined, history))
        // The retry prompt sits above the history it could not refresh; the banner already says it failed.
        val failed = Situation.CouldNotCheck(CouldNotCheckCause.RunFailed(TransitionFailure.ExecutionFailed))
        val failedAttempt = DiagnosticAttempt(3, RunOutcome.Failed, TransitionFailure.ExecutionFailed)
        assertEquals(listing(DiagnosticBanner.Failed), decide(failed, history.copy(lastAttempt = failedAttempt)))
        assertEquals(DiagnosticScreenDecision(DiagnosticBanner.Failed), decide(failed, blank.copy(lastAttempt = failedAttempt)))
        // An interruption with an earlier measurement is never this situation, so it never lists one.
        val interrupted = Situation.CouldNotCheck(CouldNotCheckCause.Interrupted(TransitionFailure.ContextChanged))
        assertEquals(DiagnosticScreenDecision(DiagnosticBanner.Interrupted), decide(interrupted, blank))
    }

    @Test
    fun `a measurement is worded by its evidence, then by its staleness`() {
        assertEquals(listing(DiagnosticBanner.Ready), decide(measured(staleness = Staleness.Current), history))
        // A re-read inside its grace is silent: Confirming renders exactly like Current.
        val confirming = Staleness.Confirming(ReadReason.Background, since = null)
        assertEquals(listing(DiagnosticBanner.Ready), decide(measured(staleness = confirming), history))
        assertEquals(listing(DiagnosticBanner.ResultsChanged), decide(measured(staleness = Staleness.Changed), history))
        val insufficient = base(measurement = measurement(CheckOutcome.NotMeasured(NotMeasuredReason.NoGroundTruth)))
        assertEquals(
            DiagnosticScreenDecision(
                DiagnosticBanner.InsufficientEvidence,
                results = insufficient.measurementResults,
                coverage = insufficient.measurement?.context?.coverageLayers,
            ),
            decide(measured(EvidenceConclusion.Insufficient, Staleness.Changed), insufficient),
        )
    }

    @Test
    fun `an attempt that did not complete is a notice beside the history`() {
        val interrupted = DiagnosticAttempt(3, RunOutcome.Interrupted, TransitionFailure.ContextChanged)
        assertEquals(
            listing(DiagnosticBanner.Ready, DiagnosticAttemptNotice.Interrupted),
            decide(measured(), history.copy(lastAttempt = interrupted)),
        )
        // A run that never started because of a condition is not a failed check (I13).
        val blocked = DiagnosticAttempt(3, RunOutcome.NotStarted, eligibility = DiagnosticEligibility.VpnOff)
        assertEquals(listing(DiagnosticBanner.Ready), decide(measured(), history.copy(lastAttempt = blocked)))
    }

    private val history = base(measurement = measurement())
    private val blank = base()
    private val partial = results(CheckOutcome.HiddenBySelinux)

    private fun decide(
        situation: Situation,
        presentation: DiagnosticPresentation,
    ) = diagnosticScreenDecision(situation, presentation)

    private fun progress() = DiagnosticScreenDecision(DiagnosticBanner.Progress)

    /** The expected decision for a banner that keeps [history]'s measurement listed with its own coverage. */
    private fun listing(
        banner: DiagnosticBanner,
        notice: DiagnosticAttemptNotice? = null,
    ) = DiagnosticScreenDecision(banner, notice, history.measurementResults, coverage = history.measurement?.context?.coverageLayers)

    private fun checkingSuite() = Situation.Checking(CheckingWhat.Suite, SelfRouting.Routed, ReadReason.Explicit, since = 0)

    private fun checkingVpn(lastKnown: SelfRouting?) = Situation.Checking(CheckingWhat.VpnState, lastKnown, ReadReason.Explicit, since = 0)

    private fun measured(
        evidence: EvidenceConclusion = EvidenceConclusion.NoObservedLeak,
        staleness: Staleness = Staleness.Current,
    ) = Situation.Measured(evidence, staleness)

    private fun results(outcome: CheckOutcome) = CheckResults(native = NATIVE_CHECKS.map { CheckResult(it.id, "", outcome, id = it.id) })

    private fun measurement(outcome: CheckOutcome = CheckOutcome.HiddenByBackend): DiagnosticMeasurement {
        val plan = NATIVE_CHECKS.map { ProbePlanEntry(it.id) }
        val layers =
            MeasurementCoverage(DisplayNativeBackend(NativeBackendId.Kmod, ModuleState.NotInstalled), emptySet(), lsposedActive = true)
        val context = MeasurementContext("pid:1;uid:10", "self", "vpn=tun0;self=ROUTED", "backend=Kmod", 0, 1, 10, coverageLayers = layers)
        return DiagnosticMeasurement(
            1,
            context,
            plan,
            plan.associate { it.id to outcome },
            completed = true,
            interrupted = false,
            endedAt = 20,
        )
    }

    private fun base(measurement: DiagnosticMeasurement? = null): DiagnosticPresentation =
        DiagnosticPresentation(
            eligibility = DiagnosticEligibility.Eligible,
            routing = RoutingKnowledge.Known(SelfRouting.Routed, 0),
            activeRunId = null,
            activeRunAutomatic = null,
            activeStage = null,
            activeResults = null,
            lastAttempt = measurement?.let { DiagnosticAttempt(it.runId, RunOutcome.Completed, measurement = it) },
            measurement = measurement,
            measurementResults = measurement?.let { results(it.outcomes.values.first()) },
            applicability = if (measurement == null) MeasurementApplicability.Absent else MeasurementApplicability.MatchesLastObservation,
            evidence = measurement?.let(::summarizeMeasurement),
            currentSuccess = false,
            probeUnavailable = false,
        )
}
