package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.ActiveDiagnosticRun
import dev.okhsunrog.vpnhide.diagnostics.CheckOutcome
import dev.okhsunrog.vpnhide.diagnostics.CheckResult
import dev.okhsunrog.vpnhide.diagnostics.CheckResults
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticAttempt
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticContextObservation
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticEligibility
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticMeasurement
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticRunState
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticRunView
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticStage
import dev.okhsunrog.vpnhide.diagnostics.EvidenceConclusion
import dev.okhsunrog.vpnhide.diagnostics.MeasurementApplicability
import dev.okhsunrog.vpnhide.diagnostics.MeasurementContext
import dev.okhsunrog.vpnhide.diagnostics.NATIVE_CHECKS
import dev.okhsunrog.vpnhide.diagnostics.NotMeasuredReason
import dev.okhsunrog.vpnhide.diagnostics.ProbePlanEntry
import dev.okhsunrog.vpnhide.diagnostics.RoutingKnowledge
import dev.okhsunrog.vpnhide.diagnostics.RunOutcome
import dev.okhsunrog.vpnhide.diagnostics.SelfRouting
import dev.okhsunrog.vpnhide.diagnostics.diagnosticPresentation
import dev.okhsunrog.vpnhide.diagnostics.diagnosticRequest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class DiagnosticPresentationDataTest {
    private val context = MeasurementContext("pid:1;uid:10", "self=x", "vpn=tun0;self=ROUTED", "backend=Kmod", 0, 1, 10)
    private val routed = RoutingKnowledge.Known(SelfRouting.Routed, 10)
    private val verifying = RoutingKnowledge.Verifying(SelfRouting.Routed, ReadReason.Transition, 5)

    @Test
    fun `a completed applicable measurement under eligible conditions is the only current success`() {
        val view = viewWith(completed(context))
        val presentation = diagnosticPresentation(view, eligible(context), changeEpoch = 0, routing = routed)
        assertTrue(presentation.currentSuccess)
        assertEquals(MeasurementApplicability.MatchesLastObservation, presentation.applicability)
        assertEquals(EvidenceConclusion.NoObservedLeak, presentation.evidence?.conclusion)
        assertEquals(DiagnosticEligibility.Eligible, presentation.eligibility)
        assertEquals(routed, presentation.routing)
        assertNull(presentation.activeRunId)
        assertEquals(view.attemptResults[1L], presentation.measurementResults)
    }

    @Test
    fun `an invalidated routing observation makes the measurement unverified until reobserved`() {
        val view = viewWith(completed(context))
        val unverified = diagnosticPresentation(view, eligible(context), changeEpoch = 0, routing = verifying)
        assertEquals(MeasurementApplicability.Unverified, unverified.applicability)
        assertFalse(unverified.currentSuccess)
        assertEquals(1L, unverified.measurement?.runId)
        val restored = diagnosticPresentation(view, eligible(context), changeEpoch = 0, routing = routed)
        assertEquals(MeasurementApplicability.MatchesLastObservation, restored.applicability)
        assertTrue(restored.currentSuccess)
    }

    /**
     * The point of stage 2: the eligibility of a re-read is Checking either way,
     * but the fact the process last had survives it, together with why the read runs.
     */
    @Test
    fun `a re-read keeps the last known routing fact and its reason beside the checking verdict`() {
        val view = viewWith(completed(context))
        val read = RoutingKnowledge.Verifying(SelfRouting.VpnOff, ReadReason.Explicit, since = 5)
        val presentation = diagnosticPresentation(view, null, 0, routing = read)
        assertEquals(DiagnosticEligibility.Checking, presentation.eligibility)
        assertEquals(read, presentation.routing)
        assertEquals(SelfRouting.VpnOff, (presentation.routing as RoutingKnowledge.Verifying).lastKnown)
        assertEquals(ReadReason.Explicit, presentation.routing.reason)
        assertEquals(MeasurementApplicability.Unverified, presentation.applicability)
        assertFalse(presentation.currentSuccess)
    }

    @Test
    fun `an unknown routing read carries its cause and keeps history as history`() {
        val view = viewWith(completed(context))
        val failed = RoutingKnowledge.Unknown(TransitionFailure.ReadFailed, SelfRouting.Routed)
        val observation = DiagnosticContextObservation(DiagnosticEligibility.Unknown, context = null)
        val presentation = diagnosticPresentation(view, observation, 0, routing = failed)
        assertEquals(DiagnosticEligibility.Unknown, presentation.eligibility)
        assertEquals(failed, presentation.routing)
        assertEquals(MeasurementApplicability.Unverified, presentation.applicability)
        assertFalse(presentation.currentSuccess)
        assertEquals(1L, presentation.measurement?.runId)
    }

    @Test
    fun `a known routing fact is what makes a measurement verifiable`() {
        val view = viewWith(completed(context))
        val off = RoutingKnowledge.Known(SelfRouting.VpnOff, observedAt = 12)
        val vpnOff = DiagnosticContextObservation(DiagnosticEligibility.VpnOff, context.copy(routing = "vpn=;self=VPN_OFF"))
        val presentation = diagnosticPresentation(view, vpnOff, 0, routing = off)
        // Known, so the measurement is compared rather than suspended: the routing identity moved.
        assertEquals(MeasurementApplicability.Changed, presentation.applicability)
        assertEquals(off, presentation.routing)
    }

    @Test
    fun `a known change or a different routing identity makes the measurement changed with history visible`() {
        val view = viewWith(completed(context))
        val epoch = diagnosticPresentation(view, eligible(context.copy(changeEpoch = 1)), changeEpoch = 1, routing = routed)
        assertEquals(MeasurementApplicability.Changed, epoch.applicability)
        assertFalse(epoch.currentSuccess)
        assertEquals(1L, epoch.measurement?.runId)
        val vpn = eligible(context.copy(routing = "vpn=tun1;self=ROUTED"))
        assertEquals(MeasurementApplicability.Changed, diagnosticPresentation(view, vpn, 0, routed).applicability)
    }

    @Test
    fun `a failed later attempt stays visible beside the last complete measurement`() {
        val attempt = DiagnosticAttempt(2, RunOutcome.Failed, TransitionFailure.ExecutionFailed)
        val view =
            viewWith(completed(context)).let {
                it.copy(core = it.core.copy(lastAttempt = attempt), attemptResults = it.attemptResults + (2L to results(CheckOutcome.Leak)))
            }
        val presentation = diagnosticPresentation(view, eligible(context), changeEpoch = 0, routing = routed)
        assertEquals(RunOutcome.Failed, presentation.lastAttempt?.outcome)
        assertEquals(1L, presentation.measurement?.runId)
        // The measurement's own evidence is what stays listed, never the failed attempt's.
        assertEquals(view.attemptResults[1L], presentation.measurementResults)
        assertTrue(presentation.currentSuccess)
    }

    @Test
    fun `blocked or unknown current conditions never present success and carry no context`() {
        val view = viewWith(completed(context))
        val vpnOff = DiagnosticContextObservation(DiagnosticEligibility.VpnOff, context.copy(routing = "vpn=;self=VPN_OFF"))
        val off = diagnosticPresentation(view, vpnOff, 0, RoutingKnowledge.Known(SelfRouting.VpnOff, 12))
        assertEquals(DiagnosticEligibility.VpnOff, off.eligibility)
        assertEquals(MeasurementApplicability.Changed, off.applicability)
        assertFalse(off.currentSuccess)
        val checking = diagnosticPresentation(view, null, 0, verifying)
        assertEquals(DiagnosticEligibility.Checking, checking.eligibility)
        assertEquals(MeasurementApplicability.Unverified, checking.applicability)
        assertFalse(checking.currentSuccess)
    }

    @Test
    fun `all unmeasured evidence is insufficient even when applicable and eligible`() {
        val measurement = completed(context, outcome = CheckOutcome.NotMeasured(NotMeasuredReason.NoGroundTruth))
        val presentation = diagnosticPresentation(viewWith(measurement), eligible(context), 0, routed)
        assertEquals(EvidenceConclusion.Insufficient, presentation.evidence?.conclusion)
        assertFalse(presentation.currentSuccess)
        assertEquals(MeasurementApplicability.MatchesLastObservation, presentation.applicability)
    }

    @Test
    fun `an active run is exposed with its stage and partial evidence without touching history`() {
        val view = viewWith(completed(context))
        val active = ActiveDiagnosticRun(3, diagnosticRequest(), stage = DiagnosticStage.Slow)
        val running = view.copy(core = view.core.copy(active = active), activeResults = results(CheckOutcome.HiddenByBackend))
        val presentation = diagnosticPresentation(running, eligible(context), 0, routed)
        assertEquals(3L, presentation.activeRunId)
        assertEquals(DiagnosticStage.Slow, presentation.activeStage)
        assertEquals(1L, presentation.measurement?.runId)
        assertTrue(presentation.currentSuccess)
    }

    @Test
    fun `a quarantined probe resource is exposed without touching the retained measurement`() {
        val view = viewWith(completed(context))
        assertFalse(diagnosticPresentation(view, eligible(context), 0, routed).probeUnavailable)
        val quarantined = view.copy(core = view.core.copy(quarantined = true))
        val presentation = diagnosticPresentation(quarantined, eligible(context), 0, routed)
        assertTrue(presentation.probeUnavailable)
        assertEquals(1L, presentation.measurement?.runId)
        assertEquals(MeasurementApplicability.MatchesLastObservation, presentation.applicability)
    }

    private fun eligible(context: MeasurementContext) = DiagnosticContextObservation(DiagnosticEligibility.Eligible, context)

    private fun plan() = NATIVE_CHECKS.map { ProbePlanEntry(it.id) }

    private fun results(outcome: CheckOutcome) = CheckResults(native = NATIVE_CHECKS.map { CheckResult(it.id, "", outcome, id = it.id) })

    private fun completed(
        context: MeasurementContext,
        outcome: CheckOutcome = CheckOutcome.HiddenByBackend,
    ) = DiagnosticMeasurement(
        1,
        context,
        plan(),
        plan().associate { it.id to outcome },
        completed = true,
        interrupted = false,
        endedAt = 20,
    )

    private fun viewWith(measurement: DiagnosticMeasurement): DiagnosticRunView =
        DiagnosticRunView(
            core =
                DiagnosticRunState(
                    lastAttempt = DiagnosticAttempt(measurement.runId, RunOutcome.Completed, measurement = measurement),
                    lastComplete = measurement,
                    nextId = measurement.runId + 1,
                ),
            attemptResults = mapOf(measurement.runId to results(measurement.outcomes.values.first())),
        )
}
