package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.ConfigReadiness
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticEligibility
import dev.okhsunrog.vpnhide.diagnostics.RestartRequirement
import dev.okhsunrog.vpnhide.diagnostics.RoutingKnowledge
import dev.okhsunrog.vpnhide.diagnostics.SelfRouting
import dev.okhsunrog.vpnhide.diagnostics.diagnosticEligibility
import dev.okhsunrog.vpnhide.diagnostics.routingKnowledge
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
    fun `an invalidated observation awaiting its re-read is checking, a failed one is unknown`() {
        val loading = reduceObservation(ObservationState<SelfRouting>(), ObservationEvent.Ensure(1)).state
        val known = reduceObservation(loading, ObservationEvent.Loaded(1, SelfRouting.Routed, 2)).state
        assertEquals(DiagnosticEligibility.Eligible, eligibility(known))
        // A VPN transition marks the observation stale before the debounced re-read starts.
        val stale = reduceObservation(known, ObservationEvent.Invalidate(3, start = false)).state
        assertEquals(DiagnosticEligibility.Checking, eligibility(stale))
        val reading = reduceObservation(stale, ObservationEvent.Ensure(4)).state
        assertEquals(DiagnosticEligibility.Checking, eligibility(reading))
        val off = reduceObservation(reading, ObservationEvent.Loaded(2, SelfRouting.VpnOff, 5)).state
        assertEquals(DiagnosticEligibility.VpnOff, eligibility(off))
        val failed =
            reduceObservation(
                reduceObservation(off, ObservationEvent.Refresh(6)).state,
                ObservationEvent.Failed(3, TransitionFailure.ReadFailed, 7),
            ).state
        assertEquals(DiagnosticEligibility.Unknown, eligibility(failed))
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

    /**
     * The knowledge model must not move any state to a different verdict: it only
     * keeps what eligibility throws away. Verifying ⇔ Checking, Unknown ⇔ Unknown,
     * Known ⇔ the fact's own verdict, for every reachable routing state.
     */
    @Test
    fun `routing knowledge and eligibility agree on every routing state`() {
        routingStates().forEach { (name, state) ->
            val knowledge = routingKnowledge(state, now = 100)
            assertEquals(name, eligibility(state), knowledge.asEligibility())
        }
    }

    private fun RoutingKnowledge.asEligibility(): DiagnosticEligibility =
        when (this) {
            is RoutingKnowledge.Verifying -> {
                DiagnosticEligibility.Checking
            }

            is RoutingKnowledge.Unknown -> {
                DiagnosticEligibility.Unknown
            }

            is RoutingKnowledge.Known -> {
                when (routing) {
                    SelfRouting.VpnOff -> DiagnosticEligibility.VpnOff
                    SelfRouting.Excluded -> DiagnosticEligibility.SelfExcluded
                    SelfRouting.Routed -> DiagnosticEligibility.Eligible
                }
            }
        }

    /** Every reachable routing state, named, built through the reducer so the table cannot drift from it. */
    private fun routingStates(): List<Pair<String, ObservationState<SelfRouting>>> {
        val initial = ObservationState<SelfRouting>()
        val loading = reduceObservation(initial, ObservationEvent.Ensure(1)).state
        val routed = reduceObservation(loading, ObservationEvent.Loaded(1, SelfRouting.Routed, 2)).state
        val off = routed.copy(lastGood = routed.lastGood?.copy(value = SelfRouting.VpnOff))
        val excluded = routed.copy(lastGood = routed.lastGood?.copy(value = SelfRouting.Excluded))
        val stale = reduceObservation(routed, ObservationEvent.Invalidate(3, start = false)).state
        val rereading = reduceObservation(stale, ObservationEvent.Ensure(4)).state
        val refreshing = reduceObservation(routed, ObservationEvent.Refresh(5, reason = ReadReason.Explicit)).state
        val failed = reduceObservation(refreshing, ObservationEvent.Failed(2, TransitionFailure.ReadFailed, 6)).state
        val firstReadFailed = reduceObservation(loading, ObservationEvent.Failed(1, TransitionFailure.ReadFailed, 2)).state
        val quarantined =
            reduceObservation(
                refreshing,
                ObservationEvent.Failed(2, TransitionFailure.DeadlineExceeded, 6, quiescent = false),
            ).state
        return listOf(
            "never read" to initial,
            "first read in flight" to loading,
            "routed" to routed,
            "vpn off" to off,
            "self excluded" to excluded,
            "invalidated, re-read owed" to stale,
            "invalidated, re-read in flight" to rereading,
            "explicit refresh in flight" to refreshing,
            "refresh failed with history" to failed,
            "first read failed, no history" to firstReadFailed,
            "probe resource quarantined" to quarantined,
        )
    }

    private fun eligibility(routing: ObservationState<SelfRouting>): DiagnosticEligibility =
        diagnosticEligibility(true, RestartRequirement.None, ConfigReadiness.Settled, routing)
}
