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
    /** Draw soft elevation shadows under cards/surfaces (off = flat, bordered). */
    val drawContainerShadows: Boolean = true,
    /** Master switch for the new expressive motion (springs, transitions). */
    val animationsEnabled: Boolean = true,
    /** Subtle haptic feedback on taps/toggles. */
    val hapticsEnabled: Boolean = true,
    /** Use full role labels in the unified Protection picker instead of single-letter chips. */
    val fullProtectionRoleLabels: Boolean = true,
    /** Expose UI-equivalent state/control through the debug host bridge for agent-driven development. */
    val agentControlEnabled: Boolean = false,
    /** Whether the user has opened Settings at least once (gates the gear hint). */
    val settingsHintSeen: Boolean = false,
    /**
     * Suppress the LSPosed running-vs-installed version warning by comparing only
     * the release base version (ignoring the git-describe dev suffix). Off by
     * default, so a stale dev build surfaces; a developer who reinstalls the APK
     * repeatedly without rebooting can flip this to silence the reminder. No
     * effect on release builds (their versions carry no dev suffix).
     */
    val suppressVersionWarnings: Boolean = false,
) {
    companion object {
        /** Brand seed — a crisp blue-teal, used as the non-dynamic fallback palette. */
        const val DEFAULT_SEED: Long = 0xFF0B6F7B
    }
}
