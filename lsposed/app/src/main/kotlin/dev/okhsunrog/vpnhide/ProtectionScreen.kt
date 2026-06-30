package dev.okhsunrog.vpnhide

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier

/**
 * The Protection tab. Formerly a segmented Tun / App-hiding / Ports switcher;
 * those three pickers are now one unified list ([AppPickerScreen]) where each
 * app row carries all four role chips (J / N / A / P, optionally full labels) and a single Save writes
 * every backend at once.
 */
@Composable
internal fun ProtectionScreen(
    searchQuery: String,
    showSystem: Boolean,
    showRussianOnly: Boolean,
    sortMode: TargetListSortMode,
    modifier: Modifier = Modifier,
) {
    AppPickerScreen(
        searchQuery = searchQuery,
        showSystem = showSystem,
        showRussianOnly = showRussianOnly,
        sortMode = sortMode,
        modifier = modifier,
    )
}
