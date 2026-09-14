package dev.okhsunrog.vpnhide.help

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withLink
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp

/**
 * Render parsed [MdBlock]s (see [parseMarkdown]) with the app's Material theme.
 * Links carry their raw href to [onLink]; the caller routes the `vpnhide://` /
 * `article:` scheme to in-app navigation and anything else to the browser.
 */
@Composable
internal fun MarkdownText(
    blocks: List<MdBlock>,
    onLink: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(10.dp)) {
        blocks.forEach { block ->
            when (block) {
                is MdBlock.Heading -> {
                    HeadingBlock(block, onLink)
                }

                is MdBlock.Paragraph -> {
                    Text(inlineAnnotated(block.spans, onLink), style = MaterialTheme.typography.bodyMedium)
                }

                is MdBlock.BulletList -> {
                    ListBlock(block.items, onLink, ordered = false)
                }

                is MdBlock.NumberedList -> {
                    ListBlock(block.items, onLink, ordered = true)
                }

                is MdBlock.Quote -> {
                    QuoteBlock(block, onLink)
                }
            }
        }
    }
}

@Composable
private fun HeadingBlock(
    block: MdBlock.Heading,
    onLink: (String) -> Unit,
) {
    val style =
        when (block.level) {
            1 -> MaterialTheme.typography.headlineSmall
            2 -> MaterialTheme.typography.titleMedium
            else -> MaterialTheme.typography.titleSmall
        }
    Text(
        text = inlineAnnotated(block.spans, onLink),
        style = style,
        fontWeight = FontWeight.Bold,
        modifier = Modifier.padding(top = 4.dp),
    )
}

@Composable
private fun ListBlock(
    items: List<List<MdSpan>>,
    onLink: (String) -> Unit,
    ordered: Boolean,
) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        items.forEachIndexed { index, spans ->
            Row(modifier = Modifier.fillMaxWidth()) {
                Text(
                    text = if (ordered) "${index + 1}." else "•",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Spacer(Modifier.width(8.dp))
                Text(inlineAnnotated(spans, onLink), style = MaterialTheme.typography.bodyMedium)
            }
        }
    }
}

@Composable
private fun QuoteBlock(
    block: MdBlock.Quote,
    onLink: (String) -> Unit,
) {
    Surface(
        color = MaterialTheme.colorScheme.surfaceVariant,
        shape = RoundedCornerShape(8.dp),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text(
            text = inlineAnnotated(block.spans, onLink),
            style = MaterialTheme.typography.bodyMedium,
            fontStyle = FontStyle.Italic,
            modifier = Modifier.padding(12.dp),
        )
    }
}

@Composable
private fun inlineAnnotated(
    spans: List<MdSpan>,
    onLink: (String) -> Unit,
): AnnotatedString {
    val linkColor = MaterialTheme.colorScheme.primary
    val codeBackground = MaterialTheme.colorScheme.surfaceVariant
    return buildAnnotatedString {
        spans.forEach { span ->
            when (span) {
                is MdSpan.Text -> {
                    append(span.text)
                }

                is MdSpan.Bold -> {
                    withStyle(SpanStyle(fontWeight = FontWeight.Bold)) { append(span.text) }
                }

                is MdSpan.Italic -> {
                    withStyle(SpanStyle(fontStyle = FontStyle.Italic)) { append(span.text) }
                }

                is MdSpan.Code -> {
                    withStyle(SpanStyle(fontFamily = FontFamily.Monospace, background = codeBackground)) {
                        append(span.text)
                    }
                }

                is MdSpan.Link -> {
                    val styles = TextLinkStyles(SpanStyle(color = linkColor, textDecoration = TextDecoration.Underline))
                    withLink(LinkAnnotation.Clickable(tag = span.href, styles = styles) { onLink(span.href) }) {
                        append(span.text)
                    }
                }
            }
        }
    }
}
