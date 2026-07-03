package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * The Java layer has no root differential; the self-in-tunnel gate guarantees a
 * VPN artifact is present, so the outcome is binary — clean is hidden-by-LSPosed,
 * dirty is a leak — with NotMeasured only as a defensive edge for a probe that
 * could not run (passed == null). There is no nothing-to-leak on this layer.
 */
class ClassifyJavaOutcomeTest {
    @Test
    fun `app saw the VPN is a leak`() {
        assertEquals(CheckOutcome.Leak, classifyJavaOutcome(false))
    }

    @Test
    fun `clean app view is hidden by the backend`() {
        assertEquals(CheckOutcome.HiddenByBackend, classifyJavaOutcome(true))
    }

    @Test
    fun `a probe that could not run is not measured`() {
        assertEquals(
            CheckOutcome.NotMeasured(NotMeasuredReason.NoGroundTruth),
            classifyJavaOutcome(null),
        )
    }
}
