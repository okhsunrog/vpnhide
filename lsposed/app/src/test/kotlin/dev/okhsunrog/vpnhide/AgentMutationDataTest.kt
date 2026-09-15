package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AgentMutationDataTest {
    @Test
    fun `activation failure reports already saved change and target restart advice`() {
        val result =
            CanonicalWriteResult(
                -1,
                "",
                ConfigOperationResult(
                    1,
                    mapOf(
                        ConfigPhase.Persist to PhaseOutcome.Confirmed,
                        ConfigPhase.Native to PhaseOutcome.FailedKnown,
                    ),
                    TransitionFailure.ExecutionFailed,
                ),
            ).toAgentMutationResult(restartTargets = true)
        assertFalse(result.ok)
        assertTrue(result.changed)
        assertTrue(result.targetRestartRecommended)
        assertEquals("execution_failed", result.errorCode)
        assertEquals("failed_known", result.phases["native"])
    }

    @Test
    fun `conflict reports exact path and whether persistence happened`() {
        val field = ConfigField(listOf("apps", "com.example.app", "java"))
        for (outcome in listOf(PhaseOutcome.NotAttempted, PhaseOutcome.Confirmed)) {
            val result =
                CanonicalWriteResult(
                    -1,
                    "",
                    ConfigOperationResult(1, mapOf(ConfigPhase.Persist to outcome), TransitionFailure.UiEditConflict, setOf(field), true),
                ).toAgentMutationResult()
            assertEquals("ui_edit_conflict", result.errorCode)
            assertEquals(listOf(field.segments), result.conflicts)
            assertTrue(result.draftPending)
            assertEquals(outcome == PhaseOutcome.Confirmed, result.changed)
            assertTrue(result.message.contains("unsaved UI edits"))
        }
    }

    @Test
    fun `no op has no restart recommendation`() {
        val result =
            CanonicalWriteResult(
                0,
                "",
                ConfigOperationResult(1, mapOf(ConfigPhase.Persist to PhaseOutcome.NotAttempted)),
            ).toAgentMutationResult(restartTargets = true)
        assertTrue(result.ok)
        assertFalse(result.changed)
        assertFalse(result.targetRestartRecommended)
    }
}
