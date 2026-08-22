package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.generated.HookIds
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * A kernel backend that loads but cannot resolve every hook target.
 *
 * From a real report (MediaTek 4.14): the KPM came up with mask 0x20003bc and
 * `error 0x4` (PARTIAL_HOOKS) because three symbols were renamed by Clang CFI,
 * so `ioctl(SIOCGIFCONF)` still enumerated tun0 — while the dashboard showed a
 * green "active" card. The gap has to be nameable for that to be explainable.
 */
class PartialHookGapTest {
    private val active = ModuleState.Installed(version = "1.2.5", active = true)

    private fun backend(
        id: NativeBackendId?,
        state: ModuleState = active,
    ) = DisplayNativeBackend(id, state)

    @Test
    fun `a complete kernel install reports no gap`() {
        assertNull(partialHookGap(backend(NativeBackendId.Kpm), KERNEL_HOOKS))
    }

    @Test
    fun `missing kernel hooks are named`() {
        val reported = KERNEL_HOOKS - HookIds.Hook.SOCK_IOCTL - HookIds.Hook.FIB_ROUTE_SEQ_SHOW
        val gap = partialHookGap(backend(NativeBackendId.Kpm), reported)!!

        assertEquals(KERNEL_HOOKS.size, gap.expected)
        assertEquals(KERNEL_HOOKS.size - 2, gap.installed)
        assertEquals(
            listOf(HookIds.Hook.FIB_ROUTE_SEQ_SHOW, HookIds.Hook.SOCK_IOCTL),
            gap.missing,
        )
    }

    @Test
    fun `an unread status is not reported as a total failure`() {
        // No status line read (no root, backend not answering) must not render as
        // "0 of 12 hooks installed" — that would be a fabricated diagnosis.
        assertNull(partialHookGap(backend(NativeBackendId.Kpm), emptySet()))
    }

    @Test
    fun `an inactive backend produces no gap`() {
        assertNull(
            partialHookGap(
                backend(NativeBackendId.Kpm, ModuleState.Installed(version = "1.2.5", active = false)),
                KERNEL_HOOKS - HookIds.Hook.SOCK_IOCTL,
            ),
        )
    }

    @Test
    fun `zygisk is out of scope — its mask is per-process`() {
        assertNull(partialHookGap(backend(NativeBackendId.Zygisk), setOf(HookIds.Hook.ZYGISK_IOCTL)))
    }

    @Test
    fun `the leaking check carries the hook that never installed`() {
        val reported = KERNEL_HOOKS - HookIds.Hook.SOCK_IOCTL
        val missing = missingBackendHooks(NativeBackendId.Kpm, reported)

        assertEquals(setOf(HookIds.Hook.SOCK_IOCTL), missing)
        // ioctl_conf is the vector SOCK_IOCTL covers — the one that leaked.
        val ioctlConf = NATIVE_CHECKS.first { it.id == "ioctl_conf" }
        assertTrue(HookIds.Hook.SOCK_IOCTL in ioctlConf.expectedHooks)
    }
}
