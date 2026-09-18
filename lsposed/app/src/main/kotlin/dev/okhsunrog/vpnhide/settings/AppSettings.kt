package dev.okhsunrog.vpnhide.settings

import androidx.compose.runtime.Immutable

/** Corner treatment applied across the design system's shapes. */
enum class CornerStyle {
    /** Plain Material 3 rounded corners. */
    Rounded,

    /** iOS-style continuous ("squircle") corners. */
    Smooth,
}

/** Which dark/light mode the UI follows. */
enum class ThemeMode {
    System,
    Light,
    Dark,
}

/**
 * Immutable snapshot of every user-tunable UI preference.
 *
 * This is the single value carried by [LocalSettingsState] so that the whole
 * Compose tree recomposes coherently when any one knob changes. Backed by
 * DataStore via [SettingsRepository]; never mutate in place — emit a new copy.
 */
@Immutable
data class AppSettings(
    /** Use the wallpaper-derived Material You palette on Android 12+. */
    val dynamicColor: Boolean = true,
    /** Pure-black surfaces for OLED panels (only meaningful in dark mode). */
    val amoled: Boolean = false,
    /** Material 3 contrast level, -1f..1f (0 = standard). */
    val contrast: Float = 0f,
    /** Seed color (ARGB) used when [dynamicColor] is off or unavailable. */
    val seedColor: Long = DEFAULT_SEED,
    val cornerStyle: CornerStyle = CornerStyle.Smooth,
    val themeMode: ThemeMode = ThemeMode.System,
    /** Master switch for the new expressive motion (springs, transitions). */
    val animationsEnabled: Boolean = true,
    /** Subtle haptic feedback on taps/toggles. */
    val hapticsEnabled: Boolean = true,
    /** Use full role labels in the unified Apps picker instead of single-letter chips. */
    val fullProtectionRoleLabels: Boolean = true,
    /** Whether the user has explicitly accepted or declined daily background update checks. */
    val backgroundUpdateChecksConfigured: Boolean = false,
    /** Check GitHub Releases in the background and notify when a newer APK is available. */
    val backgroundUpdateChecksEnabled: Boolean = false,
    /** Expose UI-equivalent state/control through the debug host bridge for agent-driven development. */
    val agentControlEnabled: Boolean = false,
    /** Whether the user has opened Settings at least once (gates the gear hint). */
    val settingsHintSeen: Boolean = false,
    /**
     * Don't open the changelog on the first launch after the version changes.
     * For someone who reinstalls their own build several times a day the dialog
     * is pure interruption.
     */
    val suppressChangelog: Boolean = false,
    /**
     * Drop every version-skew notice from the Dashboard: a module older or newer
     * than the app, and the LSPosed hook running a different build than the
     * installed APK. Off by default, because for a normal install each of those
     * is a real "reflash this / reboot" instruction. A developer whose modules
     * and APK are deliberately out of step knows that already and only sees
     * noise, so this hides the lot rather than only the git-describe suffix.
     * The versions themselves stay on the module cards either way.
     */
    val ignoreVersionMismatch: Boolean = false,
    /**
     * The donation prompt on the Dashboard has been acted on — either "Support"
     * or "Hide". Set by both, so the banner is shown until the first deliberate
     * reaction and never again.
     */
    val donatePromptDismissed: Boolean = false,
    /**
     * The user declined importing a pre-1.0 config found on disk. The files stay
     * where they are, so Settings → Advanced can still run the import later; this
     * only stops the Dashboard card from asking again.
     */
    val legacyImportDismissed: Boolean = false,
) {
    companion object {
        /** Brand seed — a crisp blue-teal, used as the non-dynamic fallback palette. */
        const val DEFAULT_SEED: Long = 0xFF0B6F7B
    }
}
