package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class NativeBackendJoinTest {
    private val raw = NativeBackendStates(ModuleState.Installed("1", true), ModuleState.NotInstalled, ModuleState.Installed("1", false))

    @Test
    fun `unknown observation withdraws active identity without inventing inactivity`() {
        val result = joinNativeBackendStates(raw, CoreBackendView(false, active = NativeBackendId.Kmod))
        assertEquals(null, result.activeId)
        assertFalse((result.kmod as ModuleState.Installed).runtimeCheckable)
    }

    @Test
    fun `current observed backend takes precedence over previous probe and boot selection`() {
        val result =
            joinNativeBackendStates(
                raw,
                CoreBackendView(
                    true,
                    selected = NativeBackendId.Kmod,
                    applied = NativeBackendId.Kmod,
                    active = NativeBackendId.Zygisk,
                ),
            )
        assertEquals(NativeBackendId.Zygisk, result.activeId)
        assertFalse((result.kmod as ModuleState.Installed).active)
    }
}
