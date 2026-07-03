package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.checks.CheckOutput
import dev.okhsunrog.vpnhide.checks.CheckStatus
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * The root-differential is the core of the diagnostics redesign: it splits a
 * clean app-view into hidden-by-backend / hidden-by-SELinux / nothing-to-leak by
 * comparing against an unfiltered root ground truth. These cases pin the exact
 * truth table verified on-device (Pixel 4a / 8 Pro, enforcing & permissive).
 */
class ClassifyNativeOutcomeTest {
    private fun out(
        status: CheckStatus,
        detail: String = "",
    ) = CheckOutput(status, detail)

    // ── A leak is a leak, whatever the ground truth says ──────────────────

    @Test
    fun `app sees VPN is always a leak`() {
        for (gt in listOf(null, out(CheckStatus.PASS), out(CheckStatus.FAIL))) {
            assertEquals(CheckOutcome.Leak, classifyNativeOutcome(out(CheckStatus.FAIL), gt))
        }
    }

    // ── Probe could not run ───────────────────────────────────────────────

    @Test
    fun `network-blocked app view is not measured, regardless of ground truth`() {
        assertEquals(
            CheckOutcome.NotMeasured(NotMeasuredReason.NoNetworkPermission),
            classifyNativeOutcome(out(CheckStatus.NETWORK_BLOCKED), out(CheckStatus.FAIL)),
        )
    }

    // ── Clean app view (PASS): who hid it? ────────────────────────────────

    @Test
    fun `pass with root also clean is nothing to leak`() {
        assertEquals(
            CheckOutcome.NothingToLeak,
            classifyNativeOutcome(out(CheckStatus.PASS), out(CheckStatus.PASS)),
        )
    }

    @Test
    fun `pass while root sees the VPN is hidden by the backend`() {
        assertEquals(
            CheckOutcome.HiddenByBackend,
            classifyNativeOutcome(out(CheckStatus.PASS), out(CheckStatus.FAIL)),
        )
    }

    @Test
    fun `pass without a usable ground truth is not measured`() {
        assertEquals(
            CheckOutcome.NotMeasured(NotMeasuredReason.NoGroundTruth),
            classifyNativeOutcome(out(CheckStatus.PASS), null),
        )
        // Root itself blocked/limited → still cannot attribute.
        assertEquals(
            CheckOutcome.NotMeasured(NotMeasuredReason.NoGroundTruth),
            classifyNativeOutcome(out(CheckStatus.PASS), out(CheckStatus.SELINUX_BLOCKED)),
        )
        assertEquals(
            CheckOutcome.NotMeasured(NotMeasuredReason.NoGroundTruth),
            classifyNativeOutcome(out(CheckStatus.PASS), out(CheckStatus.NETWORK_BLOCKED)),
        )
    }

    // ── SELinux-blocked app view: the honest split from the redesign ──────

    @Test
    fun `selinux-blocked while root sees the VPN is hidden by SELinux, not the backend`() {
        assertEquals(
            CheckOutcome.HiddenBySelinux,
            classifyNativeOutcome(out(CheckStatus.SELINUX_BLOCKED), out(CheckStatus.FAIL)),
        )
    }

    @Test
    fun `selinux-blocked but root is clean is nothing to leak (empty ground truth wins)`() {
        // The block is moot when there is nothing on that surface to hide.
        assertEquals(
            CheckOutcome.NothingToLeak,
            classifyNativeOutcome(out(CheckStatus.SELINUX_BLOCKED), out(CheckStatus.PASS)),
        )
    }

    @Test
    fun `selinux-blocked without a usable ground truth is not measured`() {
        assertEquals(
            CheckOutcome.NotMeasured(NotMeasuredReason.NoGroundTruth),
            classifyNativeOutcome(out(CheckStatus.SELINUX_BLOCKED), null),
        )
    }

    // ── Stable tokens (agent bridge / debug export wire) ──────────────────

    @Test
    fun `tokens are stable`() {
        assertEquals("leak", CheckOutcome.Leak.token())
        assertEquals("hidden_backend", CheckOutcome.HiddenByBackend.token())
        assertEquals("hidden_selinux", CheckOutcome.HiddenBySelinux.token())
        assertEquals("nothing_to_leak", CheckOutcome.NothingToLeak.token())
        assertEquals(
            "not_measured_no_network",
            CheckOutcome.NotMeasured(NotMeasuredReason.NoNetworkPermission).token(),
        )
        assertEquals(
            "not_measured_no_ground_truth",
            CheckOutcome.NotMeasured(NotMeasuredReason.NoGroundTruth).token(),
        )
    }
}
