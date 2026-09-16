package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.CheckResults
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticAttempt
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticEligibility
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticGate
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticMeasurement
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticPresentation
import dev.okhsunrog.vpnhide.diagnostics.LayerStatus
import dev.okhsunrog.vpnhide.diagnostics.MeasurementApplicability
import dev.okhsunrog.vpnhide.diagnostics.MeasurementContext
import dev.okhsunrog.vpnhide.diagnostics.MeasurementCoverage
import dev.okhsunrog.vpnhide.diagnostics.ProbePlanEntry
import dev.okhsunrog.vpnhide.diagnostics.RunOutcome
import dev.okhsunrog.vpnhide.diagnostics.reportGate
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ProtectionVerdictTest {
    private val noBackend = DisplayNativeBackend(null, ModuleState.NotInstalled)
    private val measuredLayers = MeasurementCoverage(noBackend, emptySet(), lsposedActive = true)
    private val liveLayers = MeasurementCoverage(noBackend, emptySet(), lsposedActive = false)
    private val results = CheckResults(native = emptyList())

    @Test
    fun `a complete measurement is rendered against the layers it was measured with`() {
        val verdict = protectionVerdict(presentation(measurement = measurement(measuredLayers)), liveLayers)

        // LSPosed was active at measurement time and is not now: the tile keeps the
        // measured attribution, the live layers never re-attribute retained evidence.
        assertEquals(ProtectionCheck.Checked(LayerStatus.Absent, LayerStatus.Active(hidden = 0, leaks = 0)), verdict.check)
        assertEquals(DiagnosticGate.ROUTED, verdict.report?.gate)
    }

    @Test
    fun `a measurement without layers of its own is attributed with the live ones`() {
        val verdict = protectionVerdict(presentation(measurement = measurement(layers = null)), liveLayers)

        assertEquals(ProtectionCheck.Checked(LayerStatus.Absent, LayerStatus.Inactive), verdict.check)
    }

    @Test
    fun `a later attempt that did not complete leaves the measured tiles in place`() {
        val failed = DiagnosticAttempt(2, RunOutcome.Failed, TransitionFailure.ExecutionFailed)
        val verdict = protectionVerdict(presentation(measurement = measurement(measuredLayers), lastAttempt = failed), liveLayers)

        assertEquals(ProtectionCheck.Checked(LayerStatus.Absent, LayerStatus.Active(hidden = 0, leaks = 0)), verdict.check)
    }

    @Test
    fun `a blocked latest attempt without a measurement is the gate`() {
        val blocked = DiagnosticAttempt(1, RunOutcome.NotStarted, eligibility = DiagnosticEligibility.VpnOff)
        val verdict = protectionVerdict(presentation(lastAttempt = blocked), liveLayers)

        assertEquals(ProtectionCheck.Blocked(DiagnosticGate.VPN_OFF), verdict.check)
        assertNull(verdict.report)
    }

    @Test
    fun `anything else without a measurement could not measure`() {
        val noGate = DiagnosticAttempt(1, RunOutcome.NotStarted, eligibility = DiagnosticEligibility.Applying)
        val interrupted = DiagnosticAttempt(1, RunOutcome.Interrupted, TransitionFailure.ContextChanged)

        assertEquals(ProtectionCheck.Failed, protectionVerdict(presentation(), liveLayers).check)
        assertEquals(ProtectionCheck.Failed, protectionVerdict(presentation(lastAttempt = noGate), liveLayers).check)
        assertEquals(ProtectionCheck.Failed, protectionVerdict(presentation(lastAttempt = interrupted), liveLayers).check)
        assertNull(presentation(lastAttempt = interrupted).reportGate())
    }

    @Test
    fun `a measurement whose evidence is gone is not routed`() {
        val presentation = presentation(measurement = measurement(measuredLayers), measurementResults = null)

        assertNull(presentation.reportGate())
        assertEquals(ProtectionCheck.Failed, protectionVerdict(presentation, liveLayers).check)
    }

    private fun measurement(layers: MeasurementCoverage?): DiagnosticMeasurement {
        val plan = listOf(ProbePlanEntry("ioctl_flags"))
        val context = MeasurementContext("pid:1;uid:10", "self", "vpn=tun0;self=ROUTED", "backend=none", 0, 1, 10, coverageLayers = layers)
        return DiagnosticMeasurement(1, context, plan, emptyMap(), completed = true, interrupted = false, endedAt = 20)
    }

    private fun presentation(
        measurement: DiagnosticMeasurement? = null,
        measurementResults: CheckResults? = measurement?.let { results },
        lastAttempt: DiagnosticAttempt? = measurement?.let { DiagnosticAttempt(it.runId, RunOutcome.Completed, measurement = it) },
    ) = DiagnosticPresentation(
        eligibility = DiagnosticEligibility.Eligible,
        activeRunId = null,
        activeStage = null,
        activeResults = null,
        lastAttempt = lastAttempt,
        lastAttemptResults = null,
        measurement = measurement,
        measurementResults = measurementResults,
        applicability = if (measurement == null) MeasurementApplicability.Absent else MeasurementApplicability.MatchesLastObservation,
        evidence = null,
        currentSuccess = false,
        probeUnavailable = false,
    )
}
