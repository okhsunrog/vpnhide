package dev.okhsunrog.vpnhide.ui.components

import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/** Measures with the current font resolver and density, including Android's font scaling. */
@Composable
internal fun unwrappedTextWidth(
    text: String,
    style: TextStyle,
): Dp {
    val measurer = rememberTextMeasurer()
    val width = measurer.measure(text, style, softWrap = false).size.width
    return with(LocalDensity.current) { width.toDp() }
}

/** Preserve the weighted first column until its contents need more space. */
@Composable
internal fun AdaptivePair(
    firstWidth: Dp,
    secondWidth: Dp,
    modifier: Modifier = Modifier,
    gap: Dp = 8.dp,
    equalWidth: Boolean = false,
    first: @Composable (Modifier) -> Unit,
    second: @Composable (Modifier) -> Unit,
) {
    BoxWithConstraints(modifier) {
        val required = if (equalWidth) maxOf(firstWidth, secondWidth) * 2 else firstWidth + secondWidth
        if (required + gap <= maxWidth) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                first(Modifier.weight(1f))
                Spacer(Modifier.width(gap))
                second(if (equalWidth) Modifier.weight(1f) else Modifier)
            }
        } else {
            Column {
                first(Modifier)
                Spacer(Modifier.height(gap))
                second(Modifier)
            }
        }
    }
}
