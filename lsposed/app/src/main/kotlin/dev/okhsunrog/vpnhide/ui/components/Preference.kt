package dev.okhsunrog.vpnhide.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.ripple
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp
import dev.okhsunrog.vpnhide.ui.theme.groupCornerRadii
import dev.okhsunrog.vpnhide.ui.theme.groupedShape
import dev.okhsunrog.vpnhide.ui.theme.shapeByInteraction

/**
 * A settings row: optional leading icon chip, title + optional subtitle, and an
 * optional trailing slot, on the shared [container] surface. Adapted from
 * ImageToolbox's `PreferenceRow` (Apache-2.0, © T8RIN).
 *
 * Pass [index]/[count] to make the row part of a vertical group (grouped
 * corners that morph on press); leave them defaulted for a standalone row.
 */
@Composable
fun PreferenceRow(
    title: String,
    modifier: Modifier = Modifier,
    subtitle: String? = null,
    icon: ImageVector? = null,
    enabled: Boolean = true,
    index: Int = -1,
    count: Int = 1,
    onClick: (() -> Unit)? = null,
    trailing: @Composable (() -> Unit)? = null,
) {
    val effectiveIndex = if (index < 0) 0 else index
    val effectiveCount = if (index < 0) 1 else count
    val interaction = remember { MutableInteractionSource() }
    val base = groupCornerRadii(effectiveIndex, effectiveCount)
    val shape =
        if (onClick != null) {
            shapeByInteraction(base, interaction)
        } else {
            groupedShape(effectiveIndex, effectiveCount)
        }
    val tick = rememberHapticTick()
    Row(
        modifier =
            modifier
                .fillMaxWidth()
                .container(shape = shape)
                .then(
                    if (onClick != null) {
                        Modifier.clickable(
                            interactionSource = interaction,
                            indication = ripple(),
                            enabled = enabled,
                        ) {
                            tick()
                            onClick()
                        }
                    } else {
                        Modifier
                    },
                ).padding(horizontal = 16.dp, vertical = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (icon != null) {
            Box(
                modifier =
                    Modifier
                        .size(40.dp)
                        .clip(MaterialTheme.shapes.medium)
                        .background(MaterialTheme.colorScheme.secondaryContainer),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    imageVector = icon,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.onSecondaryContainer,
                    modifier = Modifier.size(22.dp),
                )
            }
            Spacer(Modifier.width(16.dp))
        }
        Column(modifier = Modifier.weight(1f)) {
            Text(text = title, style = MaterialTheme.typography.titleMedium)
            if (subtitle != null) {
                Text(
                    text = subtitle,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        if (trailing != null) {
            Spacer(Modifier.width(12.dp))
            trailing()
        }
    }
}

/**
 * A [PreferenceRow] whose trailing control is an [EnhancedSwitch]; tapping
 * anywhere on the row toggles it.
 */
@Composable
fun PreferenceRowSwitch(
    title: String,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
    subtitle: String? = null,
    icon: ImageVector? = null,
    enabled: Boolean = true,
    index: Int = -1,
    count: Int = 1,
    progress: Boolean = false,
    onDisabledClick: (() -> Unit)? = null,
) {
    PreferenceRow(
        title = title,
        modifier = modifier,
        subtitle = subtitle,
        icon = icon,
        enabled = enabled || onDisabledClick != null,
        index = index,
        count = count,
        onClick = { if (enabled) onCheckedChange(!checked) else onDisabledClick?.invoke() },
        trailing = {
            EnhancedSwitch(
                checked = checked,
                onCheckedChange = null,
                enabled = enabled,
                progress = progress,
            )
        },
    )
}
