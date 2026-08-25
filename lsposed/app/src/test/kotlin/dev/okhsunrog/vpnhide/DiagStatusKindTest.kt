package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.CheckOutcome
import dev.okhsunrog.vpnhide.diagnostics.CheckResult
import dev.okhsunrog.vpnhide.diagnostics.DiagStatusKind
import dev.okhsunrog.vpnhide.diagnostics.NotMeasuredReason
import dev.okhsunrog.vpnhide.diagnostics.ProtectionCounts
import dev.okhsunrog.vpnhide.diagnostics.anyNetworkBlocked
import dev.okhsunrog.vpnhide.diagnostics.diagStatusKind
import dev.okhsunrog.vpnhide.diagnostics.protectionCounts
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The Diagnostics screen renders each check purely from these decisions, so the
 * per-outcome bucket and the summary counts are pinned here — the Composable is
 * left as a trivial kind→(label,colour) lookup that needs no instrumentation.
 */
class DiagStatusKindTest {
    private fun check(outcome: CheckOutcome) = CheckResult("c", detail = "", outcome = outcome)

    @Test
    fun `each outcome maps to its screen bucket`() {
        assertEquals(DiagStatusKind.Leak, CheckOutcome.Leak.diagStatusKind())
        assertEquals(DiagStatusKind.Ok, CheckOutcome.HiddenByBackend.diagStatusKind())
        assertEquals(DiagStatusKind.NothingToLeak, CheckOutcome.NothingToLeak.diagStatusKind())
        assertEquals(DiagStatusKind.Selinux, CheckOutcome.HiddenBySelinux.diagStatusKind())
        assertEquals(
            DiagStatusKind.NotMeasured,
            CheckOutcome.NotMeasured(NotMeasuredReason.NoGroundTruth).diagStatusKind(),
        )
    }

    @Test
    fun `summary counts hidden by backend and selinux, and leaks, ignoring the rest`() {
        val checks =
            listOf(
                check(CheckOutcome.HiddenByBackend),
                check(CheckOutcome.HiddenBySelinux),
                check(CheckOutcome.Leak),
                check(CheckOutcome.NothingToLeak),
                check(CheckOutcome.NotMeasured(NotMeasuredReason.NoGroundTruth)),
            )
        assertEquals(ProtectionCounts(hidden = 2, leaks = 1), checks.protectionCounts())
    }

    @Test
    fun `network-blocked is only the no-network not-measured reason`() {
        assertTrue(listOf(check(CheckOutcome.NotMeasured(NotMeasuredReason.NoNetworkPermission))).anyNetworkBlocked())
        assertFalse(listOf(check(CheckOutcome.NotMeasured(NotMeasuredReason.NoGroundTruth))).anyNetworkBlocked())
        assertFalse(listOf(check(CheckOutcome.Leak)).anyNetworkBlocked())
    }
}
