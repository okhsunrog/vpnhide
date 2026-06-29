package dev.okhsunrog.vpnhide.settings

import android.content.Context
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.floatPreferencesKey
import androidx.datastore.preferences.core.intPreferencesKey
import androidx.datastore.preferences.core.longPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.map

/**
 * Single DataStore for all UI/appearance preferences.
 *
 * Lives separately from [dev.okhsunrog.vpnhide.DebugLoggingPrefs] (which gates a
 * stealth-sensitive runtime flag and is read off the cold-start critical path):
 * these are purely cosmetic and safe to observe reactively.
 */
private val Context.uiSettingsStore: DataStore<Preferences> by preferencesDataStore(name = "ui_settings")

class SettingsRepository(
    private val context: Context,
) {
    private object Keys {
        val DYNAMIC_COLOR = booleanPreferencesKey("dynamic_color")
        val AMOLED = booleanPreferencesKey("amoled")
        val CONTRAST = floatPreferencesKey("contrast")
        val SEED = longPreferencesKey("seed_color")
        val CORNER_STYLE = intPreferencesKey("corner_style")
        val THEME_MODE = intPreferencesKey("theme_mode")
        val ANIMATIONS = booleanPreferencesKey("animations_enabled")
        val HAPTICS = booleanPreferencesKey("haptics_enabled")
        val FULL_PROTECTION_ROLE_LABELS = booleanPreferencesKey("full_protection_role_labels")
        val AGENT_CONTROL = booleanPreferencesKey("agent_control_enabled")
        val SETTINGS_HINT_SEEN = booleanPreferencesKey("settings_hint_seen")
        val SUPPRESS_VERSION_WARNINGS = booleanPreferencesKey("suppress_version_warnings")
    }

    val settings: Flow<AppSettings> =
        context.uiSettingsStore.data.map { p ->
            val defaults = AppSettings()
            AppSettings(
                dynamicColor = p[Keys.DYNAMIC_COLOR] ?: defaults.dynamicColor,
                amoled = p[Keys.AMOLED] ?: defaults.amoled,
                contrast = p[Keys.CONTRAST] ?: defaults.contrast,
                seedColor = p[Keys.SEED] ?: defaults.seedColor,
                cornerStyle = p[Keys.CORNER_STYLE]?.let { CornerStyle.entries.getOrNull(it) } ?: defaults.cornerStyle,
                themeMode = p[Keys.THEME_MODE]?.let { ThemeMode.entries.getOrNull(it) } ?: defaults.themeMode,
                animationsEnabled = p[Keys.ANIMATIONS] ?: defaults.animationsEnabled,
                hapticsEnabled = p[Keys.HAPTICS] ?: defaults.hapticsEnabled,
                fullProtectionRoleLabels =
                    p[Keys.FULL_PROTECTION_ROLE_LABELS] ?: defaults.fullProtectionRoleLabels,
                agentControlEnabled = p[Keys.AGENT_CONTROL] ?: defaults.agentControlEnabled,
                settingsHintSeen = p[Keys.SETTINGS_HINT_SEEN] ?: defaults.settingsHintSeen,
                suppressVersionWarnings =
                    p[Keys.SUPPRESS_VERSION_WARNINGS] ?: defaults.suppressVersionWarnings,
            )
        }

    suspend fun setDynamicColor(value: Boolean) = edit { it[Keys.DYNAMIC_COLOR] = value }

    suspend fun setAmoled(value: Boolean) = edit { it[Keys.AMOLED] = value }

    suspend fun setContrast(value: Float) = edit { it[Keys.CONTRAST] = value }

    suspend fun setSeedColor(value: Long) = edit { it[Keys.SEED] = value }

    suspend fun setCornerStyle(value: CornerStyle) = edit { it[Keys.CORNER_STYLE] = value.ordinal }

    suspend fun setThemeMode(value: ThemeMode) = edit { it[Keys.THEME_MODE] = value.ordinal }

    suspend fun setAnimationsEnabled(value: Boolean) = edit { it[Keys.ANIMATIONS] = value }

    suspend fun setHapticsEnabled(value: Boolean) = edit { it[Keys.HAPTICS] = value }

    suspend fun setFullProtectionRoleLabels(value: Boolean) = edit { it[Keys.FULL_PROTECTION_ROLE_LABELS] = value }

    suspend fun setAgentControlEnabled(value: Boolean) = edit { it[Keys.AGENT_CONTROL] = value }

    suspend fun setSettingsHintSeen(value: Boolean) = edit { it[Keys.SETTINGS_HINT_SEEN] = value }

    suspend fun setSuppressVersionWarnings(value: Boolean) = edit { it[Keys.SUPPRESS_VERSION_WARNINGS] = value }

    private suspend fun edit(block: (androidx.datastore.preferences.core.MutablePreferences) -> Unit) {
        context.uiSettingsStore.edit(block)
    }
}
