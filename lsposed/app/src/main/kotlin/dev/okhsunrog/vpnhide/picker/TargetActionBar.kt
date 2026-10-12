package dev.okhsunrog.vpnhide.picker

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.dp
import dev.okhsunrog.vpnhide.R
import dev.okhsunrog.vpnhide.ui.components.AdaptivePair
import dev.okhsunrog.vpnhide.ui.components.EnhancedButton
import dev.okhsunrog.vpnhide.ui.components.unwrappedTextWidth

/** The real picker footer, also used by the layout fixture. No configuration writes live here. */
@Composable
internal fun TargetActionBar(
    count: String,
    canDiscard: Boolean,
    canSave: Boolean,
    saving: Boolean,
    onDiscard: () -> Unit,
    onSave: () -> Unit,
    modifier: Modifier = Modifier,
    supportingText: String? = null,
) {
    val discard = stringResource(R.string.config_discard_draft)
    val save = stringResource(R.string.btn_save)
    val direction = LocalLayoutDirection.current
    val discardPadding = ButtonDefaults.TextButtonContentPadding
    val savePadding = ButtonDefaults.ContentPadding
    val discardWidth =
        maxOf(
            ButtonDefaults.MinWidth,
            unwrappedTextWidth(discard, MaterialTheme.typography.labelLarge) + discardPadding.calculateLeftPadding(direction) +
                discardPadding.calculateRightPadding(direction),
        )
    val saveWidth =
        maxOf(
            ButtonDefaults.MinWidth,
            unwrappedTextWidth(save, MaterialTheme.typography.labelLarge) + savePadding.calculateLeftPadding(direction) +
                savePadding.calculateRightPadding(direction) +
                if (saving) 16.dp else 0.dp,
        )
    val measure = rememberTextMeasurer()
    val density = LocalDensity.current
    BoxWithConstraints(modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp)) {
        val remaining = with(density) { (maxWidth - discardWidth - saveWidth).roundToPx() }.coerceAtLeast(0)
        val countLayout =
            measure.measure(
                count,
                MaterialTheme.typography.bodyMedium,
                constraints = Constraints(maxWidth = remaining),
                maxLines = 2,
            )
        val supportingFits =
            supportingText == null ||
                !measure
                    .measure(
                        supportingText,
                        MaterialTheme.typography.bodySmall,
                        constraints = Constraints(maxWidth = remaining),
                        maxLines = 2,
                    ).hasVisualOverflow
        val fits = remaining > 0 && !countLayout.hasVisualOverflow && supportingFits
        val counter: @Composable (Modifier) -> Unit = { textModifier ->
            Column(textModifier) {
                Text(count, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                supportingText?.let {
                    Text(
                        it,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }
        val discardButton: @Composable (Modifier) -> Unit = { buttonModifier ->
            TextButton(
                onClick = onDiscard,
                enabled = canDiscard,
                modifier = buttonModifier,
                contentPadding = discardPadding,
            ) { Text(discard) }
        }
        val saveButton: @Composable (Modifier) -> Unit = { buttonModifier ->
            EnhancedButton(onClick = onSave, enabled = canSave, modifier = buttonModifier, contentPadding = savePadding) {
                if (saving) CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
                Text(save)
            }
        }
        if (fits) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                counter(Modifier.weight(1f))
                discardButton(Modifier)
                saveButton(Modifier)
            }
        } else {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                counter(Modifier.fillMaxWidth())
                AdaptivePair(discardWidth, saveWidth, Modifier.fillMaxWidth(), first = discardButton, second = saveButton)
            }
        }
    }
}
