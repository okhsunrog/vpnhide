package dev.okhsunrog.vpnhide.ui.components

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import dev.okhsunrog.vpnhide.next

/**
 * The shared "can't proceed" card. Every full-screen blocking state in the app
 * (no root, startup-prep failure, …) renders through this one composable so they
 * stay visually consistent and a redesign updates all of them at once — instead
 * of each gate hand-rolling its own slightly-different `errorContainer` card and
 * one getting forgotten on the next redesign.
 *
 * Styled with the theme's error container, so it reads correctly in both light
 * and dark themes (and under dynamic / amoled) without hardcoded colours.
 *
 * @param icon optional leading icon (tinted to match the card's content colour).
 * @param detail optional raw/technical detail line, shown monospace and dimmed.
 * @param actionLabel + onAction optional button (e.g. Retry / Check again).
 */
@Composable
fun BlockingErrorCard(
    title: String,
    body: String,
    modifier: Modifier = Modifier,
    icon: ImageVector? = null,
    detail: String? = null,
    actionLabel: String? = null,
    onAction: (() -> Unit)? = null,
) {
    EnhancedCard(
        modifier = modifier,
        color = MaterialTheme.colorScheme.errorContainer,
        contentColor = MaterialTheme.colorScheme.onErrorContainer,
    ) {
        Column(
            modifier = Modifier.padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            if (icon != null) {
                Icon(
                    imageVector = icon,
                    contentDescription = null,
                    modifier = Modifier.size(40.dp),
                )
                Spacer(Modifier.height(12.dp))
            }
            Text(
                text = title,
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Bold,
                textAlign = TextAlign.Center,
            )
            Spacer(Modifier.height(12.dp))
            Text(
                text = body,
                style = MaterialTheme.typography.bodyMedium,
                textAlign = TextAlign.Center,
            )
            if (!detail.isNullOrBlank()) {
                Spacer(Modifier.height(10.dp))
                Text(
                    text = detail,
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = FontFamily.Monospace,
                    textAlign = TextAlign.Center,
                    color = LocalContentColor.current.copy(alpha = 0.75f),
                )
            }
            if (actionLabel != null && onAction != null) {
                Spacer(Modifier.height(16.dp))
                EnhancedButton(onClick = onAction) {
                    Text(actionLabel)
                }
            }
        }
    }
}
