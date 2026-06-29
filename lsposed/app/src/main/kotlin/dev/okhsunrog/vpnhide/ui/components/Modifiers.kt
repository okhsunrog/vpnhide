package dev.okhsunrog.vpnhide.ui.components

import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.okhsunrog.vpnhide.settings.LocalSettingsState
import dev.okhsunrog.vpnhide.ui.theme.AppColors
import dev.okhsunrog.vpnhide.ui.theme.AppEasing

/*
 * Shared visual + interaction modifiers for the design system.
 *
 * Adapted from ImageToolbox's core/ui/widget/modifier (Apache-2.0, © T8RIN):
 * a single container look (soft shadow + shape + subtle border) applied
 * everywhere for consistency, plus haptic-aware click handling.
 */

/**
 * Unified surface styling: a soft elevation shadow, a filled background and an
 * optional hairline border, all clipped to [shape]. Use on every card/row so
 * surfaces read as one coherent system.
 */
@Composable
fun Modifier.container(
    shape: Shape = MaterialTheme.shapes.large,
    color: Color = AppColors.cardContainer,
    borderColor: Color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.35f),
    drawBorder: Boolean = true,
    shadowElevation: Dp = 1.dp,
): Modifier =
    this
        .shadow(elevation = shadowElevation, shape = shape, clip = false)
        // clip before background so a following clickable's ripple is shape-clipped
        .clip(shape)
        .background(color = color)
        .then(if (drawBorder) Modifier.border(width = 1.dp, color = borderColor, shape = shape) else Modifier)

/**
 * Clickable that fires a light haptic tick before [onClick] when haptics are
 * enabled in settings. The ripple/indication comes from the standard
 * [clickable].
 */
@Composable
fun Modifier.hapticsClickable(
    enabled: Boolean = true,
    onClick: () -> Unit,
): Modifier {
    val tick = rememberHapticTick()
    return this.clickable(enabled = enabled) {
        tick()
        onClick()
    }
}

/**
 * A reusable haptic "tick" bound to the current settings + haptic feedback.
 * Fires a light feedback when haptics are enabled; no-op otherwise. Call the
 * returned lambda from any onClick (buttons, switches, nav items).
 */
@Composable
fun rememberHapticTick(): () -> Unit {
    val haptics = LocalHapticFeedback.current
    val enabled = LocalSettingsState.current.hapticsEnabled
    return remember(haptics, enabled) {
        { if (enabled) haptics.performHapticFeedback(HapticFeedbackType.TextHandleMove) }
    }
}

/**
 * A gentle, looping scale "breathing" used to draw attention to a control
 * (e.g. the settings gear on first launch). No-op when [enabled] is false.
 * Adapted from ImageToolbox's `Pulsate` modifier (Apache-2.0, © T8RIN).
 */
@Composable
fun Modifier.pulse(
    enabled: Boolean,
    min: Float = 0.9f,
    max: Float = 1.12f,
    durationMillis: Int = 900,
): Modifier {
    if (!enabled) return this
    val transition = rememberInfiniteTransition(label = "pulse")
    val scale by transition.animateFloat(
        initialValue = min,
        targetValue = max,
        animationSpec =
            infiniteRepeatable(
                animation = tween(durationMillis, easing = AppEasing.Alpha),
                repeatMode = RepeatMode.Reverse,
            ),
        label = "pulseScale",
    )
    return this.graphicsLayer {
        scaleX = scale
        scaleY = scale
    }
}

/** Tracks the pressed state of [interactionSource] (for press-scale effects). */
@Composable
fun rememberIsPressed(interactionSource: MutableInteractionSource): Boolean {
    val pressed by interactionSource.collectIsPressedAsState()
    return pressed
}

/** A medium rounded shape constant for ad-hoc use where the theme scale doesn't fit. */
val DefaultRowShape: Shape = RoundedCornerShape(20.dp)
