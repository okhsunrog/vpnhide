package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ConfigStatusDataTest {
    @Test
    fun `rechecking and pending work supersede an old failure`() {
        val paused = ConfigCoordinatorView(mode = ConfigCoordinatorMode.Paused)
        assertTrue(requireNotNull(configStatus(paused)).canRecheck)
        val rechecking = requireNotNull(configStatus(paused.copy(rechecking = true)))
        assertEquals(ConfigStatusMessage.Rechecking, rechecking.message)
        assertFalse(rechecking.canRecheck)
        assertEquals(ConfigStatusMessage.Applying, configStatus(failed(ConfigPhase.Native).copy(operations = setOf(2)))?.message)
    }

    @Test
    fun `secret and cleanup failure cannot be retried with backend activation`() {
        assertEquals(ConfigStatusMessage.SecretFailed, configStatus(failed(ConfigPhase.Secret))?.message)
        assertEquals(ConfigStatusMessage.CleanupFailed, configStatus(failed(ConfigPhase.Cleanup))?.message)
        assertFalse(requireNotNull(configStatus(failed(ConfigPhase.Cleanup))).canApply)
        assertFalse(requireNotNull(configStatus(failed(ConfigPhase.Secret))).canApply)
        assertTrue(requireNotNull(configStatus(failed(ConfigPhase.Native))).canApply)
    }

    private fun failed(phase: ConfigPhase) =
        ConfigCoordinatorView(
            mode = ConfigCoordinatorMode.Open,
            lastResult =
                ConfigOperationResult(
                    1,
                    mapOf(ConfigPhase.Persist to PhaseOutcome.Confirmed, phase to PhaseOutcome.FailedKnown),
                    TransitionFailure.ExecutionFailed,
                ),
        )
}
