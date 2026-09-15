package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.ActiveDiagnosticRun
import dev.okhsunrog.vpnhide.diagnostics.CheckOutcome
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticAttempt
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticContextObservation
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticEligibility
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticMeasurement
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticRunState
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticRunView
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticStage
import dev.okhsunrog.vpnhide.diagnostics.MeasurementApplicability
import dev.okhsunrog.vpnhide.diagnostics.MeasurementContext
import dev.okhsunrog.vpnhide.diagnostics.NATIVE_CHECKS
import dev.okhsunrog.vpnhide.diagnostics.ProbePlanEntry
import dev.okhsunrog.vpnhide.diagnostics.RunOutcome
import dev.okhsunrog.vpnhide.diagnostics.diagnosticPresentation
import dev.okhsunrog.vpnhide.diagnostics.diagnosticRequest
import dev.okhsunrog.vpnhide.diagnostics.diagnosticSummary
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class DiagnosticSummaryDataTest {
    private val context = MeasurementContext("pid:1;uid:10", "self=x", "vpn=tun0;self=ROUTED", "backend=Kmod", 0, 7, 10)

    @Test
    fun `the summary carries the presentation's decisions and identities without the results`() {
        val measurement = completed(context)
        val attempt = DiagnosticAttempt(2, RunOutcome.Failed, TransitionFailure.ExecutionFailed)
        val active = ActiveDiagnosticRun(3, diagnosticRequest(), stage = DiagnosticStage.Slow)
        val view = viewWith(measurement).let { it.copy(core = it.core.copy(lastAttempt = attempt, active = active)) }
        val presentation = diagnosticPresentation(view, eligible(context), changeEpoch = 0, uncertain = false)

        val summary = diagnosticSummary(presentation)

        assertEquals(presentation.eligibility, summary.eligibility)
        assertEquals(3L, summary.activeRunId)
        assertEquals(DiagnosticStage.Slow, summary.activeStage)
        assertEquals(2L, summary.lastAttempt?.runId)
        assertEquals(RunOutcome.Failed, summary.lastAttempt?.outcome)
        assertEquals(TransitionFailure.ExecutionFailed, summary.lastAttempt?.failure)
        assertNull(summary.lastAttempt?.eligibility)
        assertEquals(1L, summary.measurement?.runId)
        assertEquals(10L, summary.measurement?.startedAt)
        assertEquals(20L, summary.measurement?.endedAt)
        assertEquals(7L, summary.measurement?.observationId)
        assertTrue(summary.measurement?.completed == true)
        assertFalse(summary.measurement?.interrupted == true)
        assertEquals(presentation.applicability, summary.applicability)
        assertEquals(presentation.evidence, summary.evidence)
        assertEquals(presentation.currentSuccess, summary.currentSuccess)
    }

    @Test
    fun `a blocked attempt keeps its blocking eligibility and an empty history stays empty`() {
        val blocked = DiagnosticAttempt(1, RunOutcome.NotStarted, eligibility = DiagnosticEligibility.VpnOff)
        val view = DiagnosticRunView(core = DiagnosticRunState(lastAttempt = blocked, nextId = 2))
        val vpnOff = DiagnosticContextObservation(DiagnosticEligibility.VpnOff, context.copy(routing = "vpn=;self=VPN_OFF"))

        val summary = diagnosticSummary(diagnosticPresentation(view, vpnOff, changeEpoch = 0, uncertain = false))

        assertEquals(DiagnosticEligibility.VpnOff, summary.eligibility)
        assertEquals(DiagnosticEligibility.VpnOff, summary.lastAttempt?.eligibility)
        assertEquals(RunOutcome.NotStarted, summary.lastAttempt?.outcome)
        assertNull(summary.activeRunId)
        assertNull(summary.activeStage)
        assertNull(summary.measurement)
        assertNull(summary.evidence)
        assertEquals(MeasurementApplicability.Absent, summary.applicability)
        assertFalse(summary.currentSuccess)
    }

    private fun eligible(context: MeasurementContext) = DiagnosticContextObservation(DiagnosticEligibility.Eligible, context)

    private fun plan() = NATIVE_CHECKS.map { ProbePlanEntry(it.id) }

    private fun completed(context: MeasurementContext) =
        DiagnosticMeasurement(
            1,
            context,
            plan(),
            plan().associate { it.id to CheckOutcome.HiddenByBackend },
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
        )
}
