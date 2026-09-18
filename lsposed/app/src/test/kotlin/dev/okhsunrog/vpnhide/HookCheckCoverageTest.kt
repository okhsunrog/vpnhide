package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.NATIVE_CHECKS
import dev.okhsunrog.vpnhide.generated.HookIds
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * Every kernel hook must be exercised by at least one diagnostic check.
 *
 * The partial-hooks story leans on the checks: a hook that silently fails to
 * install is only caught behaviourally when a check for the vector it covers
 * measures a leak. A kernel hook with NO check has no such safety net — it could
 * regress and the suite would still read green, because nothing probes it.
 *
 * So this fails the build when a KERNEL_HOOKS member is not named by any
 * [NATIVE_CHECKS] spec's `expectedHooks`. Adding a hook without wiring a check
 * (or, deliberately, listing it in [DELIBERATELY_UNCHECKED] with a reason) is a
 * compile-green gap this test turns into a red one.
 */
class HookCheckCoverageTest {
    /**
     * Kernel hooks intentionally without a diagnostic check. Empty today — every
     * kernel hook has an observable vector. A future hook with no probeable
     * surface goes here WITH a comment saying why, so the exemption is a decision
     * on the record rather than an oversight.
     */
    private val deliberatelyUnchecked: Set<HookIds.Hook> = emptySet()

    @Test
    fun `every kernel hook is covered by a diagnostic check`() {
        val checked = NATIVE_CHECKS.flatMap { it.expectedHooks }.toSet()
        val uncovered = KERNEL_HOOKS - checked - deliberatelyUnchecked
        assertEquals(
            "kernel hooks with no diagnostic check (add a NATIVE_CHECKS entry, " +
                "or list them in deliberatelyUnchecked with a reason): $uncovered",
            emptySet<HookIds.Hook>(),
            uncovered,
        )
    }

    @Test
    fun `the unchecked exemption list stays honest`() {
        // An exemption for a hook that DOES have a check is stale — drop it.
        val checked = NATIVE_CHECKS.flatMap { it.expectedHooks }.toSet()
        val staleExemptions = deliberatelyUnchecked.intersect(checked)
        assertEquals(emptySet<HookIds.Hook>(), staleExemptions)
    }
}
