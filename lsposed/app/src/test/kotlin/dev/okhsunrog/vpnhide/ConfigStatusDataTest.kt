package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ConfigStatusDataTest {
    @Test
    fun `dismissed pause stays quiet until another write attempt`() {
        val paused = ConfigCoordinatorMode.Paused
        val opened = reduceConfigDialog(ConfigDialogState(), ConfigDialogEvent.Observed(paused))
        assertTrue(opened.visible)
        val dismissed = reduceConfigDialog(opened, ConfigDialogEvent.Dismissed(paused))
        assertFalse(dismissed.visible)
        assertEquals(dismissed, reduceConfigDialog(dismissed, ConfigDialogEvent.Observed(paused)))
        assertTrue(reduceConfigDialog(dismissed, ConfigDialogEvent.WriteAttempted(paused)).visible)
    }

    @Test
    fun `automatic recheck stays local and recovery resets acknowledgement`() {
        val paused = ConfigCoordinatorMode.Paused
        val initial = ConfigDialogState()
        assertEquals(initial, reduceConfigDialog(initial, ConfigDialogEvent.Observed(paused, rechecking = true)))
        val opened = reduceConfigDialog(initial, ConfigDialogEvent.Observed(paused))
        assertTrue(reduceConfigDialog(opened, ConfigDialogEvent.Observed(paused, rechecking = true)).visible)
        val dismissed = reduceConfigDialog(opened, ConfigDialogEvent.Dismissed(paused))
        val recovered = reduceConfigDialog(dismissed, ConfigDialogEvent.Observed(ConfigCoordinatorMode.Open))
        assertEquals(initial, recovered)
        assertTrue(reduceConfigDialog(recovered, ConfigDialogEvent.Observed(paused)).visible)
    }

    @Test
    fun `a different blocking problem is explained and an admitted write needs no dialog`() {
        val dismissed = ConfigDialogState(dismissedMode = ConfigCoordinatorMode.Paused)
        assertTrue(reduceConfigDialog(dismissed, ConfigDialogEvent.Observed(ConfigCoordinatorMode.Invalid)).visible)
        assertFalse(reduceConfigDialog(ConfigDialogState(), ConfigDialogEvent.WriteAttempted(ConfigCoordinatorMode.Open)).visible)
        assertTrue(reduceConfigDialog(ConfigDialogState(), ConfigDialogEvent.WriteAttempted(ConfigCoordinatorMode.Initializing)).visible)
    }

    @Test
    fun `normal startup missing config and success never interrupt browsing`() {
        for (mode in listOf(ConfigCoordinatorMode.Initializing, ConfigCoordinatorMode.Missing, ConfigCoordinatorMode.Open)) {
            assertFalse(configNeedsAttention(mode))
            assertNull(configFailureNotice(ConfigCoordinatorView(mode = mode)))
        }
        assertNull(configFailureNotice(failed(ConfigPhase.Native).let { it.copy(lastResult = it.lastResult?.copy(failure = null)) }))
    }

    @Test
    fun `blocking errors use a dialog instead of a snackbar`() {
        for (mode in listOf(
            ConfigCoordinatorMode.Paused,
            ConfigCoordinatorMode.Invalid,
            ConfigCoordinatorMode.Unavailable,
            ConfigCoordinatorMode.RebootRequired,
        )) {
            assertTrue(configNeedsAttention(mode))
            assertNull(configFailureNotice(failed(ConfigPhase.Native).copy(mode = mode)))
        }
    }

    @Test
    fun `progress suppresses stale errors and a second failed attempt has a new notice`() {
        val view = failed(ConfigPhase.Native)
        assertNull(configFailureNotice(view.copy(operations = setOf(2))))
        assertNull(configFailureNotice(view.copy(rechecking = true)))
        val first = requireNotNull(configFailureNotice(view))
        assertTrue(first.status.canApply)
        val second = configFailureNotice(view.copy(lastResult = view.lastResult?.copy(id = 2)))
        assertNotEquals(first, second)
        // Recovery can refine the same operation's failure; do not suppress its new explanation.
        assertNotEquals(first, configFailureNotice(failed(ConfigPhase.Secret)))
    }

    @Test
    fun `known write failure does not offer activation repair`() {
        val notice = requireNotNull(configFailureNotice(failed(ConfigPhase.Persist)))
        assertEquals(ConfigStatusMessage.WriteFailed, notice.status.message)
        assertFalse(notice.status.canApply)
    }

    @Test
    fun `failed activation repair still offers repair without rewriting config`() {
        for (phase in listOf(ConfigPhase.Native, ConfigPhase.Ports)) {
            val view = failed(phase)
            val result = requireNotNull(view.lastResult).copy(phases = mapOf(phase to PhaseOutcome.FailedKnown))
            val notice = requireNotNull(configFailureNotice(view.copy(lastResult = result)))
            assertEquals(ConfigStatusMessage.ApplyFailed, notice.status.message)
            assertTrue(notice.status.canApply)
        }
    }

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
