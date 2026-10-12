package dev.okhsunrog.vpnhide.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.okhsunrog.vpnhide.ui.theme.AppColors

/**
 * 58dp circular icon bubble used in the Dashboard and Statistics hero cards,
 * which each used to carry their own copy (one with the icon as a parameter,
 * the other hard-coding it). With [progress] the bubble spins instead of naming
 * a state — the hero uses it while a read or a run is in flight, so the "we are
 * still looking" signal sits where the state icon would be, not beside it.
 */
@Composable
fun IconBubble(
    icon: ImageVector,
    tint: Color,
    container: Color,
    modifier: Modifier = Modifier,
    progress: Boolean = false,
) {
    Box(
        modifier =
            modifier
                .size(58.dp)
                .clip(CircleShape)
                .background(container),
        contentAlignment = Alignment.Center,
    ) {
        if (progress) {
            CircularProgressIndicator(
                modifier = Modifier.size(28.dp),
                color = tint,
                strokeWidth = 3.dp,
            )
        } else {
            Icon(
                imageVector = icon,
                contentDescription = null,
                tint = tint,
                modifier = Modifier.size(31.dp),
            )
        }
    }
}

/**
 * label-over-value metric tile used in the Dashboard and Statistics hero cards.
 * The two screens previously kept near-identical copies that had already drifted
 * (3dp vs 4dp label/value gap); this is the single source.
 */
@Composable
fun MetricTile(
    label: String,
    value: String,
    accent: Color,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier =
            modifier
                .clip(MaterialTheme.shapes.medium)
                .background(AppColors.cardContainerStrong)
                .padding(horizontal = 12.dp, vertical = 10.dp),
    ) {
        Text(
            text = label,
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(4.dp))
        Text(
            text = value,
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.Bold,
            color = accent,
        )
    }
}

internal data class MetricContent(
    val label: String,
    val value: String,
    val accent: Color,
)

/** Keep a consistent grid: all pairs use two columns, or every metric gets a full row. */
@Composable
internal fun MetricGrid(vararg metrics: MetricContent) {
    val width =
        metrics.maxOfOrNull {
            maxOf(
                unwrappedTextWidth(it.label, MaterialTheme.typography.labelMedium),
                unwrappedTextWidth(it.value, MaterialTheme.typography.titleMedium.copy(fontWeight = FontWeight.Bold)),
            ) + 24.dp
        } ?: 0.dp
    BoxWithConstraints(Modifier.fillMaxWidth()) {
        val columns = if (width * 2 + 8.dp <= maxWidth) 2 else 1
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            metrics.toList().chunked(columns).forEach { row ->
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    row.forEach { metric ->
                        MetricTile(metric.label, metric.value, metric.accent, Modifier.weight(1f))
                    }
                }
            }
        }
    }
}

@Composable
internal fun ModuleHeading(
    name: String,
    version: String?,
) {
    val titleStyle = MaterialTheme.typography.titleSmall.copy(fontWeight = FontWeight.Bold)
    if (version == null) {
        Text(name, style = titleStyle)
        return
    }
    AdaptivePair(
        firstWidth = unwrappedTextWidth(name, titleStyle),
        secondWidth = unwrappedTextWidth(version, MaterialTheme.typography.bodySmall),
        modifier = Modifier.fillMaxWidth(),
        first = { Text(name, modifier = it, style = titleStyle) },
        second = {
            Text(
                version,
                modifier = it,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        },
    )
}
