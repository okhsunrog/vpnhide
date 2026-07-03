package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * Layer rollup: presence gates before the checks (an inactive backend can never
 * render a verdict), and the tile is judged only on vectors the backend owns —
 * the fix for the old "inactive backend shows Partial" from SELinux-only passes.
 */
class LayerStatusTest {
    private fun installed(active: Boolean) = ModuleState.Installed(version = "1.0", active = active, targetCount = 1)

    private fun kmod(state: ModuleState) =
        displayNativeBackend(
            NativeBackendStates(kmod = state, kpm = ModuleState.NotInstalled, zygisk = ModuleState.NotInstalled),
        )

    private fun zygisk(state: ModuleState) =
        displayNativeBackend(
            NativeBackendStates(kmod = ModuleState.NotInstalled, kpm = ModuleState.NotInstalled, zygisk = state),
        )

    // ── verdict ───────────────────────────────────────────────────────────

    @Test
    fun `verdict is Ok when nothing owned leaks`() {
        assertEquals(Verdict.Ok, LayerStatus.Active(hidden = 5, leaks = 0).verdict)
    }

    @Test
    fun `verdict is Partial when it hides some but an owned vector still leaks`() {
        assertEquals(Verdict.Partial, LayerStatus.Active(hidden = 5, leaks = 1).verdict)
    }

    @Test
    fun `verdict is Broken when active but hid nothing while leaking`() {
        assertEquals(Verdict.Broken, LayerStatus.Active(hidden = 0, leaks = 3).verdict)
    }

    // ── presence gates before checks ───────────────────────────────────────

    @Test
    fun `native layer is Absent when the module is not installed`() {
        assertEquals(LayerStatus.Absent, summarizeNativeLayer(kmod(ModuleState.NotInstalled), emptyMap()))
    }

    @Test
    fun `native layer is Inactive when installed but not loaded`() {
        // Even with SELinux-only passes in the map, an inactive backend is never Active.
        val outcomes = mapOf("ioctl_flags" to CheckOutcome.HiddenBySelinux)
        assertEquals(LayerStatus.Inactive, summarizeNativeLayer(kmod(installed(active = false)), outcomes))
    }

    // ── the tile is judged only on OWNED vectors ───────────────────────────

    @Test
    fun `a leak on a vector the kernel backend does not own is out of scope for the tile`() {
        // ioctl_flags: kernel-owned (dev_ioctl). proc_dev: zygisk-only (no kernel
        // hook) → a leak there must NOT drag the kmod tile toward Broken.
        val outcomes =
            mapOf(
                "ioctl_flags" to CheckOutcome.HiddenByBackend,
                "ioctl_mtu" to CheckOutcome.HiddenByBackend,
                "proc_dev" to CheckOutcome.Leak,
            )
        val layer = summarizeNativeLayer(kmod(installed(active = true)), outcomes)
        assertEquals(LayerStatus.Active(hidden = 2, leaks = 0), layer)
        // …but it is still surfaced, via the unowned-leak count (the hero warning).
        assertEquals(1, unownedNativeLeaks(kmod(installed(active = true)), outcomes))
    }

    @Test
    fun `a leak on an owned vector counts against the tile`() {
        val outcomes = mapOf("ioctl_flags" to CheckOutcome.Leak)
        val layer = summarizeNativeLayer(kmod(installed(active = true)), outcomes)
        assertEquals(LayerStatus.Active(hidden = 0, leaks = 1), layer)
        assertEquals(Verdict.Broken, (layer as LayerStatus.Active).verdict)
    }

    @Test
    fun `ownership is per backend family - zygisk owns what the kernel does not`() {
        // Mirror image of the kmod case: proc_dev (zygisk openat) is zygisk-owned;
        // netlink_getrule (fib_nl_fill_rule, kernel-only — no zygisk parses rule
        // dumps) is not. That kernel-only leak is out of scope for the zygisk tile
        // and shows up as an unowned leak instead. (Note: ioctl_flags would NOT
        // work here — it is zygisk-owned too, via zygisk_ioctl.)
        val outcomes =
            mapOf(
                "proc_dev" to CheckOutcome.Leak,
                "netlink_getrule" to CheckOutcome.Leak,
            )
        assertEquals(
            LayerStatus.Active(hidden = 0, leaks = 1),
            summarizeNativeLayer(zygisk(installed(active = true)), outcomes),
        )
        assertEquals(1, unownedNativeLeaks(zygisk(installed(active = true)), outcomes))
    }

    // ── java layer ─────────────────────────────────────────────────────────

    @Test
    fun `java layer is Inactive when LSPosed is not active`() {
        assertEquals(LayerStatus.Inactive, summarizeJavaLayer(lsposedActive = false, javaChecks = emptyList()))
    }

    @Test
    fun `java layer counts passed as hidden and failed as leaks, ignoring not-run`() {
        val checks =
            listOf(
                CheckResult("a", passed = true, detail = ""),
                CheckResult("b", passed = true, detail = ""),
                CheckResult("c", passed = false, detail = ""),
                CheckResult("d", passed = null, detail = ""),
            )
        assertEquals(
            LayerStatus.Active(hidden = 2, leaks = 1),
            summarizeJavaLayer(lsposedActive = true, javaChecks = checks),
        )
    }
}
