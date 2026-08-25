package dev.okhsunrog.vpnhide.settings

import androidx.compose.runtime.staticCompositionLocalOf
import dev.okhsunrog.vpnhide.next
import dev.okhsunrog.vpnhide.startup.VpnHideApp
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

/**
 * Write-side façade for the UI/appearance settings.
 *
 * Reads come from [LocalSettingsState] (a live [AppSettings] snapshot); writes
 * go through this interactor, provided via [LocalSettingsInteractor] so any
 * composable — a card, a switch, a dialog — can flip a preference without the
 * [SettingsRepository] being threaded through its signature. Mirrors
 * ImageToolbox's read/write split (`LocalSettingsState` +
 * `SimpleSettingsInteractor`, Apache-2.0, © T8RIN), minus the domain/data
 * layering we don't need for a handful of cosmetic knobs.
 *
 * Methods are fire-and-forget: each launches the suspending repository write on
 * the provided scope; the new value flows back through DataStore into
 * [LocalSettingsState] and recomposes the tree.
 */
interface SettingsInteractor {
    fun setDynamicColor(value: Boolean)

    fun setAmoled(value: Boolean)

    fun setContrast(value: Float)

    fun setSeedColor(value: Long)

    fun setCornerStyle(value: CornerStyle)

    fun setThemeMode(value: ThemeMode)

    fun setAnimationsEnabled(value: Boolean)

    fun setHapticsEnabled(value: Boolean)

    fun setFullProtectionRoleLabels(value: Boolean)

    fun setBackgroundUpdateChecksEnabled(value: Boolean)

    fun setAgentControlEnabled(value: Boolean)

    fun setSettingsHintSeen(value: Boolean)

    fun setSuppressVersionWarnings(value: Boolean)

    fun setDonatePromptDismissed(value: Boolean)

    fun setLegacyImportDismissed(value: Boolean)
}

/**
 * Default [SettingsInteractor] backed by [SettingsRepository], launching each
 * write on [scope] (typically the composition's `rememberCoroutineScope`).
 */
class RepositorySettingsInteractor(
    private val repository: SettingsRepository,
    private val scope: CoroutineScope,
) : SettingsInteractor {
    override fun setDynamicColor(value: Boolean) = launch { repository.setDynamicColor(value) }

    override fun setAmoled(value: Boolean) = launch { repository.setAmoled(value) }

    override fun setContrast(value: Float) = launch { repository.setContrast(value) }

    override fun setSeedColor(value: Long) = launch { repository.setSeedColor(value) }

    override fun setCornerStyle(value: CornerStyle) = launch { repository.setCornerStyle(value) }

    override fun setThemeMode(value: ThemeMode) = launch { repository.setThemeMode(value) }

    override fun setAnimationsEnabled(value: Boolean) = launch { repository.setAnimationsEnabled(value) }

    override fun setHapticsEnabled(value: Boolean) = launch { repository.setHapticsEnabled(value) }

    override fun setFullProtectionRoleLabels(value: Boolean) = launch { repository.setFullProtectionRoleLabels(value) }

    override fun setBackgroundUpdateChecksEnabled(value: Boolean) = launch { repository.setBackgroundUpdateChecksEnabled(value) }

    override fun setAgentControlEnabled(value: Boolean) = launch { repository.setAgentControlEnabled(value) }

    override fun setSettingsHintSeen(value: Boolean) = launch { repository.setSettingsHintSeen(value) }

    override fun setSuppressVersionWarnings(value: Boolean) = launch { repository.setSuppressVersionWarnings(value) }

    override fun setDonatePromptDismissed(value: Boolean) = launch { repository.setDonatePromptDismissed(value) }

    override fun setLegacyImportDismissed(value: Boolean) = launch { repository.setLegacyImportDismissed(value) }

    private inline fun launch(crossinline block: suspend () -> Unit) {
        scope.launch { block() }
    }
}

/**
 * Ambient write access to UI settings. Provided once near the root (see
 * `VpnHideApp`) next to [LocalSettingsState].
 */
val LocalSettingsInteractor =
    staticCompositionLocalOf<SettingsInteractor> { error("SettingsInteractor not provided") }
