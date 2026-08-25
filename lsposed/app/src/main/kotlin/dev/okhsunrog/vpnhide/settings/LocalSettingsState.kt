package dev.okhsunrog.vpnhide.settings

import androidx.compose.runtime.staticCompositionLocalOf
import dev.okhsunrog.vpnhide.startup.VpnHideApp

/**
 * Ambient access to the current [AppSettings] snapshot.
 *
 * Provided once near the root (see `VpnHideApp`) so any composable — a card, a
 * switch, a shape modifier — can read the live appearance preferences without
 * threading them through every signature. Mirrors ImageToolbox's
 * `LocalSettingsState` pattern.
 */
val LocalSettingsState =
    staticCompositionLocalOf { AppSettings() }
