package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.ActionNeededKind
import dev.okhsunrog.vpnhide.diagnostics.CheckingWhat
import dev.okhsunrog.vpnhide.diagnostics.CouldNotCheckCause
import dev.okhsunrog.vpnhide.diagnostics.EvidenceConclusion
import dev.okhsunrog.vpnhide.diagnostics.LayerStatus
import dev.okhsunrog.vpnhide.diagnostics.SelfRouting
import dev.okhsunrog.vpnhide.diagnostics.Situation
import dev.okhsunrog.vpnhide.diagnostics.Staleness
import dev.okhsunrog.vpnhide.diagnostics.Verdict
import dev.okhsunrog.vpnhide.diagnostics.verdict

/** The hero's colour: green, yellow, red, grey. A function of the case, never of a note. */
internal enum class HeroTone { Protected, Attention, Unprotected, Neutral }

internal enum class HeroTitle {
    VpnHidden,
    NeedsAttention,
    VpnVisible,
    VpnOff,
    Checking,
    CannotCheck,
    RestartToCheck,
    CouldNotCheck,
    RecheckNeeded,
}

internal enum class HeroSubtitle {
    AllLayersActive,
    SomeChecksNeedLook,
    HidingNotActive,
    InactiveWithoutVpn,
    CheckingVpn,
    RunningChecks,
    ApplyingConfig,
    SelfExcluded,
    RestartApp,
    RestartDevice,
    ApplicationUnknown,
    ApplicationFailed,
    RoutingUnknown,
    LastCheckFailed,
    LastCheckInterrupted,
    ProbeUnavailable,
    ResultsChanged,
    InsufficientEvidence,
}

/** The block under the hero: a condition to act on, or nothing. */
internal enum class HeroPrompt { None, VpnOff, SelfExcluded, Restart, Recheck }

/**
 * Everything the Dashboard hero renders, decided purely from one [Situation]
 * (plus the tiles and the issue counts, which the Diagnostics banner does not
 * show and which therefore stay inputs to this map rather than fields of the
 * Situation). The screen turns the enums into strings and colours; it decides
 * nothing.
 */
internal data class HeroVisual(
    val tone: HeroTone,
    val title: HeroTitle,
    val subtitle: HeroSubtitle,
    val prompt: HeroPrompt,
    /** A progress indicator replaces the icon in the bubble and the prompt's button is busy. */
    val checking: Boolean,
)

internal fun heroVisual(
    situation: Situation,
    tiles: ProtectionCheck?,
    errorCount: Int,
    warningCount: Int,
): HeroVisual =
    when (situation) {
        // Nothing has been observed yet, not even a failure: the suite is what we are waiting for.
        Situation.Initializing -> {
            checkingVisual(CheckingWhat.Suite, lastKnown = null)
        }

        is Situation.Checking -> {
            checkingVisual(situation.what, situation.lastKnown)
        }

        Situation.VpnOff -> {
            HeroVisual(HeroTone.Neutral, HeroTitle.VpnOff, HeroSubtitle.InactiveWithoutVpn, HeroPrompt.VpnOff, checking = false)
        }

        Situation.NotMeasurable -> {
            HeroVisual(HeroTone.Neutral, HeroTitle.CannotCheck, HeroSubtitle.SelfExcluded, HeroPrompt.SelfExcluded, checking = false)
        }

        is Situation.ActionNeeded -> {
            actionNeededVisual(situation.kind)
        }

        is Situation.CouldNotCheck -> {
            couldNotCheckVisual(situation.cause)
        }

        is Situation.Measured -> {
            measuredVisual(situation, tiles, errorCount, warningCount)
        }
    }

/**
 * A read or a run in flight makes no claim, so the hero is neutral and says what it
 * is waiting for. The last known condition keeps its prompt — the user should still
 * see "turn the VPN on" while we re-read — with the button busy for the duration.
 */
private fun checkingVisual(
    what: CheckingWhat,
    lastKnown: SelfRouting?,
): HeroVisual =
    HeroVisual(
        tone = HeroTone.Neutral,
        title = HeroTitle.Checking,
        subtitle =
            when (what) {
                CheckingWhat.VpnState -> HeroSubtitle.CheckingVpn
                CheckingWhat.Suite -> HeroSubtitle.RunningChecks
                CheckingWhat.ConfigApplying -> HeroSubtitle.ApplyingConfig
            },
        prompt =
            when (lastKnown) {
                SelfRouting.VpnOff -> HeroPrompt.VpnOff
                SelfRouting.Excluded -> HeroPrompt.SelfExcluded
                SelfRouting.Routed, null -> HeroPrompt.None
            },
        checking = true,
    )

/** An action the user owes: the restart cases name the action, the configuration ones name the failure. */
private fun actionNeededVisual(kind: ActionNeededKind): HeroVisual =
    when (kind) {
        ActionNeededKind.RestartApp -> {
            HeroVisual(HeroTone.Attention, HeroTitle.RestartToCheck, HeroSubtitle.RestartApp, HeroPrompt.Restart, checking = false)
        }

        ActionNeededKind.RestartDevice -> {
            HeroVisual(HeroTone.Attention, HeroTitle.RestartToCheck, HeroSubtitle.RestartDevice, HeroPrompt.Restart, checking = false)
        }

        ActionNeededKind.ApplicationUnknown -> {
            HeroVisual(HeroTone.Attention, HeroTitle.NeedsAttention, HeroSubtitle.ApplicationUnknown, HeroPrompt.None, checking = false)
        }

        ActionNeededKind.ApplicationFailed -> {
            HeroVisual(HeroTone.Attention, HeroTitle.NeedsAttention, HeroSubtitle.ApplicationFailed, HeroPrompt.None, checking = false)
        }
    }

/** No current answer, and why. A quarantined probe offers no re-check: a new run would be rejected. */
private fun couldNotCheckVisual(cause: CouldNotCheckCause): HeroVisual {
    val subtitle =
        when (cause) {
            is CouldNotCheckCause.RoutingUnknown -> HeroSubtitle.RoutingUnknown
            is CouldNotCheckCause.RunFailed -> HeroSubtitle.LastCheckFailed
            is CouldNotCheckCause.Interrupted -> HeroSubtitle.LastCheckInterrupted
            CouldNotCheckCause.ProbeUnavailable -> HeroSubtitle.ProbeUnavailable
        }
    val prompt = if (cause == CouldNotCheckCause.ProbeUnavailable) HeroPrompt.None else HeroPrompt.Recheck
    return HeroVisual(HeroTone.Attention, HeroTitle.CouldNotCheck, subtitle, prompt, checking = false)
}

/**
 * A measurement is presentable: the tone is the worst signal among the tiles and the
 * dashboard issues, ranked exactly as the hero has always ranked them. A measurement
 * the conditions have outrun, or one that attributed nothing, replaces the wording
 * and is at least Attention — but never softens a worse signal, so a broken layer or
 * an error stays red while the re-check is asked for. [Staleness.Confirming] renders
 * exactly like [Staleness.Current]: it is silent by design. Tiles derived from an
 * older attempt than the measurement are passed as null: the evidence conclusion
 * ranks instead, so a fresh measurement is never yellowed by the previous tiles.
 */
private fun measuredVisual(
    measured: Situation.Measured,
    tiles: ProtectionCheck?,
    errorCount: Int,
    warningCount: Int,
): HeroVisual {
    val layers = tiles?.let(::tilesRank) ?: evidenceRank(measured.evidence)
    val rank = maxOf(layers, issuesRank(errorCount, warningCount))
    if (measured.staleness == Staleness.Changed) {
        return HeroVisual(
            toneOf(maxOf(rank, RANK_ATTENTION)),
            HeroTitle.RecheckNeeded,
            HeroSubtitle.ResultsChanged,
            HeroPrompt.Recheck,
            false,
        )
    }
    if (measured.evidence == EvidenceConclusion.Insufficient) {
        return HeroVisual(
            toneOf(maxOf(rank, RANK_ATTENTION)),
            HeroTitle.NeedsAttention,
            HeroSubtitle.InsufficientEvidence,
            HeroPrompt.None,
            checking = false,
        )
    }
    return HeroVisual(toneOf(rank), titleOf(rank), subtitleOf(rank), HeroPrompt.None, checking = false)
}

// Worst-signal ranks: 0 = nothing to report, 1 = worth a look, 2 = hiding is not working.
private const val RANK_PROTECTED = 0
private const val RANK_ATTENTION = 1
private const val RANK_UNPROTECTED = 2

private fun toneOf(rank: Int): HeroTone =
    when (rank) {
        RANK_PROTECTED -> HeroTone.Protected
        RANK_ATTENTION -> HeroTone.Attention
        else -> HeroTone.Unprotected
    }

private fun titleOf(rank: Int): HeroTitle =
    when (rank) {
        RANK_PROTECTED -> HeroTitle.VpnHidden
        RANK_ATTENTION -> HeroTitle.NeedsAttention
        else -> HeroTitle.VpnVisible
    }

private fun subtitleOf(rank: Int): HeroSubtitle =
    when (rank) {
        RANK_PROTECTED -> HeroSubtitle.AllLayersActive
        RANK_ATTENTION -> HeroSubtitle.SomeChecksNeedLook
        else -> HeroSubtitle.HidingNotActive
    }

/**
 * Only [ProtectionCheck.Checked] tiles are expected beside a measurement; a blocked
 * or failed one means the tiles and the situation came from different instants, and
 * "worth a look" is the honest reading of that.
 */
private fun tilesRank(tiles: ProtectionCheck): Int =
    when (tiles) {
        is ProtectionCheck.Checked -> maxOf(tiles.native.heroRank(), tiles.java.heroRank())
        is ProtectionCheck.Blocked, ProtectionCheck.Failed -> RANK_ATTENTION
    }

/** The measurement's own worst signal, for the moment its tiles have not been derived yet. */
private fun evidenceRank(evidence: EvidenceConclusion): Int =
    when (evidence) {
        EvidenceConclusion.NoObservedLeak -> RANK_PROTECTED
        EvidenceConclusion.Partial, EvidenceConclusion.Insufficient -> RANK_ATTENTION
        EvidenceConclusion.OwnedLeak -> RANK_UNPROTECTED
    }

private fun issuesRank(
    errorCount: Int,
    warningCount: Int,
): Int =
    when {
        errorCount > 0 -> RANK_UNPROTECTED
        warningCount > 0 -> RANK_ATTENTION
        else -> RANK_PROTECTED
    }

/** Worst-signal rank a layer contributes: leaking-and-dead = 2, partial / inactive / absent = 1, ok = 0. */
private fun LayerStatus.heroRank(): Int =
    when (this) {
        LayerStatus.Absent, LayerStatus.Inactive, LayerStatus.Unverified -> {
            RANK_ATTENTION
        }

        is LayerStatus.Active -> {
            when (verdict) {
                Verdict.Ok -> RANK_PROTECTED
                Verdict.Partial -> RANK_ATTENTION
                Verdict.Broken -> RANK_UNPROTECTED
            }
        }
    }
