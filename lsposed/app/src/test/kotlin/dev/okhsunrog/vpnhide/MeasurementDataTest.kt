package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.CheckOutcome
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticEligibility
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticMeasurement
import dev.okhsunrog.vpnhide.diagnostics.EvidenceConclusion
import dev.okhsunrog.vpnhide.diagnostics.MeasurementApplicability
import dev.okhsunrog.vpnhide.diagnostics.NotMeasuredReason
import dev.okhsunrog.vpnhide.diagnostics.ProbePlanEntry
import dev.okhsunrog.vpnhide.diagnostics.canPresentCurrentSuccess
import dev.okhsunrog.vpnhide.diagnostics.measurementApplicability
import dev.okhsunrog.vpnhide.diagnostics.summarizeMeasurement
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class MeasurementDataTest {
    @Test
    fun `all unmeasured and nothing-to-leak-only runs never prove hiding`() {
        val outcomes = listOf(CheckOutcome.NotMeasured(NotMeasuredReason.NoGroundTruth), CheckOutcome.NothingToLeak)
        for (outcome in outcomes) {
            val measurement = measurement(mapOf("owned" to outcome))
            assertEquals(EvidenceConclusion.Insufficient, summarizeMeasurement(measurement).conclusion)
            assertFalse(
                canPresentCurrentSuccess(measurement, MeasurementApplicability.MatchesLastObservation, DiagnosticEligibility.Eligible),
            )
        }
    }

    @Test
    fun `missing planned probe keeps denominator and blocks positive summary`() {
        val measurement =
            measurement(
                mapOf("owned" to CheckOutcome.HiddenByBackend),
            ).copy(plan = listOf(ProbePlanEntry("owned"), ProbePlanEntry("missing")))
        assertEquals(1, summarizeMeasurement(measurement).notRun)
        assertEquals(EvidenceConclusion.Partial, summarizeMeasurement(measurement).conclusion)
        assertFalse(canPresentCurrentSuccess(measurement, MeasurementApplicability.MatchesLastObservation, DiagnosticEligibility.Eligible))
    }

    @Test
    fun `uncovered leak remains visible without becoming an owned module failure`() {
        val measurement = measurement(mapOf("owned" to CheckOutcome.HiddenBySelinux, "uncovered" to CheckOutcome.Leak))
        val evidence = summarizeMeasurement(measurement)
        assertEquals(EvidenceConclusion.NoObservedLeak, evidence.conclusion)
        assertEquals(1, evidence.uncoveredLeaks)
        assertEquals(0, evidence.hidden)
        assertEquals(1, evidence.systemBlocked)
        assertTrue(canPresentCurrentSuccess(measurement, MeasurementApplicability.MatchesLastObservation, DiagnosticEligibility.Eligible))
        assertEquals(
            EvidenceConclusion.OwnedLeak,
            summarizeMeasurement(measurement.copy(outcomes = mapOf("owned" to CheckOutcome.Leak))).conclusion,
        )
    }

    @Test
    fun `known VPN off-on cannot resurrect a run even if apparent identities match again`() {
        val measurement = measurement(mapOf("owned" to CheckOutcome.HiddenByBackend))
        assertEquals(MeasurementApplicability.Changed, measurementApplicability(measurement, runContext(), 2, false))
        assertEquals(1L, measurement.context.changeEpoch)
        assertFalse(canPresentCurrentSuccess(measurement, MeasurementApplicability.Changed, DiagnosticEligibility.Eligible))
    }

    @Test
    fun `observation failure allows consistent recovery without redating measurement`() {
        val measurement = measurement(mapOf("owned" to CheckOutcome.HiddenByBackend))
        assertEquals(MeasurementApplicability.Unverified, measurementApplicability(measurement, null, 1, true))
        val refreshed = runContext().copy(observationId = 7, observedAt = 99)
        assertEquals(MeasurementApplicability.MatchesLastObservation, measurementApplicability(measurement, refreshed, 1, false))
        assertEquals(10L, measurement.context.observedAt)
    }

    @Test
    fun `interrupted and changed-backend evidence cannot present fresh success`() {
        val measurement = measurement(mapOf("owned" to CheckOutcome.HiddenByBackend))
        assertEquals(
            MeasurementApplicability.Changed,
            measurementApplicability(measurement.copy(interrupted = true), runContext(), 1, false),
        )
        assertEquals(
            MeasurementApplicability.Changed,
            measurementApplicability(measurement, runContext().copy(coverage = "zygisk:2"), 1, false),
        )
        assertEquals("kpm:1", measurement.context.coverage)
        assertFalse(
            canPresentCurrentSuccess(
                measurement.copy(completed = false),
                MeasurementApplicability.MatchesLastObservation,
                DiagnosticEligibility.Eligible,
            ),
        )
    }

    @Test
    fun `matching evidence cannot show success while current eligibility is unknown or applying`() {
        val measurement = measurement(mapOf("owned" to CheckOutcome.HiddenByBackend))
        for (eligibility in listOf(DiagnosticEligibility.Unknown, DiagnosticEligibility.Applying, DiagnosticEligibility.VpnOff)) {
            assertFalse(canPresentCurrentSuccess(measurement, MeasurementApplicability.MatchesLastObservation, eligibility))
        }
        assertEquals(
            MeasurementApplicability.Unverified,
            measurementApplicability(measurement, runContext().copy(coverage = "old cached backend"), 1, true),
        )
    }

    private fun measurement(outcomes: Map<String, CheckOutcome>): DiagnosticMeasurement =
        DiagnosticMeasurement(
            1,
            runContext(),
            listOf(ProbePlanEntry("owned"), ProbePlanEntry("uncovered", owned = false)),
            outcomes,
            true,
            false,
            20,
        )
}
