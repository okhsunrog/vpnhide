package dev.okhsunrog.vpnhide.ui.components

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight

/**
 * Section title shared across the Dashboard / Statistics / Diagnostics / Settings
 * screens, which each used to carry their own near-identical copy.
 *
 * Defaults match the most common form (Statistics & Diagnostics): bold
 * `titleSmall` in the primary color. [emphasized] switches to `titleMedium`
 * (Dashboard), [bold] off drops the weight (Settings), and [color] / [modifier]
 * cover the remaining per-screen tweaks.
 */
@Composable
fun SectionHeader(
    text: String,
    modifier: Modifier = Modifier,
    color: Color = MaterialTheme.colorScheme.primary,
    emphasized: Boolean = false,
    bold: Boolean = true,
) {
    Text(
        text = text,
        style = if (emphasized) MaterialTheme.typography.titleMedium else MaterialTheme.typography.titleSmall,
        fontWeight = if (bold) FontWeight.Bold else null,
        color = color,
        modifier = modifier,
    )
}
