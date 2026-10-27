package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.ConfigReadiness
import dev.okhsunrog.vpnhide.diagnostics.coreMeasurementReadiness
import dev.okhsunrog.vpnhide.diagnostics.desiredDeliveryIdentity
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class DesiredDeliveryDataTest {
    private val self = "self.app"
    private val config = CanonicalConfig(apps = mapOf(self to CanonicalApp(native = NativeRole.All)))

    @Test
    fun `new app process cannot infer desired delivery from saved canonical state`() {
        val state =
            ConfigCoordinatorView(confirmed = config, desiredRevision = 5, coreCurrent = true, heldResources = setOf(ConfigPhase.Native))
        assertFalse(desiredDeliveryIdentity(state, self).confirmed)
        // Actual probes remain useful while a retained apply is unknown.
        assertEquals(ConfigReadiness.Settled, coreMeasurementReadiness(state, ConfigReadiness.Settled))
        val applied =
            state.copy(
                heldResources = emptySet(),
                nativeObservationCurrent = true,
                applyEvidence = listOf(CoreApplyIdentity(ConfigPhase.Native, 5, 91, PhaseOutcome.Confirmed)),
            )
        assertTrue(desiredDeliveryIdentity(applied, self).confirmed)
        assertFalse(desiredDeliveryIdentity(applied.copy(desiredRevision = 6), self).confirmed)
    }

    @Test
    fun `ongoing core work survives lost app intent and blocks measurements until settled`() {
        val state = ConfigCoordinatorView(confirmed = config, desiredRevision = 5, coreCurrent = true, coreOperations = setOf(92))
        assertEquals(ConfigReadiness.Applying, coreMeasurementReadiness(state, ConfigReadiness.Settled))
        assertFalse(desiredDeliveryIdentity(state, self).confirmed)
    }
}
