package dev.okhsunrog.vpnhide.hook

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The per-candidate cover-usability policy. The source sequencing (underlying →
 * default → heuristic, and heuristic only when the AOSP methods are absent) lives
 * in [VisibleNetworkResolver] against the live service; this pins what makes a
 * single candidate a usable cover at all.
 */
class NetworkViewResolverDataTest {
    private fun candidate(
        source: String = "underlying",
        netId: Int = 100,
        isVpn: Boolean = false,
        hasInternet: Boolean = true,
        notRestricted: Boolean = true,
        blocked: Boolean? = false,
    ) = CoverCandidate(source, netId, isVpn, hasInternet, notRestricted, blocked)

    @Test
    fun `a non-vpn internet network the uid can use is a usable cover`() {
        assertTrue(isUsableCover(candidate()))
    }

    @Test
    fun `a vpn candidate is never a usable cover`() {
        assertFalse(isUsableCover(candidate(isVpn = true)))
    }

    @Test
    fun `a candidate without internet is not usable`() {
        // A transient underlying still coming up has no capabilities yet.
        assertFalse(isUsableCover(candidate(hasInternet = false)))
    }

    @Test
    fun `a restricted candidate is not usable`() {
        assertFalse(isUsableCover(candidate(notRestricted = false)))
    }

    @Test
    fun `a candidate blocked for the uid is not usable`() {
        // A background uid whose networks are blocked by the per-uid firewall: no
        // cover, so the caller reports no active network — as a no-VPN uid would.
        assertFalse(isUsableCover(candidate(blocked = true)))
    }

    @Test
    fun `an unknown UID policy must not authorize the cover`() {
        assertFalse(isUsableCover(candidate(blocked = null)))
    }
}
