package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.counterDeltaText
import dev.okhsunrog.vpnhide.generated.HookIds
import org.junit.Assert.assertEquals
import org.junit.Test

class HookDiagnosticsTest {
    @Test
    fun `disabled optional kmod hooks are not reported as missing`() {
        assertEquals(
            KERNEL_HOOKS,
            expectedInstalledHooks(HookIds.Backend.KMOD, installed = KERNEL_HOOKS),
        )
    }

    @Test
    fun `installed optional kmod hooks are preserved as a typed set`() {
        val filesystemHook = HookIds.Hook.FILESYSTEM_IFACE_PATHS
        val installedMask = (KERNEL_HOOKS + filesystemHook).toHookMask()
        val installed = installedHooks(Protocol.formatStatus(kmodStatus(installedMask)))

        assertEquals(KERNEL_HOOKS + filesystemHook, installed)
        assertEquals(
            KERNEL_HOOKS + filesystemHook,
            expectedInstalledHooks(HookIds.Backend.KMOD, installed),
        )
    }

    @Test
    fun `disabled optional kpm hooks are not reported as missing`() {
        assertEquals(
            KERNEL_HOOKS,
            expectedInstalledHooks(HookIds.Backend.KPM, installed = KERNEL_HOOKS),
        )
    }

    @Test
    fun `installed optional kpm hooks are preserved as a typed set`() {
        val filesystemHook = HookIds.Hook.FILESYSTEM_IFACE_PATHS
        assertEquals(
            KERNEL_HOOKS + filesystemHook,
            expectedInstalledHooks(HookIds.Backend.KPM, KERNEL_HOOKS + filesystemHook),
        )
    }

    @Test
    fun `no baseline captured yields n a`() {
        // A baseline that was never captured must read n/a — but this is driven by
        // an explicit hasBaseline flag, NOT by the baseline map being empty (a
        // fresh boot captures a valid but empty baseline).
        assertEquals("n/a", counterDeltaText(current = 5, baseline = 3, hasBaseline = false))
    }

    @Test
    fun `new counter with a captured baseline shows the full count`() {
        assertEquals("+5", counterDeltaText(current = 5, baseline = null, hasBaseline = true))
    }

    @Test
    fun `positive delta is the difference`() {
        assertEquals("+7", counterDeltaText(current = 10, baseline = 3, hasBaseline = true))
    }

    @Test
    fun `a counter that went down reads as reset`() {
        assertEquals("reset", counterDeltaText(current = 2, baseline = 9, hasBaseline = true))
    }

    @Test
    fun `unsigned arithmetic survives values past Long MAX`() {
        // Counters are unsigned; a value beyond Long.MAX_VALUE is stored as a
        // negative Long. Comparisons/subtraction must treat it as unsigned.
        assertEquals(
            "+1",
            counterDeltaText(current = Long.MIN_VALUE + 1, baseline = Long.MIN_VALUE, hasBaseline = true),
        )
        // current=0 (unsigned 0) is below baseline=-1 (unsigned ULong.MAX) → reset.
        assertEquals("reset", counterDeltaText(current = 0, baseline = -1L, hasBaseline = true))
    }

    private fun kmodStatus(hooks: Long): Protocol.Status {
        val backend =
            HookIds.Backend.KMOD.id
                .toLong()
        val ok =
            HookIds.StatusError.OK.code
                .toLong()
        return Protocol.Status(
            backend = backend,
            kver = 0,
            hooks = hooks,
            error = ok,
        )
    }
}
