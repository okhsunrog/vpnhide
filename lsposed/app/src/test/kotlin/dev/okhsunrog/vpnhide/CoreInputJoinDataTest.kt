package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.AppVpnContext
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticEligibility
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticGate
import dev.okhsunrog.vpnhide.diagnostics.buildDiagnosticContextObservation
import dev.okhsunrog.vpnhide.diagnostics.coreInputsJoined
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class CoreInputJoinDataTest {
    private val state = CoreStateView(7, 5, RootCanonicalRead.Available(CanonicalConfig()), emptyList())
    private val root = joinRootSnapshot(emptyMap(), CoreSessionUpdate(state, current = true, sequence = 12))
    private val accepted = ConfigCoordinatorView(coreCurrent = true, coreEpoch = 7, coreSequence = 12, desiredRevision = 5)

    @Test
    fun `newer root publication waits for actor acceptance before any measurement context exists`() {
        val lagging = accepted.copy(coreSequence = 11)
        assertFalse(coreInputsJoined(root, lagging))
        val routing =
            ObservationState(
                lastGood = ObservedValue(DiagnosticGate.ROUTED, ObservationRequest(1, 0, 0, ReadReason.Background), 0),
                attempted = true,
            )
        val observation =
            buildDiagnosticContextObservation(
                false,
                AppVpnContext(routing, "vpn-session"),
                root,
                CanonicalConfig(),
                "self",
                "process",
                0,
                configJoined = coreInputsJoined(root, lagging),
            )
        assertEquals(DiagnosticEligibility.Checking, observation.eligibility)
        assertNull(observation.context)
        assertTrue(coreInputsJoined(root, accepted))
        assertTrue(coreInputsJoined(root, accepted.copy(coreSequence = 13)))
        assertEquals(
            DiagnosticEligibility.Eligible,
            buildDiagnosticContextObservation(
                false,
                AppVpnContext(routing, "vpn-session"),
                root,
                CanonicalConfig(),
                "self",
                "process",
                0,
                configJoined = coreInputsJoined(root, accepted),
            ).eligibility,
        )
    }

    @Test
    fun `new epoch cannot join previous actor state with the same canonical revision`() {
        val restarted = root.copy(core = root.core.copy(state = state.copy(epoch = 8)))
        assertFalse(coreInputsJoined(restarted, accepted))
        assertTrue(coreInputsJoined(restarted, accepted.copy(coreEpoch = 8)))
        assertFalse(coreInputsJoined(root, accepted.copy(desiredRevision = 6)))
        assertFalse(coreInputsJoined(root, accepted.copy(coreCurrent = false)))
    }

    @Test
    fun `waiting for a joined publication does not hide a known restart requirement`() {
        val observation =
            buildDiagnosticContextObservation(
                true,
                AppVpnContext(ObservationState(), null),
                root,
                CanonicalConfig(),
                "self",
                "process",
                0,
                configJoined = false,
            )
        assertEquals(DiagnosticEligibility.RestartApp, observation.eligibility)
        assertNull(observation.context)
    }
}
