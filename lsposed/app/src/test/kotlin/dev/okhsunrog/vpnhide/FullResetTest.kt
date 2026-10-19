package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class FullResetTest {
    private val installed = ModuleState.Installed(version = "1.0", active = false)
    private val activeLsposed = LsposedState.Active(version = "1.0", targetCount = 0)

    @Test
    fun `ready when nothing is installed or active`() {
        assertEquals(
            emptyList<ResetBlocker>(),
            resetBlockers(
                kmod = ModuleState.NotInstalled,
                kpm = ModuleState.NotInstalled,
                zygisk = ModuleState.NotInstalled,
                lsposed = LsposedState.NotInstalled,
                kernelCtlPresent = false,
            ),
        )
    }

    @Test
    fun `each remaining backend or hook is a blocker`() {
        assertEquals(
            listOf(
                ResetBlocker.KmodInstalled,
                ResetBlocker.KpmInstalled,
                ResetBlocker.ZygiskInstalled,
                ResetBlocker.KernelStillHooked,
                ResetBlocker.LsposedActive,
            ),
            resetBlockers(
                kmod = installed,
                kpm = installed,
                zygisk = installed,
                lsposed = activeLsposed,
                kernelCtlPresent = true,
            ),
        )
    }

    @Test
    fun `a disabled-but-present module still blocks`() {
        // module dir present (Installed, inactive) -> user must remove it, not
        // just disable it.
        assertEquals(
            listOf(ResetBlocker.KmodInstalled),
            resetBlockers(
                kmod = installed,
                kpm = ModuleState.NotInstalled,
                zygisk = ModuleState.NotInstalled,
                lsposed = LsposedState.NotInstalled,
                kernelCtlPresent = false,
            ),
        )
    }

    @Test
    fun `loaded ko with no module dir still blocks via proc`() {
        // Removed the module but didn't reboot: /proc/vpnhide_ctl still exists.
        assertEquals(
            listOf(ResetBlocker.KernelStillHooked),
            resetBlockers(
                kmod = ModuleState.NotInstalled,
                kpm = ModuleState.NotInstalled,
                zygisk = ModuleState.NotInstalled,
                lsposed = LsposedState.NotInstalled,
                kernelCtlPresent = true,
            ),
        )
    }

    @Test
    fun `inactive lsposed module does not block`() {
        // disabled hook (InstalledInactive) is fine — only an active hook blocks.
        assertEquals(
            emptyList<ResetBlocker>(),
            resetBlockers(
                kmod = ModuleState.NotInstalled,
                kpm = ModuleState.NotInstalled,
                zygisk = ModuleState.NotInstalled,
                lsposed = LsposedState.InstalledInactive(version = "1.0"),
                kernelCtlPresent = false,
            ),
        )
    }
}
