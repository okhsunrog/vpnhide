package dev.okhsunrog.vpnhide.hook

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * The cover-selection policy: which network a target uid is shown in place of the
 * VPN, given candidates already in AOSP priority order (underlying, default,
 * heuristic). Each case is a routing situation the resolver must get right.
 */
class NetworkViewResolverDataTest {
    private fun candidate(
        source: String,
        netId: Int,
        isVpn: Boolean = false,
        hasInternet: Boolean = true,
        notRestricted: Boolean = true,
        blocked: Boolean = false,
    ) = CoverCandidate(source, netId, isVpn, hasInternet, notRestricted, blocked)

    @Test
    fun `the vpn underlying network wins over the heuristic-best`() {
        val chosen =
            selectCoverIndex(
                listOf(
                    candidate("underlying", 101),
                    candidate("heuristic", 114),
                ),
                underlyingDeclaredEmpty = false,
            )
        assertEquals(0, chosen)
    }

    @Test
    fun `an unusable underlying falls through to the default network`() {
        // The underlying is briefly restricted/no-internet during handover.
        val chosen =
            selectCoverIndex(
                listOf(
                    candidate("underlying", 101, hasInternet = false),
                    candidate("default", 114),
                    candidate("heuristic", 114),
                ),
                underlyingDeclaredEmpty = false,
            )
        assertEquals(1, chosen)
    }

    @Test
    fun `a restricted or blocked candidate is never a cover`() {
        val chosen =
            selectCoverIndex(
                listOf(
                    candidate("default", 120, notRestricted = false),
                    candidate("default", 121, blocked = true),
                    candidate("heuristic", 114),
                ),
                underlyingDeclaredEmpty = false,
            )
        assertEquals(2, chosen)
    }

    @Test
    fun `a vpn candidate is never chosen even first in order`() {
        val chosen =
            selectCoverIndex(
                listOf(
                    candidate("underlying", 113, isVpn = true),
                    candidate("heuristic", 114),
                ),
                underlyingDeclaredEmpty = false,
            )
        assertEquals(1, chosen)
    }

    @Test
    fun `an explicitly empty underlying means no cover, never the heuristic`() {
        val chosen =
            selectCoverIndex(
                listOf(candidate("heuristic", 114)),
                underlyingDeclaredEmpty = true,
            )
        assertNull(chosen)
    }

    @Test
    fun `no usable candidate resolves to no cover`() {
        val chosen =
            selectCoverIndex(
                listOf(
                    candidate("default", 120, hasInternet = false),
                    candidate("heuristic", 121, notRestricted = false),
                ),
                underlyingDeclaredEmpty = false,
            )
        assertNull(chosen)
    }
}
