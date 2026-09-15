package dev.okhsunrog.vpnhide.help

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
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
import org.commonmark.ext.gfm.tables.TableBlock
import org.commonmark.ext.gfm.tables.TableHead
import org.commonmark.node.BlockQuote
import org.commonmark.node.BulletList
import org.commonmark.node.Code
import org.commonmark.node.Emphasis
import org.commonmark.node.FencedCodeBlock
import org.commonmark.node.HardLineBreak
import org.commonmark.node.Heading
import org.commonmark.node.Image
import org.commonmark.node.IndentedCodeBlock
import org.commonmark.node.Link
import org.commonmark.node.Node
import org.commonmark.node.OrderedList
import org.commonmark.node.Paragraph
import org.commonmark.node.SoftLineBreak
import org.commonmark.node.StrongEmphasis
import org.commonmark.node.ThematicBreak
import org.commonmark.node.Text as CmText

/**
 * Render a parsed CommonMark [document] (see [parseMarkdown]) with the app's
 * Material theme. Links carry their normalized href to [onLink]; the caller
 * routes `vpnhide://` / `article:` to in-app navigation and the rest to the
 * browser. Soft line breaks join with a space except at a CJK–CJK boundary, so
 * the hard-wrapped Chinese article reads without spurious spaces.
 */
@Composable
internal fun MarkdownText(
    document: Node,
    onLink: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(10.dp)) {
        var child = document.firstChild
        while (child != null) {
            MarkdownBlock(child, onLink)
            child = child.next
        }
    }
}

@Composable
private fun MarkdownBlock(
    node: Node,
    onLink: (String) -> Unit,
) {
    when (node) {
        is Heading -> {
            HeadingBlock(node, onLink)
        }

        is Paragraph -> {
            Text(inlineAnnotated(node, onLink), style = MaterialTheme.typography.bodyMedium)
        }

        is BulletList -> {
            ListBlock(node, onLink, ordered = false)
        }

        is OrderedList -> {
            ListBlock(node, onLink, ordered = true)
        }

        is BlockQuote -> {
            QuoteBlock(node, onLink)
        }

        is FencedCodeBlock -> {
            CodeBlock(node.literal)
        }

        is IndentedCodeBlock -> {
            CodeBlock(node.literal)
        }

        is TableBlock -> {
            TableBlockView(node, onLink)
        }

        is ThematicBreak -> {
            HorizontalDivider()
        }

        else -> {
            var child = node.firstChild
            while (child != null) {
                MarkdownBlock(child, onLink)
                child = child.next
            }
        }
    }
}

@Composable
private fun HeadingBlock(
    node: Heading,
    onLink: (String) -> Unit,
) {
    val style =
        when (node.level) {
            1 -> MaterialTheme.typography.headlineSmall
            2 -> MaterialTheme.typography.titleMedium
            else -> MaterialTheme.typography.titleSmall
        }
    Text(
        text = inlineAnnotated(node, onLink),
        style = style,
        fontWeight = FontWeight.Bold,
        modifier = Modifier.padding(top = 4.dp),
    )
}

@Composable
private fun ListBlock(
    list: Node,
    onLink: (String) -> Unit,
    ordered: Boolean,
) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        var item = list.firstChild
        var index = 1
        while (item != null) {
            ListItemView(item, onLink, if (ordered) "$index." else "•")
            index++
            item = item.next
        }
    }
}

@Composable
private fun ListItemView(
    item: Node,
    onLink: (String) -> Unit,
    marker: String,
) {
    Row(modifier = Modifier.fillMaxWidth()) {
        Text(
            text = "$marker ",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            var block = item.firstChild
            while (block != null) {
                MarkdownBlock(block, onLink)
                block = block.next
            }
        }
    }
}

@Composable
private fun QuoteBlock(
    node: BlockQuote,
    onLink: (String) -> Unit,
) {
    Surface(
        color = MaterialTheme.colorScheme.surfaceVariant,
        shape = RoundedCornerShape(8.dp),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(modifier = Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            var child = node.firstChild
            while (child != null) {
                if (child is Paragraph) {
                    Text(
                        text = inlineAnnotated(child, onLink),
                        style = MaterialTheme.typography.bodyMedium,
                        fontStyle = FontStyle.Italic,
                    )
                } else {
                    MarkdownBlock(child, onLink)
                }
                child = child.next
            }
        }
    }
}

@Composable
private fun CodeBlock(code: String) {
    Surface(
        color = MaterialTheme.colorScheme.surfaceVariant,
        shape = RoundedCornerShape(8.dp),
        modifier = Modifier.fillMaxWidth(),
    ) {
        SelectionContainer {
            Text(
                text = code.trimEnd('\n'),
                style = MaterialTheme.typography.bodySmall,
                fontFamily = FontFamily.Monospace,
                modifier =
                    Modifier
                        .padding(12.dp)
                        .horizontalScroll(rememberScrollState()),
            )
        }
    }
}

@Composable
private fun TableBlockView(
    table: TableBlock,
    onLink: (String) -> Unit,
) {
    Surface(
        color = MaterialTheme.colorScheme.surfaceVariant,
        shape = RoundedCornerShape(8.dp),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(modifier = Modifier.horizontalScroll(rememberScrollState())) {
            var section = table.firstChild
            while (section != null) {
                val header = section is TableHead
                var row = section.firstChild
                while (row != null) {
                    TableRowView(row, onLink, header)
                    HorizontalDivider()
                    row = row.next
                }
                section = section.next
            }
        }
    }
}

@Composable
private fun TableRowView(
    row: Node,
    onLink: (String) -> Unit,
    header: Boolean,
) {
    Row {
        var cell = row.firstChild
        while (cell != null) {
            Box(modifier = Modifier.width(140.dp).padding(8.dp)) {
                Text(
                    text = inlineAnnotated(cell, onLink),
                    style = MaterialTheme.typography.bodySmall,
                    fontWeight = if (header) FontWeight.Bold else FontWeight.Normal,
                )
            }
            cell = cell.next
        }
    }
}

@Composable
private fun inlineAnnotated(
    parent: Node,
    onLink: (String) -> Unit,
): AnnotatedString {
    val linkColor = MaterialTheme.colorScheme.primary
    val codeBackground = MaterialTheme.colorScheme.surfaceVariant
    return buildAnnotatedString { appendInline(parent, onLink, linkColor, codeBackground) }
}

private fun AnnotatedString.Builder.appendInline(
    parent: Node,
    onLink: (String) -> Unit,
    linkColor: Color,
    codeBackground: Color,
) {
    var child = parent.firstChild
    while (child != null) {
        when (val n = child) {
            is CmText -> {
                append(n.literal)
            }

            is StrongEmphasis -> {
                withStyle(SpanStyle(fontWeight = FontWeight.Bold)) {
                    appendInline(n, onLink, linkColor, codeBackground)
                }
            }

            is Emphasis -> {
                withStyle(SpanStyle(fontStyle = FontStyle.Italic)) {
                    appendInline(n, onLink, linkColor, codeBackground)
                }
            }

            is Code -> {
                withStyle(SpanStyle(fontFamily = FontFamily.Monospace, background = codeBackground)) {
                    append(n.literal)
                }
            }

            is Image -> {
                append(n.firstChildText().ifEmpty { n.title.orEmpty() })
            }

            is Link -> {
                val href = normalizeHelpHref(n.destination)
                val styles = TextLinkStyles(SpanStyle(color = linkColor, textDecoration = TextDecoration.Underline))
                withLink(LinkAnnotation.Clickable(tag = href, styles = styles) { onLink(href) }) {
                    appendInline(n, onLink, linkColor, codeBackground)
                }
            }

            is SoftLineBreak -> {
                if (needsBreakSpace(n)) append(' ')
            }

            is HardLineBreak -> {
                append('\n')
            }

            else -> {
                appendInline(n, onLink, linkColor, codeBackground)
            }
        }
        child = child.next
    }
}

/** A soft break joins with a space unless the text on both sides is CJK. */
private fun needsBreakSpace(brk: Node): Boolean {
    val prev = brk.previous?.let(::lastTextChar)
    val next = brk.next?.let(::firstTextChar)
    return !(prev != null && prev.isCjk() && next != null && next.isCjk())
}

private fun lastTextChar(node: Node): Char? =
    when (node) {
        is CmText -> {
            node.literal.lastOrNull()
        }

        else -> {
            var child = node.lastChild
            while (child != null) {
                lastTextChar(child)?.let { return it }
                child = child.previous
            }
            null
        }
    }

private fun firstTextChar(node: Node): Char? =
    when (node) {
        is CmText -> {
            node.literal.firstOrNull()
        }

        else -> {
            var child = node.firstChild
            while (child != null) {
                firstTextChar(child)?.let { return it }
                child = child.next
            }
            null
        }
    }

private fun Node.firstChildText(): String {
    val sb = StringBuilder()
    var child = firstChild
    while (child != null) {
        if (child is CmText) sb.append(child.literal)
        child = child.next
    }
    return sb.toString()
}
