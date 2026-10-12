package dev.okhsunrog.vpnhide.help

import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.unit.dp

/** Soft halo plus a bright core; drawing outside the text bounds never changes article layout. */
internal fun Modifier.anchorGlow(
    color: Color,
    fillColor: Color,
    intensity: Float,
): Modifier =
    drawBehind {
        if (intensity <= 0f) return@drawBehind
        val radius = 5.dp.toPx()
        val padding = 3.dp.toPx()
        // Overlapping translucent layers approximate a soft glow on all supported Android versions.
        // No platform blur or offscreen layer that could clip the halo or blur the glyphs.
        for (step in 12 downTo 1) {
            val spread = padding + step.dp.toPx()
            drawRoundRect(
                color = color.copy(alpha = 0.08f * intensity),
                topLeft = Offset(-spread, -spread),
                size = Size(size.width + spread * 2, size.height + spread * 2),
                cornerRadius = CornerRadius(radius + spread),
            )
        }
        drawRoundRect(
            color = fillColor.copy(alpha = intensity),
            topLeft = Offset(-padding, -padding),
            size = Size(size.width + padding * 2, size.height + padding * 2),
            cornerRadius = CornerRadius(radius),
        )
        drawRoundRect(
            color = color.copy(alpha = intensity),
            topLeft = Offset(-padding, -padding),
            size = Size(size.width + padding * 2, size.height + padding * 2),
            cornerRadius = CornerRadius(radius),
            style = Stroke(width = 1.5.dp.toPx()),
        )
    }
