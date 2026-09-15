package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.debug.debugSelfTestFrom
import dev.okhsunrog.vpnhide.diagnostics.CORE_JAVA_CHECKS
import dev.okhsunrog.vpnhide.diagnostics.CheckOutcome
import dev.okhsunrog.vpnhide.diagnostics.CheckResult
import dev.okhsunrog.vpnhide.diagnostics.CheckResults
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticAttempt
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticCaptureOutcome
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticEligibility
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticGate
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticRunResult
import dev.okhsunrog.vpnhide.diagnostics.JavaCheckSpec
import dev.okhsunrog.vpnhide.diagnostics.NATIVE_CHECKS
import dev.okhsunrog.vpnhide.diagnostics.NATIVE_EXTRA_CHECKS
import dev.okhsunrog.vpnhide.diagnostics.RunOutcome
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The bundle's view of a capture's run. The rule under test: ROUTED is reserved
 * for a completed run, every other outcome is reported with its reason instead of
 * failing the export.
 */
class DebugExportDataTest {
    @Test
    fun `a completed run is the routed gate with its evidence and run id`() {
        val results = coreResults()
        val selfTest = debugSelfTestFrom(ran(DiagnosticAttempt(7, RunOutcome.Completed), results))

        assertEquals(DiagnosticGate.ROUTED, selfTest.gate)
        assertEquals(results, selfTest.checkResults)
        assertEquals(7L, selfTest.runId)
        assertEquals(emptyList<String>(), selfTest.errors)
    }

    @Test
    fun `a completed run without retained evidence reports the anomaly instead of a bare gate`() {
        val selfTest = debugSelfTestFrom(ran(DiagnosticAttempt(7, RunOutcome.Completed), results = null))

        assertNull(selfTest.checkResults)
        assertEquals(listOf("self-test completed without evidence"), selfTest.errors)
    }

    @Test
    fun `a blocked eligibility becomes its gate and is not an error`() {
        val cases =
            mapOf(
                DiagnosticEligibility.VpnOff to DiagnosticGate.VPN_OFF,
                DiagnosticEligibility.SelfExcluded to DiagnosticGate.SELF_NOT_ROUTED,
                DiagnosticEligibility.RestartApp to DiagnosticGate.NEEDS_RESTART,
            )
        for ((eligibility, gate) in cases) {
            val selfTest =
                debugSelfTestFrom(
                    ran(DiagnosticAttempt(3, RunOutcome.NotStarted, eligibility = eligibility), results = null),
                )

            assertEquals(gate, selfTest.gate)
            assertNull(selfTest.checkResults)
            assertEquals(3L, selfTest.runId)
            assertEquals(emptyList<String>(), selfTest.errors)
        }
    }

    @Test
    fun `an eligibility with no gate vocabulary is named in the errors`() {
        for (eligibility in listOf(
            DiagnosticEligibility.Applying,
            DiagnosticEligibility.ApplicationUnknown,
            DiagnosticEligibility.Unknown,
        )) {
            val selfTest =
                debugSelfTestFrom(
                    ran(DiagnosticAttempt(4, RunOutcome.NotStarted, eligibility = eligibility), results = null),
                )

            assertNull(selfTest.gate)
            assertEquals(4L, selfTest.runId)
            assertEquals(listOf("self-test not eligible: ${eligibility.name}"), selfTest.errors)
        }
    }

    @Test
    fun `an interrupted run keeps its partial evidence but never claims routed`() {
        val results = coreResults()
        val selfTest =
            debugSelfTestFrom(
                ran(DiagnosticAttempt(9, RunOutcome.Interrupted, failure = TransitionFailure.ContextChanged), results),
            )

        assertNull(selfTest.gate)
        assertEquals(results, selfTest.checkResults)
        assertEquals(9L, selfTest.runId)
        assertEquals(listOf("self-test interrupted: ContextChanged"), selfTest.errors)
    }

    @Test
    fun `a failed run carries whatever evidence exists and its failure`() {
        val selfTest =
            debugSelfTestFrom(
                ran(DiagnosticAttempt(11, RunOutcome.Failed, failure = TransitionFailure.ExecutionFailed), results = null),
            )

        assertNull(selfTest.gate)
        assertNull(selfTest.checkResults)
        assertEquals(listOf("self-test failed: ExecutionFailed"), selfTest.errors)
    }

    @Test
    fun `a run that never started for a failure reports that failure, not an eligibility`() {
        val selfTest =
            debugSelfTestFrom(
                ran(DiagnosticAttempt(12, RunOutcome.NotStarted, failure = TransitionFailure.DeadlineExceeded), results = null),
            )

        assertNull(selfTest.gate)
        assertEquals(listOf("self-test not started: DeadlineExceeded"), selfTest.errors)
    }

    @Test
    fun `a rejected capture has no run at all`() {
        for (reason in listOf(TransitionFailure.Busy, TransitionFailure.ResourceUnavailable)) {
            val selfTest = debugSelfTestFrom(DiagnosticCaptureOutcome.NotAdmitted(reason))

            assertNull(selfTest.gate)
            assertNull(selfTest.checkResults)
            assertNull(selfTest.runId)
            assertEquals(listOf("self-test not admitted: $reason"), selfTest.errors)
        }
    }

    @Test
    fun `only a completed run may be written up as routed`() {
        val nonCompleted =
            RunOutcome.entries.filter { it != RunOutcome.Completed }.map { outcome ->
                debugSelfTestFrom(
                    ran(DiagnosticAttempt(1, outcome, failure = TransitionFailure.ExecutionFailed), coreResults()),
                )
            }

        assertTrue(nonCompleted.none { it.gate == DiagnosticGate.ROUTED })
    }
}

private fun ran(
    attempt: DiagnosticAttempt,
    results: CheckResults?,
) = DiagnosticCaptureOutcome.Ran(DiagnosticRunResult(attempt, results))

private fun List<JavaCheckSpec>.results(): List<CheckResult> = map { CheckResult(it.id, "", CheckOutcome.HiddenByBackend, id = it.id) }

private fun coreResults(): CheckResults =
    CheckResults(
        native = NATIVE_CHECKS.map { CheckResult(it.id, "", CheckOutcome.HiddenByBackend, "root: tun0", id = it.id) },
        nativeExtra = NATIVE_EXTRA_CHECKS.results(),
        coreJava = CORE_JAVA_CHECKS.results(),
    )
