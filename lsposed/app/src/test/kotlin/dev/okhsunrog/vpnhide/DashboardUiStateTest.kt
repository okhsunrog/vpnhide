package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.ActionNeededKind
import dev.okhsunrog.vpnhide.diagnostics.CheckingWhat
import dev.okhsunrog.vpnhide.diagnostics.CouldNotCheckCause
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticGate
import dev.okhsunrog.vpnhide.diagnostics.EvidenceConclusion
import dev.okhsunrog.vpnhide.diagnostics.LayerStatus
import dev.okhsunrog.vpnhide.diagnostics.SelfRouting
import dev.okhsunrog.vpnhide.diagnostics.Situation
import dev.okhsunrog.vpnhide.diagnostics.Staleness
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** One row of the design review's scenario table (docs/notes/ui-state-presentation-review.md §2/§3.C). */
private data class HeroRow(
    val name: String,
    val situation: Situation,
    val expected: HeroVisual,
    val tiles: ProtectionCheck,
    val errorCount: Int = 0,
    val warningCount: Int = 0,
)

class DashboardUiStateTest {
    private val ok = LayerStatus.Active(hidden = 5, leaks = 0)
    private val partial = LayerStatus.Active(hidden = 5, leaks = 1)
    private val broken = LayerStatus.Active(hidden = 0, leaks = 3)
    private val okTiles = ProtectionCheck.Checked(ok, ok)

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
    fun `every situation is worded as one whole hero visual`() {
        val rows = checkingRows() + conditionRows() + couldNotCheckRows() + measuredRows() + staleMeasurementRows()
        rows.forEach { row ->
            assertEquals(row.name, row.expected, heroVisual(row.situation, row.tiles, row.errorCount, row.warningCount))
        }
    }

    /** A read or a run in flight: neutral, a progress bubble, and the last known condition's prompt. */
    private fun checkingRows(): List<HeroRow> =
        listOf(
            HeroRow(
                "nothing observed yet",
                Situation.Initializing,
                HeroVisual(HeroTone.Neutral, HeroTitle.Checking, HeroSubtitle.RunningChecks, HeroPrompt.None, checking = true),
                okTiles,
            ),
            HeroRow(
                "Retry with the VPN off keeps the VPN-off prompt, with its button busy",
                Situation.Checking(CheckingWhat.VpnState, SelfRouting.VpnOff, ReadReason.Explicit, 0),
                HeroVisual(HeroTone.Neutral, HeroTitle.Checking, HeroSubtitle.CheckingVpn, HeroPrompt.VpnOff, checking = true),
                okTiles,
            ),
            HeroRow(
                "a re-read while this app is split-tunnelled out keeps its own prompt",
                Situation.Checking(CheckingWhat.VpnState, SelfRouting.Excluded, ReadReason.Transition, 0),
                HeroVisual(HeroTone.Neutral, HeroTitle.Checking, HeroSubtitle.CheckingVpn, HeroPrompt.SelfExcluded, checking = true),
                okTiles,
            ),
            HeroRow(
                "the suite is running with no condition to prompt about",
                Situation.Checking(CheckingWhat.Suite, SelfRouting.Routed, ReadReason.Explicit, 0),
                HeroVisual(HeroTone.Neutral, HeroTitle.Checking, HeroSubtitle.RunningChecks, HeroPrompt.None, checking = true),
                okTiles,
            ),
            HeroRow(
                "a configuration change settling is a wait, not a problem",
                Situation.Checking(CheckingWhat.ConfigApplying, SelfRouting.Routed, ReadReason.Explicit, 0),
                HeroVisual(HeroTone.Neutral, HeroTitle.Checking, HeroSubtitle.ApplyingConfig, HeroPrompt.None, checking = true),
                okTiles,
            ),
        )

    /** Known conditions: grey when nothing is claimed, yellow when the user owes an action. */
    private fun conditionRows(): List<HeroRow> =
        listOf(
            HeroRow(
                "VPN off",
                Situation.VpnOff,
                HeroVisual(HeroTone.Neutral, HeroTitle.VpnOff, HeroSubtitle.InactiveWithoutVpn, HeroPrompt.VpnOff, checking = false),
                okTiles,
            ),
            HeroRow(
                "this app is excluded from the tunnel: expected, not a warning",
                Situation.NotMeasurable,
                HeroVisual(HeroTone.Neutral, HeroTitle.CannotCheck, HeroSubtitle.SelfExcluded, HeroPrompt.SelfExcluded, checking = false),
                okTiles,
            ),
            HeroRow(
                "the app must be restarted before hiding can be checked",
                Situation.ActionNeeded(ActionNeededKind.RestartApp),
                HeroVisual(HeroTone.Attention, HeroTitle.RestartToCheck, HeroSubtitle.RestartApp, HeroPrompt.Restart, checking = false),
                okTiles,
            ),
            HeroRow(
                "a device reboot is owed",
                Situation.ActionNeeded(ActionNeededKind.RestartDevice),
                HeroVisual(HeroTone.Attention, HeroTitle.RestartToCheck, HeroSubtitle.RestartDevice, HeroPrompt.Restart, checking = false),
                okTiles,
            ),
            HeroRow(
                "the last configuration change could not be confirmed",
                Situation.ActionNeeded(ActionNeededKind.ApplicationUnknown),
                HeroVisual(
                    HeroTone.Attention,
                    HeroTitle.NeedsAttention,
                    HeroSubtitle.ApplicationUnknown,
                    HeroPrompt.None,
                    checking = false,
                ),
                okTiles,
            ),
            HeroRow(
                "the last configuration change did not apply",
                Situation.ActionNeeded(ActionNeededKind.ApplicationFailed),
                HeroVisual(
                    HeroTone.Attention,
                    HeroTitle.NeedsAttention,
                    HeroSubtitle.ApplicationFailed,
                    HeroPrompt.None,
                    checking = false,
                ),
                okTiles,
            ),
        )

    /** No current answer: yellow, the cause in the subtitle, a re-check unless one would be rejected. */
    private fun couldNotCheckRows(): List<HeroRow> =
        listOf(
            HeroRow(
                "the routing read failed",
                Situation.CouldNotCheck(CouldNotCheckCause.RoutingUnknown(TransitionFailure.ReadFailed)),
                HeroVisual(HeroTone.Attention, HeroTitle.CouldNotCheck, HeroSubtitle.RoutingUnknown, HeroPrompt.Recheck, checking = false),
                okTiles,
            ),
            HeroRow(
                "the latest run could not execute",
                Situation.CouldNotCheck(CouldNotCheckCause.RunFailed(TransitionFailure.ExecutionFailed)),
                HeroVisual(HeroTone.Attention, HeroTitle.CouldNotCheck, HeroSubtitle.LastCheckFailed, HeroPrompt.Recheck, checking = false),
                okTiles,
            ),
            HeroRow(
                "the latest run was cut short with nothing to fall back on",
                Situation.CouldNotCheck(CouldNotCheckCause.Interrupted(TransitionFailure.ContextChanged)),
                HeroVisual(
                    HeroTone.Attention,
                    HeroTitle.CouldNotCheck,
                    HeroSubtitle.LastCheckInterrupted,
                    HeroPrompt.Recheck,
                    checking = false,
                ),
                okTiles,
            ),
            HeroRow(
                "a quarantined probe offers no re-check: a new run would be rejected",
                Situation.CouldNotCheck(CouldNotCheckCause.ProbeUnavailable),
                HeroVisual(HeroTone.Attention, HeroTitle.CouldNotCheck, HeroSubtitle.ProbeUnavailable, HeroPrompt.None, checking = false),
                okTiles,
            ),
        )

    /** A current, sufficient measurement: the tiles and the dashboard issues decide the tone. */
    private fun measuredRows(): List<HeroRow> =
        listOf(
            HeroRow(
                "a current, sufficient measurement with clean tiles",
                Situation.Measured(EvidenceConclusion.NoObservedLeak, Staleness.Current),
                HeroVisual(HeroTone.Protected, HeroTitle.VpnHidden, HeroSubtitle.AllLayersActive, HeroPrompt.None, checking = false),
                okTiles,
            ),
            HeroRow(
                "a silent background confirmation renders exactly like a current one",
                Situation.Measured(EvidenceConclusion.NoObservedLeak, Staleness.Confirming(ReadReason.Background, since = 0)),
                HeroVisual(HeroTone.Protected, HeroTitle.VpnHidden, HeroSubtitle.AllLayersActive, HeroPrompt.None, checking = false),
                okTiles,
            ),
            HeroRow(
                "a dashboard warning is worth a look even with clean tiles",
                Situation.Measured(EvidenceConclusion.NoObservedLeak, Staleness.Current),
                HeroVisual(
                    HeroTone.Attention,
                    HeroTitle.NeedsAttention,
                    HeroSubtitle.SomeChecksNeedLook,
                    HeroPrompt.None,
                    checking = false,
                ),
                okTiles,
                warningCount = 1,
            ),
            HeroRow(
                "a dashboard error outranks a clean measurement",
                Situation.Measured(EvidenceConclusion.NoObservedLeak, Staleness.Current),
                HeroVisual(HeroTone.Unprotected, HeroTitle.VpnVisible, HeroSubtitle.HidingNotActive, HeroPrompt.None, checking = false),
                okTiles,
                errorCount = 1,
            ),
            HeroRow(
                "a layer with a gap is worth a look",
                Situation.Measured(EvidenceConclusion.Partial, Staleness.Current),
                HeroVisual(
                    HeroTone.Attention,
                    HeroTitle.NeedsAttention,
                    HeroSubtitle.SomeChecksNeedLook,
                    HeroPrompt.None,
                    checking = false,
                ),
                ProtectionCheck.Checked(partial, ok),
            ),
            HeroRow(
                "a leaking, dead layer is not hidden",
                Situation.Measured(EvidenceConclusion.OwnedLeak, Staleness.Current),
                HeroVisual(HeroTone.Unprotected, HeroTitle.VpnVisible, HeroSubtitle.HidingNotActive, HeroPrompt.None, checking = false),
                ProtectionCheck.Checked(ok, broken),
            ),
        )

    /** A measurement the conditions outran, or one that proved nothing: the wording asks for more. */
    private fun staleMeasurementRows(): List<HeroRow> =
        listOf(
            HeroRow(
                "conditions moved on since the measurement: ask for a re-check",
                Situation.Measured(EvidenceConclusion.NoObservedLeak, Staleness.Changed),
                HeroVisual(HeroTone.Attention, HeroTitle.RecheckNeeded, HeroSubtitle.ResultsChanged, HeroPrompt.Recheck, checking = false),
                okTiles,
            ),
            HeroRow(
                "a re-check never softens a leaking layer to yellow",
                Situation.Measured(EvidenceConclusion.OwnedLeak, Staleness.Changed),
                HeroVisual(
                    HeroTone.Unprotected,
                    HeroTitle.RecheckNeeded,
                    HeroSubtitle.ResultsChanged,
                    HeroPrompt.Recheck,
                    checking = false,
                ),
                ProtectionCheck.Checked(broken, ok),
            ),
            HeroRow(
                "a measurement that attributed nothing proves nothing",
                Situation.Measured(EvidenceConclusion.Insufficient, Staleness.Current),
                HeroVisual(
                    HeroTone.Attention,
                    HeroTitle.NeedsAttention,
                    HeroSubtitle.InsufficientEvidence,
                    HeroPrompt.None,
                    checking = false,
                ),
                okTiles,
            ),
            HeroRow(
                "tiles from another instant are worth a look, never a green claim",
                Situation.Measured(EvidenceConclusion.NoObservedLeak, Staleness.Current),
                HeroVisual(
                    HeroTone.Attention,
                    HeroTitle.NeedsAttention,
                    HeroSubtitle.SomeChecksNeedLook,
                    HeroPrompt.None,
                    checking = false,
                ),
                ProtectionCheck.Failed,
            ),
        )

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
