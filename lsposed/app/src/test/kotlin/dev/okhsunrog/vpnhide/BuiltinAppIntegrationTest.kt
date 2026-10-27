package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.generated.HookIds
import dev.okhsunrog.vpnhide.picker.AppEntry
import dev.okhsunrog.vpnhide.picker.parseTargetsSnapshot
import dev.okhsunrog.vpnhide.picker.toggleAllProtection
import dev.okhsunrog.vpnhide.statistics.buildStatisticsState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class BuiltinAppIntegrationTest {
    private val builtinSections =
        mapOf(
            "proc_exists" to "1",
            "kmod_state" to "vpnhide 1 status\nbackend 0x4\nkver 0x60100\nhooks 0x20003ff\nerror 0x0\n",
        )
    private val builtinPublication =
        CoreSessionUpdate(
            CoreStateView(
                1,
                1,
                RootCanonicalRead.Missing,
                emptyList(),
                backend = CoreBackendView(current = true, active = NativeBackendId.Builtin, installed = setOf(NativeBackendId.Builtin)),
            ),
            current = true,
        )

    @Test
    fun `builtin participates in native conflict classification`() {
        assertEquals(MultiNativeSeverity.None, classifyMultiNative(false, false, false, builtinActive = true))
        assertEquals(MultiNativeSeverity.Warning, classifyMultiNative(false, false, true, builtinActive = true))
        assertEquals(MultiNativeSeverity.Error, classifyMultiNative(false, true, false, builtinActive = true))
    }

    @Test
    fun `whole row toggle includes builtin native and can clear it again`() {
        val targets = parseTargetsSnapshot(joinRootSnapshot(builtinSections, builtinPublication))
        val app = AppEntry(packageName = "com.byxiaorun.detector", label = "Ruru", icon = null, isSystem = false)
        val selected = toggleAllProtection(app, targets)
        assertTrue(selected.java && selected.native && selected.appHiding && selected.ports)
        assertFalse(toggleAllProtection(selected, targets).anySelected)
    }

    @Test
    fun `builtin exposes native picker role and kernel hook configuration`() {
        val targets = parseTargetsSnapshot(joinRootSnapshot(builtinSections, builtinPublication))
        assertTrue(targets.anyNativeInstalled)
        assertFalse(targets.kmodModuleInstalled)
        assertEquals(NativeBackendId.Builtin, targets.activeNativeBackendId)
        assertEquals(NativeBackendId.Builtin, targets.displayNativeBackendId)
        assertEquals(NativeHookFamily.Kmod, targets.nativeHookFamily)
    }

    @Test
    fun `no live built-in node means no built-in backend`() {
        // The driver has no module: without its control node there is nothing to configure.
        val targets = parseTargetsSnapshot(RootSnapshot(builtinSections - "kmod_state" - "proc_exists"))
        assertFalse(targets.anyNativeInstalled)
        assertEquals(null, targets.displayNativeBackendId)
    }

    @Test
    fun `builtin statistics exist before first interception and keep their identity afterwards`() {
        for (stats in listOf("", "vpnhide 1 stats\n0x28ad 0x3:0x2\n")) {
            val state =
                buildStatisticsState(
                    RootSnapshot(builtinSections + ("kmod_state" to (builtinSections.getValue("kmod_state") + stats))),
                )
            val native = state.backends.first()
            assertEquals(HookIds.Backend.BUILTIN, native.backend)
            assertEquals(if (stats.isEmpty()) 0uL else 2uL, native.totalCount)
        }
    }

    @Test
    fun `optional builtin hooks are expected only when installed`() {
        assertEquals(KERNEL_HOOKS, expectedInstalledHooks(HookIds.Backend.BUILTIN, KERNEL_HOOKS))
        val withFilesystem = KERNEL_HOOKS + HookIds.Hook.FILESYSTEM_IFACE_PATHS
        assertEquals(withFilesystem, expectedInstalledHooks(HookIds.Backend.BUILTIN, withFilesystem))
    }

    @Test
    fun `a live built-in driver blocks reset through its control node`() {
        val blockers =
            resetBlockers(
                kmod = ModuleState.NotInstalled,
                kpm = ModuleState.NotInstalled,
                zygisk = ModuleState.NotInstalled,
                lsposed = LsposedState.NotInstalled,
                kernelCtlPresent = builtinSections["proc_exists"] == "1",
            )
        assertEquals(listOf(ResetBlocker.KernelStillHooked), blockers)
    }
}
