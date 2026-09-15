package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.DiagnosticEligibility
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticGate
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticPresentation
import dev.okhsunrog.vpnhide.diagnostics.EvidenceConclusion
import dev.okhsunrog.vpnhide.diagnostics.MeasurementApplicability
import dev.okhsunrog.vpnhide.diagnostics.RunOutcome

/**
 * A qualifier the hero shows under its title when the diagnostic presentation
 * says the cached tiles are not the whole story. Decided purely here, worded in
 * the screen. Notes that downgrade a Protected hero to Attention are the ones
 * that make a positive claim unsafe; ResultsUnverified only names a refresh in
 * flight and keeps the previous status, so a routine refresh does not flicker.
 */
internal enum class HeroNote(
    val downgrades: Boolean,
    /** Names a current condition that explains why nothing could be measured. */
    val explainsCondition: Boolean = false,
) {
    None(false),

    /** The probe helper of an earlier run has not returned, so nothing can be measured at all. */
    ProbeUnavailable(true, explainsCondition = true),
    Applying(true, explainsCondition = true),
    ApplicationUnknown(true, explainsCondition = true),
    ApplicationFailed(true, explainsCondition = true),
    RoutingUnknown(true, explainsCondition = true),
    Checking(true, explainsCondition = true),
    Interrupted(true),
    Failed(true),
    ResultsChanged(true),
    ResultsUnverified(false),
    InsufficientEvidence(true),
}

internal data class HeroDecision(
    val status: HeroStatus,
    val note: HeroNote,
    /**
     * Whether the "diagnostics failed, retry" prompt belongs under the hero. A run
     * that never started because of a current condition is not an execution
     * failure (I13): the note already names the condition, so the prompt would
     * contradict it in the same frame.
     */
    val showsFailedPrompt: Boolean,
)

/**
 * The cached protection tiles overlaid with the current eligibility: a blocking
 * condition always wins over whatever was measured earlier, so the hero and the
 * prompt under it react to a VPN toggle without a Dashboard refresh.
 */
internal fun effectiveProtection(
    protection: ProtectionCheck,
    eligibility: DiagnosticEligibility,
): ProtectionCheck =
    when (eligibility) {
        DiagnosticEligibility.RestartApp, DiagnosticEligibility.RestartDevice -> ProtectionCheck.Blocked(DiagnosticGate.NEEDS_RESTART)
        DiagnosticEligibility.VpnOff -> ProtectionCheck.Blocked(DiagnosticGate.VPN_OFF)
        DiagnosticEligibility.SelfExcluded -> ProtectionCheck.Blocked(DiagnosticGate.SELF_NOT_ROUTED)
        else -> protection
    }

internal fun heroDecision(
    state: DashboardState,
    presentation: DiagnosticPresentation,
    errorCount: Int,
    warningCount: Int,
): HeroDecision {
    val protection = effectiveProtection(state.protection, presentation.eligibility)
    val base = computeHeroStatus(state.copy(protection = protection), errorCount, warningCount)
    val note = heroNote(presentation)
    val status = if (base == HeroStatus.Protected && note.downgrades) HeroStatus.Attention else base
    return HeroDecision(status, note, showsFailedPrompt = protection is ProtectionCheck.Failed && !note.explainsCondition)
}

/** The process first, then current conditions, then the latest attempt, then sufficiency, then applicability. */
private fun heroNote(presentation: DiagnosticPresentation): HeroNote {
    // A quarantined probe cannot measure whatever the conditions are, so it is
    // named before the eligibility that would otherwise promise a re-check.
    if (presentation.probeUnavailable) return HeroNote.ProbeUnavailable
    conditionNote(presentation)?.let { return it }
    val attempt = presentation.lastAttempt
    if (presentation.measurement != null && attempt != null && attempt.outcome != RunOutcome.Completed) {
        return if (attempt.outcome == RunOutcome.Interrupted) HeroNote.Interrupted else HeroNote.Failed
    }
    if (presentation.evidence?.conclusion == EvidenceConclusion.Insufficient) return HeroNote.InsufficientEvidence
    return when (presentation.applicability) {
        MeasurementApplicability.Changed -> HeroNote.ResultsChanged
        MeasurementApplicability.Unverified -> HeroNote.ResultsUnverified
        MeasurementApplicability.Absent, MeasurementApplicability.MatchesLastObservation -> HeroNote.None
    }
}

private fun conditionNote(presentation: DiagnosticPresentation): HeroNote? =
    when (presentation.eligibility) {
        DiagnosticEligibility.Applying -> {
            HeroNote.Applying
        }

        DiagnosticEligibility.ApplicationUnknown -> {
            HeroNote.ApplicationUnknown
        }

        DiagnosticEligibility.ApplicationFailed -> {
            HeroNote.ApplicationFailed
        }

        DiagnosticEligibility.Unknown -> {
            HeroNote.RoutingUnknown
        }

        DiagnosticEligibility.Initializing, DiagnosticEligibility.Checking -> {
            if (presentation.measurement == null) HeroNote.Checking else null
        }

        // Blocked conditions have their own prompt under the hero; the generic subtitle stays.
        DiagnosticEligibility.RestartApp, DiagnosticEligibility.RestartDevice, DiagnosticEligibility.VpnOff,
        DiagnosticEligibility.SelfExcluded,
        -> {
            HeroNote.None
        }

        DiagnosticEligibility.Eligible -> {
            null
        }
    }
