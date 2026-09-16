package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.CheckOutcome
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticAttempt
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticEligibility
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticGate
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticMeasurement
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticPresentation
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticStage
import dev.okhsunrog.vpnhide.diagnostics.EvidenceConclusion
import dev.okhsunrog.vpnhide.diagnostics.LayerStatus
import dev.okhsunrog.vpnhide.diagnostics.MeasurementApplicability
import dev.okhsunrog.vpnhide.diagnostics.MeasurementContext
import dev.okhsunrog.vpnhide.diagnostics.MeasurementEvidence
import dev.okhsunrog.vpnhide.diagnostics.NATIVE_CHECKS
import dev.okhsunrog.vpnhide.diagnostics.ProbePlanEntry
import dev.okhsunrog.vpnhide.diagnostics.RunOutcome
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

    @Test
    fun `heroDecision overlays blocked eligibility and keeps protected only for an applicable sufficient measurement`() {
        val checked = dashboardState(protection = ProtectionCheck.Checked(ok, ok))
        val good = presentation()
        assertEquals(HeroDecision(HeroStatus.Protected, HeroNote.None, showsFailedPrompt = false), heroDecision(checked, good, 0, 0))
        assertEquals(HeroStatus.VpnOff, heroDecision(checked, good.copy(eligibility = DiagnosticEligibility.VpnOff), 0, 0).status)
        assertEquals(
            HeroDecision(HeroStatus.Attention, HeroNote.None, showsFailedPrompt = false),
            heroDecision(checked, good.copy(eligibility = DiagnosticEligibility.SelfExcluded), 0, 0),
        )
        assertEquals(
            ProtectionCheck.Blocked(DiagnosticGate.NEEDS_RESTART),
            effectiveProtection(ProtectionCheck.Checked(ok, ok), DiagnosticEligibility.RestartApp),
        )
        assertEquals(HeroNote.Applying, heroDecision(checked, good.copy(eligibility = DiagnosticEligibility.Applying), 0, 0).note)
        assertEquals(HeroStatus.Attention, heroDecision(checked, good.copy(eligibility = DiagnosticEligibility.Unknown), 0, 0).status)
        assertEquals(
            HeroDecision(HeroStatus.Attention, HeroNote.ResultsChanged, showsFailedPrompt = false),
            heroDecision(checked, good.copy(applicability = MeasurementApplicability.Changed), 0, 0),
        )
        // A refresh in flight names itself but does not flip a protected hero.
        assertEquals(
            HeroDecision(HeroStatus.Protected, HeroNote.ResultsUnverified, showsFailedPrompt = false),
            heroDecision(checked, good.copy(applicability = MeasurementApplicability.Unverified), 0, 0),
        )
        assertEquals(
            HeroDecision(HeroStatus.Attention, HeroNote.InsufficientEvidence, showsFailedPrompt = false),
            heroDecision(checked, good.copy(evidence = insufficient), 0, 0),
        )
        val interrupted = DiagnosticAttempt(2, RunOutcome.Interrupted, TransitionFailure.ContextChanged)
        assertEquals(
            HeroDecision(HeroStatus.Attention, HeroNote.Interrupted, showsFailedPrompt = false),
            heroDecision(checked, good.copy(lastAttempt = interrupted), 0, 0),
        )
        // A run that never started because the VPN was off is a condition, not a failed
        // check: once the condition clears the measured hero stays protected, and while a
        // re-check is in flight it only names the confirmation (the retry-with-VPN-off case).
        val blocked = DiagnosticAttempt(2, RunOutcome.NotStarted, eligibility = DiagnosticEligibility.VpnOff)
        assertEquals(
            HeroDecision(HeroStatus.Protected, HeroNote.None, showsFailedPrompt = false),
            heroDecision(checked, good.copy(lastAttempt = blocked), 0, 0),
        )
        assertEquals(
            HeroDecision(HeroStatus.Protected, HeroNote.ResultsUnverified, showsFailedPrompt = false),
            heroDecision(checked, good.copy(lastAttempt = blocked, activeRunId = 3, activeStage = DiagnosticStage.Checking), 0, 0),
        )
        val failed = DiagnosticAttempt(2, RunOutcome.Failed, TransitionFailure.ExecutionFailed)
        assertEquals(HeroNote.Failed, heroDecision(checked, good.copy(lastAttempt = failed), 0, 0).note)
        assertEquals(
            HeroNote.ResultsUnverified,
            heroDecision(checked, good.copy(lastAttempt = failed, activeRunId = 3, activeStage = DiagnosticStage.Core), 0, 0).note,
        )
        // Errors still outrank: an unprotected hero is never lifted by a note.
        assertEquals(HeroStatus.Unprotected, heroDecision(checked, good, errorCount = 1, warningCount = 0).status)
        assertEquals(
            HeroNote.Checking,
            heroDecision(checked, good.copy(measurement = null, eligibility = DiagnosticEligibility.Checking), 0, 0).note,
        )
    }

    @Test
    fun `the failed prompt is shown for an execution failure but not for a run blocked by a current condition`() {
        val failed = dashboardState(protection = ProtectionCheck.Failed)
        val good = presentation()
        assertTrue(heroDecision(failed, good, 0, 0).showsFailedPrompt)
        for (
        eligibility in
        listOf(
            DiagnosticEligibility.Applying,
            DiagnosticEligibility.ApplicationUnknown,
            DiagnosticEligibility.ApplicationFailed,
            DiagnosticEligibility.Unknown,
        )
        ) {
            val decision = heroDecision(failed, good.copy(eligibility = eligibility), 0, 0)
            assertFalse("$eligibility", decision.showsFailedPrompt)
            assertTrue("$eligibility", decision.note.explainsCondition)
        }
        assertFalse(heroDecision(dashboardState(), good.copy(eligibility = DiagnosticEligibility.Applying), 0, 0).showsFailedPrompt)
    }

    @Test
    fun `a quarantined probe is named before any eligibility and replaces the failed prompt`() {
        val good = presentation().copy(probeUnavailable = true)
        val checked = dashboardState(protection = ProtectionCheck.Checked(ok, ok))
        assertEquals(
            HeroDecision(HeroStatus.Attention, HeroNote.ProbeUnavailable, showsFailedPrompt = false),
            heroDecision(checked, good, 0, 0),
        )
        // Even an eligibility that would otherwise word its own note is outranked.
        assertEquals(
            HeroNote.ProbeUnavailable,
            heroDecision(checked, good.copy(eligibility = DiagnosticEligibility.Applying), 0, 0).note,
        )
        val decision = heroDecision(dashboardState(protection = ProtectionCheck.Failed), good, 0, 0)
        assertFalse(decision.showsFailedPrompt)
        assertEquals(HeroNote.ProbeUnavailable, decision.note)
    }

    private val sufficient = MeasurementEvidence(5, 0, 0, 0, 0, 0, 0, EvidenceConclusion.NoObservedLeak)
    private val insufficient = MeasurementEvidence(0, 0, 5, 0, 0, 0, 0, EvidenceConclusion.Insufficient)

    private fun presentation(): DiagnosticPresentation {
        val plan = NATIVE_CHECKS.map { ProbePlanEntry(it.id) }
        val context = MeasurementContext("pid:1;uid:10", "self", "vpn=tun0;self=ROUTED", "backend=Kmod", 0, 1, 10)
        val measurement =
            DiagnosticMeasurement(1, context, plan, plan.associate { it.id to CheckOutcome.HiddenByBackend }, true, false, 20)
        return DiagnosticPresentation(
            eligibility = DiagnosticEligibility.Eligible,
            activeRunId = null,
            activeStage = null,
            activeResults = null,
            lastAttempt = DiagnosticAttempt(1, RunOutcome.Completed, measurement = measurement),
            measurement = measurement,
            measurementResults = null,
            applicability = MeasurementApplicability.MatchesLastObservation,
            evidence = sufficient,
            currentSuccess = true,
            probeUnavailable = false,
        )
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
