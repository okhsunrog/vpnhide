package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.checks.CheckStatus
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class CheckStatusToPassedTest {
    @Test
    fun `pass maps to true, fail to false, network blocked to null`() {
        assertEquals(true, CheckStatus.PASS.toPassed())
        assertEquals(false, CheckStatus.FAIL.toPassed())
        assertNull(CheckStatus.NETWORK_BLOCKED.toPassed())
    }

    @Test
    fun `selinux-blocked maps to true for the legacy tri-state`() {
        // A SELinux-blocked read is "no VPN visible" (green) for the passed-based
        // UI; the honest SELinux-vs-backend split lives in CheckOutcome instead.
        assertEquals(true, CheckStatus.SELINUX_BLOCKED.toPassed())
    }
}
