package dev.okhsunrog.vpnhide.ui.components

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonColors
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonColors
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Switch
import androidx.compose.material3.contentColorFor
import androidx.compose.material3.ripple
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.takeOrElse
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.okhsunrog.vpnhide.ui.theme.AppColors
import dev.okhsunrog.vpnhide.ui.theme.AppMotion
import dev.okhsunrog.vpnhide.ui.theme.groupCornerRadii
import dev.okhsunrog.vpnhide.ui.theme.groupedShape
import dev.okhsunrog.vpnhide.ui.theme.shapeByInteraction

/*
 * Enhanced Material 3 controls: a light haptic tick on activation plus a spring
 * press-scale, for a tactile, expressive feel. Adapted from ImageToolbox's
 * EnhancedButton / EnhancedSwitch (Apache-2.0, © T8RIN).
 */

/**
 * A drop-in replacement for [androidx.compose.material3.Card] built on the
 * shared [dev.okhsunrog.vpnhide.ui.components.container] look (soft shadow,
 * theme shape, hairline border). Content keeps its own padding, like Card.
 */
@Composable
fun EnhancedCard(
    modifier: Modifier = Modifier,
    shape: Shape = MaterialTheme.shapes.large,
    color: Color = AppColors.cardContainer,
    contentColor: Color = contentColorFor(color).takeOrElse { MaterialTheme.colorScheme.onSurface },
    onClick: (() -> Unit)? = null,
    content: @Composable ColumnScope.() -> Unit,
) {
    CompositionLocalProvider(LocalContentColor provides contentColor) {
        Column(
            modifier =
                modifier
                    .container(shape = shape, color = color)
                    .then(if (onClick != null) Modifier.hapticsClickable { onClick() } else Modifier),
            content = content,
        )
    }
}

/**
 * An [EnhancedCard] that participates in a vertical group: it takes the grouped
 * shape for its [index] of [count] (big outer corners on the group's edges,
 * small inner corners) so a stack of these reads as one unit. When [onClick] is
 * set the corners morph on press (`shapeByInteraction`); otherwise they morph as
 * group membership changes. This is the signature ImageToolbox grouped look —
 * place several in a `Column` with a small spacing (≈3.dp).
 */
@Composable
fun GroupedCard(
    index: Int,
    count: Int,
    modifier: Modifier = Modifier,
    color: Color = AppColors.cardContainer,
    contentColor: Color = contentColorFor(color).takeOrElse { MaterialTheme.colorScheme.onSurface },
    onClick: (() -> Unit)? = null,
    content: @Composable ColumnScope.() -> Unit,
) {
    val interaction = remember { MutableInteractionSource() }
    val base = groupCornerRadii(index, count)
    val shape =
        if (onClick != null) {
            shapeByInteraction(base, interaction)
        } else {
            groupedShape(index, count)
        }
    val tick = rememberHapticTick()
    CompositionLocalProvider(LocalContentColor provides contentColor) {
        Column(
            modifier =
                modifier
                    .container(shape = shape, color = color)
                    .then(
                        if (onClick != null) {
                            Modifier.clickable(
                                interactionSource = interaction,
                                indication = ripple(),
                            ) {
                                tick()
                                onClick()
                            }
                        } else {
                            Modifier
                        },
                    ),
            content = content,
        )
    }
}

@Composable
fun EnhancedButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    colors: ButtonColors = ButtonDefaults.buttonColors(),
    shape: Shape = MaterialTheme.shapes.large,
    contentPadding: PaddingValues = ButtonDefaults.ContentPadding,
    content: @Composable RowScope.() -> Unit,
) {
    val interactionSource = remember { MutableInteractionSource() }
    val pressed by interactionSource.collectIsPressedAsState()
    val scale by animateFloatAsState(if (pressed) 0.96f else 1f, AppMotion.fastSpatial())
    val tick = rememberHapticTick()
    Button(
        onClick = {
            tick()
            onClick()
        },
        modifier =
            modifier.graphicsLayer {
                scaleX = scale
                scaleY = scale
            },
        enabled = enabled,
        colors = colors,
        shape = shape,
        interactionSource = interactionSource,
        contentPadding = contentPadding,
        content = content,
    )
}

@Composable
fun EnhancedOutlinedButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    shape: Shape = MaterialTheme.shapes.large,
    contentPadding: PaddingValues = ButtonDefaults.ContentPadding,
    content: @Composable RowScope.() -> Unit,
) {
    val interactionSource = remember { MutableInteractionSource() }
    val pressed by interactionSource.collectIsPressedAsState()
    val scale by animateFloatAsState(if (pressed) 0.96f else 1f, AppMotion.fastSpatial())
    val tick = rememberHapticTick()
    OutlinedButton(
        onClick = {
            tick()
            onClick()
        },
        modifier =
            modifier.graphicsLayer {
                scaleX = scale
                scaleY = scale
            },
        enabled = enabled,
        shape = shape,
        interactionSource = interactionSource,
        contentPadding = contentPadding,
        content = content,
    )
}

@Composable
fun EnhancedIconButton(
    onClick: () -> Unit,
    icon: ImageVector,
    contentDescription: String?,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    colors: IconButtonColors = IconButtonDefaults.iconButtonColors(),
) {
    val interactionSource = remember { MutableInteractionSource() }
    val pressed by interactionSource.collectIsPressedAsState()
    val scale by animateFloatAsState(if (pressed) 0.88f else 1f, AppMotion.fastSpatial())
    val tick = rememberHapticTick()
    IconButton(
        onClick = {
            tick()
            onClick()
        },
        modifier =
            modifier.graphicsLayer {
                scaleX = scale
                scaleY = scale
            },
        enabled = enabled,
        colors = colors,
        interactionSource = interactionSource,
    ) {
        Icon(imageVector = icon, contentDescription = contentDescription)
    }
}

/**
 * A small circular progress spinner sized for inline use inside a button or a
 * compact status row (the `strokeWidth = 2.dp` look repeated across the app).
 * [color] defaults to [LocalContentColor], so inside a Material button it inherits
 * the button's content color automatically — pass it explicitly only outside one.
 */
@Composable
fun ButtonSpinner(
    modifier: Modifier = Modifier,
    size: Dp = 18.dp,
    color: Color = LocalContentColor.current,
) {
    CircularProgressIndicator(
        modifier = modifier.size(size),
        strokeWidth = 2.dp,
        color = color,
    )
}

@Composable
fun EnhancedSwitch(
    checked: Boolean,
    onCheckedChange: ((Boolean) -> Unit)?,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    progress: Boolean = false,
) {
    val tick = rememberHapticTick()
    Switch(
        checked = checked,
        onCheckedChange =
            onCheckedChange?.let { cb ->
                { value ->
                    tick()
                    cb(value)
                }
            },
        modifier = modifier,
        enabled = enabled,
        thumbContent =
            if (progress) {
                { ButtonSpinner(size = 16.dp, color = MaterialTheme.colorScheme.onSurface) }
            } else {
                null
            },
    )
}
