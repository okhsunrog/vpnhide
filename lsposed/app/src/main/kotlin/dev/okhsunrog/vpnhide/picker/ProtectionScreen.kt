package dev.okhsunrog.vpnhide.picker

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier

/**
 * The Apps tab. Each app row carries all four role chips
 * (J / N / A / P, optionally full labels), and a single Save writes every
 * backend at once.
 */
@Composable
internal fun ProtectionScreen(
    searchQuery: String,
    showSystem: Boolean,
    showRussianOnly: Boolean,
    sortMode: TargetListSortMode,
    onToggleSystem: () -> Unit,
    onToggleRussianOnly: () -> Unit,
    onSortModeChange: (TargetListSortMode) -> Unit,
    modifier: Modifier = Modifier,
) {
    AppPickerScreen(
        searchQuery = searchQuery,
        showSystem = showSystem,
        showRussianOnly = showRussianOnly,
        sortMode = sortMode,
        onToggleSystem = onToggleSystem,
        onToggleRussianOnly = onToggleRussianOnly,
        onSortModeChange = onSortModeChange,
        modifier = modifier,
    )
}
