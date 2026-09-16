package dev.okhsunrog.vpnhide.debug

import dev.okhsunrog.vpnhide.diagnostics.CheckResults
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticCaptureOutcome
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticGate
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticRunResult
import dev.okhsunrog.vpnhide.diagnostics.RunOutcome
import dev.okhsunrog.vpnhide.diagnostics.blockedGate
import java.io.File

/**
 * The explicit result of an export. A capture whose self-test could not measure is
 * still a written bundle — it carries the reason in [Written.errors] — so the only
 * failure left is one that produced no file at all.
 */
internal sealed interface DebugExportOutcome {
    data class Written(
        val file: File,
        val errors: List<String>,
    ) : DebugExportOutcome

    data class Failed(
        val reason: String,
    ) : DebugExportOutcome
}

/**
 * What one capture's self-test contributes to the bundle: the gate the report is
 * built under, the evidence, the run it came from, and the reasons anything is
 * missing. A capture always produces one of these — there is no outcome that
 * makes the whole export fail.
 */
internal data class DebugSelfTest(
    val gate: DiagnosticGate?,
    val checkResults: CheckResults?,
    val runId: Long?,
    val errors: List<String> = emptyList(),
)

/**
 * Map a capture's run onto the bundle fields. The one rule that matters: only a
 * Completed run may report [DiagnosticGate.ROUTED]. An interrupted, failed or
 * never-admitted run contributes its partial evidence and an explicit reason, so
 * a bundle can never present half a measurement as a routed verdict.
 */
internal fun debugSelfTestFrom(outcome: DiagnosticCaptureOutcome): DebugSelfTest =
    when (outcome) {
        is DiagnosticCaptureOutcome.Ran -> debugSelfTestFromRun(outcome.result)
        is DiagnosticCaptureOutcome.NotAdmitted -> DebugSelfTest(null, null, null, listOf("self-test not admitted: ${outcome.reason}"))
    }

private fun debugSelfTestFromRun(result: DiagnosticRunResult): DebugSelfTest {
    val attempt = result.attempt
    return when (attempt.outcome) {
        RunOutcome.Completed -> {
            DebugSelfTest(
                gate = DiagnosticGate.ROUTED,
                checkResults = result.results,
                runId = attempt.id,
                // A completed run with no retained evidence cannot be rendered as a report;
                // say so rather than emitting a gate the bundle then silently drops.
                errors = if (result.results == null) listOf("self-test completed without evidence") else emptyList(),
            )
        }

        RunOutcome.NotStarted -> {
            attempt.eligibility?.let { blockedSelfTest(attempt.id, it.blockedGate(), it.name) }
                ?: failedSelfTest(result, "not started")
        }

        RunOutcome.Interrupted -> {
            failedSelfTest(result, "interrupted")
        }

        RunOutcome.Failed -> {
            failedSelfTest(result, "failed")
        }
    }
}

/**
 * A blocked attempt is a real, informative result: VPN off, this app split-tunnelled
 * out or a pending self-restart become the gate and need no error. An eligibility
 * with no gate vocabulary (Applying, ApplicationUnknown, Unknown) has no gate, so
 * the reason is recorded instead of being lost.
 */
private fun blockedSelfTest(
    runId: Long,
    gate: DiagnosticGate?,
    eligibility: String,
): DebugSelfTest =
    DebugSelfTest(
        gate = gate,
        checkResults = null,
        runId = runId,
        errors = if (gate == null) listOf("self-test not eligible: $eligibility") else emptyList(),
    )

private fun failedSelfTest(
    result: DiagnosticRunResult,
    outcome: String,
): DebugSelfTest =
    DebugSelfTest(
        gate = null,
        checkResults = result.results,
        runId = result.attempt.id,
        errors = listOf("self-test $outcome: ${result.attempt.failure?.name ?: "unknown"}"),
    )
