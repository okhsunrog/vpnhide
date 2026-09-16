package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.networkViewClean
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The network-view self-test verdict must obey the diagnostic principle: a
 * confirmed violation is a leak even when the capture was incomplete — an error
 * elsewhere must never mask a proven leak. Only a clean-but-incomplete capture is
 * not-measured; a clean, complete one holds.
 */
class NetworkViewCheckTest {
    @Test
    fun `a violation is a leak even with capture errors`() {
        assertFalse(networkViewClean(hasViolation = true, hasError = true)!!)
        assertFalse(networkViewClean(hasViolation = true, hasError = false)!!)
    }

    @Test
    fun `a clean but incomplete capture is not measured`() {
        assertNull(networkViewClean(hasViolation = false, hasError = true))
    }

    @Test
    fun `a clean complete capture holds`() {
        assertTrue(networkViewClean(hasViolation = false, hasError = false)!!)
    }

    @Test
    fun `the verdict is a three-way distinction`() {
        assertEquals(
            listOf<Boolean?>(false, false, null, true),
            listOf(
                networkViewClean(hasViolation = true, hasError = true),
                networkViewClean(hasViolation = true, hasError = false),
                networkViewClean(hasViolation = false, hasError = true),
                networkViewClean(hasViolation = false, hasError = false),
            ),
        )
    }
}
