package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.DiagnosticGate
import dev.okhsunrog.vpnhide.diagnostics.LayerStatus
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class DashboardUiStateTest {
    private val ok = LayerStatus.Active(hidden = 5, leaks = 0)
    private val partial = LayerStatus.Active(hidden = 5, leaks = 1)
    private val broken = LayerStatus.Active(hidden = 0, leaks = 3)

    @Test
    fun `computeHeroStatus returns protected when checks pass and there are no issues`() {
        assertEquals(
            HeroStatus.Protected,
            computeHeroStatus(
                state =
                    dashboardState(
                        protection = ProtectionCheck.Checked(ok, ok),
                    ),
                errorCount = 0,
                warningCount = 0,
            ),
        )
    }

    @Test
    fun `computeHeroStatus ignores info messages`() {
        val state =
            dashboardState(
                protection = ProtectionCheck.Checked(ok, ok),
                messages = listOf(DashboardMessage(DashboardMessageSeverity.INFO, "note")),
            )

        assertEquals(
            HeroStatus.Protected,
            computeHeroStatus(
                state = state,
                errorCount = 0,
                warningCount = 0,
            ),
        )
    }

    @Test
    fun `protectionFullyPassed is true only when native and java layers are ok`() {
        assertTrue(protectionFullyPassed(ProtectionCheck.Checked(ok, ok)))
        assertFalse(protectionFullyPassed(ProtectionCheck.Blocked(DiagnosticGate.VPN_OFF)))
        assertFalse(protectionFullyPassed(ProtectionCheck.Blocked(DiagnosticGate.NEEDS_RESTART)))
        assertFalse(protectionFullyPassed(ProtectionCheck.Blocked(DiagnosticGate.SELF_NOT_ROUTED)))
        assertFalse(protectionFullyPassed(ProtectionCheck.Checked(LayerStatus.Absent, ok)))
        assertFalse(protectionFullyPassed(ProtectionCheck.Checked(partial, ok)))
        assertFalse(protectionFullyPassed(ProtectionCheck.Checked(ok, partial)))
        assertFalse(protectionFullyPassed(ProtectionCheck.Checked(ok, LayerStatus.Inactive)))
    }

    @Test
    fun `computeHeroStatus returns vpn off before issue ranking`() {
        assertEquals(
            HeroStatus.VpnOff,
            computeHeroStatus(
                state = dashboardState(protection = ProtectionCheck.Blocked(DiagnosticGate.VPN_OFF)),
                errorCount = 1,
                warningCount = 1,
            ),
        )
    }

    @Test
    fun `computeHeroStatus returns attention for restart partial layer or warning`() {
        assertEquals(
            HeroStatus.Attention,
            computeHeroStatus(
                state = dashboardState(protection = ProtectionCheck.Blocked(DiagnosticGate.NEEDS_RESTART)),
                errorCount = 0,
                warningCount = 0,
            ),
        )
        // A VPN is up but this app is split-tunnelled out — action needed, not a
        // hard failure and not "VPN off".
        assertEquals(
            HeroStatus.Attention,
            computeHeroStatus(
                state = dashboardState(protection = ProtectionCheck.Blocked(DiagnosticGate.SELF_NOT_ROUTED)),
                errorCount = 0,
                warningCount = 0,
            ),
        )
        // A failed run couldn't measure — attention, never "VPN off" (the VPN may be up).
        assertEquals(
            HeroStatus.Attention,
            computeHeroStatus(
                state = dashboardState(protection = ProtectionCheck.Failed),
                errorCount = 0,
                warningCount = 0,
            ),
        )
        assertEquals(
            HeroStatus.Attention,
            computeHeroStatus(
                state =
                    dashboardState(
                        protection = ProtectionCheck.Checked(partial, ok),
                    ),
                errorCount = 0,
                warningCount = 0,
            ),
        )
        // A couple of failing Java probes is Partial (works, has a gap) → Attention,
        // no longer a hard "not working".
        assertEquals(
            HeroStatus.Attention,
            computeHeroStatus(
                state =
                    dashboardState(
                        protection = ProtectionCheck.Checked(ok, partial),
                    ),
                errorCount = 0,
                warningCount = 0,
            ),
        )
        assertEquals(
            HeroStatus.Attention,
            computeHeroStatus(
                state =
                    dashboardState(
                        protection = ProtectionCheck.Checked(ok, ok),
                    ),
                errorCount = 0,
                warningCount = 1,
            ),
        )
    }

    @Test
    fun `computeHeroStatus returns unprotected for broken layer or errors`() {
        assertEquals(
            HeroStatus.Unprotected,
            computeHeroStatus(
                state =
                    dashboardState(
                        protection = ProtectionCheck.Checked(broken, ok),
                    ),
                errorCount = 0,
                warningCount = 0,
            ),
        )
        assertEquals(
            HeroStatus.Unprotected,
            computeHeroStatus(
                state =
                    dashboardState(
                        protection = ProtectionCheck.Checked(ok, broken),
                    ),
                errorCount = 0,
                warningCount = 0,
            ),
        )
        assertEquals(
            HeroStatus.Unprotected,
            computeHeroStatus(
                state =
                    dashboardState(
                        protection = ProtectionCheck.Checked(ok, ok),
                    ),
                errorCount = 1,
                warningCount = 0,
            ),
        )
    }

    @Test
    fun `moduleActive is true only for active installed module`() {
        assertTrue(moduleActive(ModuleState.Installed(version = "1.0", active = true)))
        assertFalse(moduleActive(ModuleState.Installed(version = "1.0", active = false)))
        assertFalse(moduleActive(ModuleState.NotInstalled))
    }

    @Test
    fun `activeModuleCount and moduleSummaryText count active runtime modules`() {
        val state =
            dashboardState(
                kmod = ModuleState.Installed(version = "1.0", active = true),
                zygisk = ModuleState.Installed(version = "1.0", active = false),
                lsposed = LsposedState.Active(version = "1.0", targetCount = 3),
                ports = ModuleState.NotInstalled,
            )

        // Native layer counts once (kmod active); +LSPosed = 2 of the 3 layers.
        assertEquals(2, activeModuleCount(state))
        assertEquals("2/3", moduleSummaryText(state))
    }

    private fun dashboardState(
        kmod: ModuleState = ModuleState.NotInstalled,
        kpm: ModuleState = ModuleState.NotInstalled,
        zygisk: ModuleState = ModuleState.NotInstalled,
        lsposed: LsposedState = LsposedState.NotInstalled,
        ports: ModuleState = ModuleState.NotInstalled,
        protection: ProtectionCheck = ProtectionCheck.Checked(ok, ok),
        messages: List<DashboardMessage> = emptyList(),
    ): DashboardState =
        DashboardState(
            kmod = kmod,
            kpm = kpm,
            zygisk = zygisk,
            lsposed = lsposed,
            ports = ports,
            nativeTargetCount = 0,
            portsTargetCount = 0,
            nativeBackend = displayNativeBackend(NativeBackendStates(kmod = kmod, kpm = kpm, zygisk = zygisk)),
            nativeInstallRecommendation = null,
            kmodLoadStatus = null,
            protection = protection,
            messages = messages,
        )
}
