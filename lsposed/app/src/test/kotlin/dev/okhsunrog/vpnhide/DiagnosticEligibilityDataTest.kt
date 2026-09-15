package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.ConfigReadiness
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticEligibility
import dev.okhsunrog.vpnhide.diagnostics.RestartRequirement
import dev.okhsunrog.vpnhide.diagnostics.SelfRouting
import dev.okhsunrog.vpnhide.diagnostics.diagnosticEligibility
import org.junit.Assert.assertEquals
import org.junit.Test

class DiagnosticEligibilityDataTest {
    @Test
    fun `gate failure retains historical routed evidence but becomes unknown for every consumer`() {
        val loading = reduceObservation(ObservationState<SelfRouting>(), ObservationEvent.Ensure(1)).state
        val known = reduceObservation(loading, ObservationEvent.Loaded(1, SelfRouting.Routed, 2)).state
        val refresh = reduceObservation(known, ObservationEvent.Refresh(3)).state
        assertEquals(DiagnosticEligibility.Checking, eligibility(refresh))
        val failed = reduceObservation(refresh, ObservationEvent.Failed(2, TransitionFailure.ReadFailed, 4)).state
        assertEquals(SelfRouting.Routed, failed.lastGood?.value)
        assertEquals(DiagnosticEligibility.Unknown, eligibility(failed))
        val retry = reduceObservation(failed, ObservationEvent.Refresh(5)).state
        val excluded = reduceObservation(retry, ObservationEvent.Loaded(3, SelfRouting.Excluded, 6)).state
        assertEquals(DiagnosticEligibility.SelfExcluded, eligibility(excluded))
    }

    @Test
    fun `initialization restart and application uncertainty outrank stale network facts`() {
        val unknown = ObservationState<SelfRouting>()
        assertEquals(
            DiagnosticEligibility.Initializing,
            diagnosticEligibility(false, RestartRequirement.Device, ConfigReadiness.Unknown, unknown),
        )
        assertEquals(
            DiagnosticEligibility.RestartDevice,
            diagnosticEligibility(true, RestartRequirement.Device, ConfigReadiness.Unknown, unknown),
        )
        assertEquals(
            DiagnosticEligibility.ApplicationUnknown,
            diagnosticEligibility(true, RestartRequirement.None, ConfigReadiness.Unknown, unknown),
        )
        assertEquals(
            DiagnosticEligibility.ApplicationFailed,
            diagnosticEligibility(true, RestartRequirement.None, ConfigReadiness.Failed, unknown),
        )
    }

    private fun eligibility(routing: ObservationState<SelfRouting>): DiagnosticEligibility =
        diagnosticEligibility(true, RestartRequirement.None, ConfigReadiness.Settled, routing)
}
