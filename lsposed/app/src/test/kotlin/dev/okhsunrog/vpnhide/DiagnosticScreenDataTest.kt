package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.CheckOutcome
import dev.okhsunrog.vpnhide.diagnostics.CheckResult
import dev.okhsunrog.vpnhide.diagnostics.CheckResults
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticAttempt
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticAttemptNotice
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticBanner
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticEligibility
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticMeasurement
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticPresentation
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticStage
import dev.okhsunrog.vpnhide.diagnostics.MeasurementApplicability
import dev.okhsunrog.vpnhide.diagnostics.MeasurementContext
import dev.okhsunrog.vpnhide.diagnostics.NATIVE_CHECKS
import dev.okhsunrog.vpnhide.diagnostics.NotMeasuredReason
import dev.okhsunrog.vpnhide.diagnostics.ProbePlanEntry
import dev.okhsunrog.vpnhide.diagnostics.RunOutcome
import dev.okhsunrog.vpnhide.diagnostics.diagnosticScreenDecision
import dev.okhsunrog.vpnhide.diagnostics.summarizeMeasurement
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Test

class DiagnosticScreenDataTest {
    @Test
    fun `current conditions outrank history and never spin forever`() {
        val measured = base(measurement = measurement())
        assertEquals(DiagnosticBanner.RestartApp, decide(measured.copy(eligibility = DiagnosticEligibility.RestartApp)).banner)
        assertEquals(DiagnosticBanner.VpnOff, decide(measured.copy(eligibility = DiagnosticEligibility.VpnOff)).banner)
        assertEquals(DiagnosticBanner.SelfExcluded, decide(measured.copy(eligibility = DiagnosticEligibility.SelfExcluded)).banner)
        assertEquals(DiagnosticBanner.Applying, decide(measured.copy(eligibility = DiagnosticEligibility.Applying)).banner)
        assertEquals(
            DiagnosticBanner.ApplicationUnknown,
            decide(measured.copy(eligibility = DiagnosticEligibility.ApplicationUnknown)).banner,
        )
        assertEquals(
            DiagnosticBanner.ApplicationFailed,
            decide(measured.copy(eligibility = DiagnosticEligibility.ApplicationFailed)).banner,
        )
        assertEquals(DiagnosticBanner.RoutingUnknown, decide(measured.copy(eligibility = DiagnosticEligibility.Unknown)).banner)
        // The existing prompts replace the list; the explicit condition banners keep history visible (T17).
        assertNull(decide(measured.copy(eligibility = DiagnosticEligibility.VpnOff)).results)
        assertNull(decide(measured.copy(eligibility = DiagnosticEligibility.SelfExcluded)).results)
        assertNull(decide(measured.copy(eligibility = DiagnosticEligibility.RestartApp)).results)
        for (
        eligibility in
        listOf(
            DiagnosticEligibility.Applying,
            DiagnosticEligibility.ApplicationUnknown,
            DiagnosticEligibility.ApplicationFailed,
            DiagnosticEligibility.Unknown,
        )
        ) {
            assertEquals("$eligibility", measured.measurementResults, decide(measured.copy(eligibility = eligibility)).results)
            assertNull("$eligibility", decide(base().copy(eligibility = eligibility)).results)
        }
        // Re-checking routing keeps an existing measurement on screen as unverified.
        val checking = decide(measured.copy(eligibility = DiagnosticEligibility.Checking))
        assertEquals(DiagnosticBanner.ResultsUnverified, checking.banner)
        assertEquals(measured.measurementResults, checking.results)
        assertEquals(DiagnosticBanner.Progress, decide(base().copy(eligibility = DiagnosticEligibility.Checking)).banner)
    }

    @Test
    fun `a quarantined probe outranks history but not a current condition`() {
        val measured = base(measurement = measurement()).copy(probeUnavailable = true)
        val quarantined = decide(measured)
        assertEquals(DiagnosticBanner.ProbeUnavailable, quarantined.banner)
        // History stays listed: the measurement is as good as it was, only a new one cannot start.
        assertEquals(measured.measurementResults, quarantined.results)
        assertNull(quarantined.attemptNotice)
        // A condition that explains the same impossibility in more detail still wins.
        assertEquals(DiagnosticBanner.VpnOff, decide(measured.copy(eligibility = DiagnosticEligibility.VpnOff)).banner)
        assertEquals(DiagnosticBanner.ProbeUnavailable, decide(base().copy(probeUnavailable = true)).banner)
    }

    @Test
    fun `an active run shows progress and its partial evidence as incomplete`() {
        val core = decide(base().copy(activeRunId = 2, activeStage = DiagnosticStage.Core))
        assertEquals(DiagnosticBanner.Progress, core.banner)
        assertNull(core.results)
        val partial = results(CheckOutcome.HiddenByBackend)
        val slow =
            decide(base(measurement = measurement()).copy(activeRunId = 2, activeStage = DiagnosticStage.Slow, activeResults = partial))
        assertEquals(DiagnosticBanner.Progress, slow.banner)
        assertEquals(partial, slow.results)
        assertFalse(slow.complete)
    }

    @Test
    fun `a failed or interrupted latest attempt is a banner without history and a notice beside it`() {
        val failed = DiagnosticAttempt(3, RunOutcome.Failed, TransitionFailure.ExecutionFailed)
        val interrupted = DiagnosticAttempt(3, RunOutcome.Interrupted, TransitionFailure.ContextChanged)
        assertEquals(DiagnosticBanner.Failed, decide(base().copy(lastAttempt = failed)).banner)
        assertEquals(DiagnosticBanner.Interrupted, decide(base().copy(lastAttempt = interrupted)).banner)
        val notStarted = DiagnosticAttempt(3, RunOutcome.NotStarted, TransitionFailure.ReadFailed)
        assertEquals(DiagnosticBanner.Failed, decide(base().copy(lastAttempt = notStarted)).banner)

        val withHistory = decide(base(measurement = measurement()).copy(lastAttempt = interrupted))
        assertEquals(DiagnosticBanner.Ready, withHistory.banner)
        assertEquals(DiagnosticAttemptNotice.Interrupted, withHistory.attemptNotice)
        assertEquals(results(CheckOutcome.HiddenByBackend), withHistory.results)
    }

    @Test
    fun `sufficiency outranks applicability and applicability words the ready banner`() {
        val insufficient = base(measurement = measurement(CheckOutcome.NotMeasured(NotMeasuredReason.NoGroundTruth)))
        assertEquals(DiagnosticBanner.InsufficientEvidence, decide(insufficient).banner)
        assertEquals(
            DiagnosticBanner.InsufficientEvidence,
            decide(insufficient.copy(applicability = MeasurementApplicability.Changed)).banner,
        )
        val measured = base(measurement = measurement())
        assertEquals(DiagnosticBanner.Ready, decide(measured).banner)
        assertEquals(DiagnosticBanner.ResultsChanged, decide(measured.copy(applicability = MeasurementApplicability.Changed)).banner)
        assertEquals(
            DiagnosticBanner.ResultsUnverified,
            decide(measured.copy(applicability = MeasurementApplicability.Unverified)).banner,
        )
        assertEquals(DiagnosticBanner.Progress, decide(base()).banner)
    }

    private fun decide(presentation: DiagnosticPresentation) = diagnosticScreenDecision(presentation)

    private fun results(outcome: CheckOutcome) = CheckResults(native = NATIVE_CHECKS.map { CheckResult(it.id, "", outcome, id = it.id) })

    private fun measurement(outcome: CheckOutcome = CheckOutcome.HiddenByBackend): DiagnosticMeasurement {
        val plan = NATIVE_CHECKS.map { ProbePlanEntry(it.id) }
        val context = MeasurementContext("pid:1;uid:10", "self", "vpn=tun0;self=ROUTED", "backend=Kmod", 0, 1, 10)
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
            activeRunId = null,
            activeStage = null,
            activeResults = null,
            lastAttempt = measurement?.let { DiagnosticAttempt(it.runId, RunOutcome.Completed, measurement = it) },
            lastAttemptResults = measurement?.let { results(it.outcomes.values.first()) },
            measurement = measurement,
            measurementResults = measurement?.let { results(it.outcomes.values.first()) },
            applicability = if (measurement == null) MeasurementApplicability.Absent else MeasurementApplicability.MatchesLastObservation,
            evidence = measurement?.let(::summarizeMeasurement),
            currentSuccess = false,
            probeUnavailable = false,
        )
}
