package dev.okhsunrog.vpnhide.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
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
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.okhsunrog.vpnhide.ui.theme.AppColors

/**
 * 58dp circular icon bubble used in the Dashboard and Statistics hero cards,
 * which each used to carry their own copy (one with the icon as a parameter,
 * the other hard-coding it).
 */
@Composable
fun IconBubble(
    icon: ImageVector,
    tint: Color,
    container: Color,
    modifier: Modifier = Modifier,
) {
    Box(
        modifier =
            modifier
                .size(58.dp)
                .clip(CircleShape)
                .background(container),
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            imageVector = icon,
            contentDescription = null,
            tint = tint,
            modifier = Modifier.size(31.dp),
        )
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
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Spacer(Modifier.height(4.dp))
        Text(
            text = value,
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.Bold,
            color = accent,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}
