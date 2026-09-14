package dev.okhsunrog.vpnhide.help

/**
 * A deliberately small Markdown subset — only what the offline guide articles
 * under `docs/help/` actually use. Not a general Markdown engine: we own the
 * source, so the parser stays a few pure functions with a unit test rather than
 * a dependency, and links carry a custom scheme the UI turns into in-app
 * navigation (see [MdSpan.Link]).
 *
 * Supported: `#`/`##`/`###` headings, `- ` bullet and `1. ` numbered lists
 * (items may soft-wrap across physical lines), `> ` block quotes, paragraphs,
 * and inline `**bold**`, `*italic*`, `` `code` ``, `[text](href)`, with `\`
 * escaping the next character.
 */
internal sealed interface MdBlock {
    data class Heading(
        val level: Int,
        val spans: List<MdSpan>,
    ) : MdBlock

    data class Paragraph(
        val spans: List<MdSpan>,
    ) : MdBlock

    data class BulletList(
        val items: List<List<MdSpan>>,
    ) : MdBlock

    data class NumberedList(
        val items: List<List<MdSpan>>,
    ) : MdBlock

    data class Quote(
        val spans: List<MdSpan>,
    ) : MdBlock
}

internal sealed interface MdSpan {
    data class Text(
        val text: String,
    ) : MdSpan

    data class Bold(
        val text: String,
    ) : MdSpan

    data class Italic(
        val text: String,
    ) : MdSpan

    data class Code(
        val text: String,
    ) : MdSpan

    data class Link(
        val text: String,
        val href: String,
    ) : MdSpan
}

private val HEADING = Regex("^(#{1,3})\\s+(.*)$")
private val BULLET = Regex("^- +(.*)$")
private val NUMBERED = Regex("^\\d+\\. +(.*)$")

/** Parse an article body into a flat list of blocks, in document order. */
internal fun parseMarkdown(markdown: String): List<MdBlock> {
    val lines = markdown.replace("\r\n", "\n").split("\n")
    val blocks = mutableListOf<MdBlock>()
    var i = 0
    while (i < lines.size) {
        val line = lines[i]
        when {
            line.isBlank() -> {
                i++
            }

            HEADING.matches(line) -> {
                val m = HEADING.find(line)!!
                blocks += MdBlock.Heading(m.groupValues[1].length, parseInline(m.groupValues[2]))
                i++
            }

            line.startsWith(">") -> {
                i = collectQuote(lines, i, blocks)
            }

            BULLET.matches(line) || NUMBERED.matches(line) -> {
                i = collectList(lines, i, blocks)
            }

            else -> {
                i = collectParagraph(lines, i, blocks)
            }
        }
    }
    return blocks
}

/** Join consecutive `>` lines into one quote; returns the next unread index. */
private fun collectQuote(
    lines: List<String>,
    start: Int,
    out: MutableList<MdBlock>,
): Int {
    var i = start
    val sb = StringBuilder()
    while (i < lines.size && lines[i].startsWith(">")) {
        if (sb.isNotEmpty()) sb.append(' ')
        sb.append(lines[i].removePrefix(">").trim())
        i++
    }
    out += MdBlock.Quote(parseInline(sb.toString()))
    return i
}

/** Collect a run of list items; wrapped continuation lines fold into the item. */
private fun collectList(
    lines: List<String>,
    start: Int,
    out: MutableList<MdBlock>,
): Int {
    val ordered = NUMBERED.matches(lines[start])
    val items = mutableListOf<StringBuilder>()
    var i = start
    while (i < lines.size) {
        val line = lines[i]
        val marker = (if (ordered) NUMBERED else BULLET).find(line)
        when {
            marker != null -> {
                items += StringBuilder(marker.groupValues[1])
            }

            line.isNotBlank() && line[0] == ' ' && items.isNotEmpty() -> {
                items.last().append(' ').append(line.trim())
            }

            else -> {
                break
            }
        }
        i++
    }
    val spans = items.map { parseInline(it.toString()) }
    out += if (ordered) MdBlock.NumberedList(spans) else MdBlock.BulletList(spans)
    return i
}

/** Fold consecutive plain lines into one soft-wrapped paragraph. */
private fun collectParagraph(
    lines: List<String>,
    start: Int,
    out: MutableList<MdBlock>,
): Int {
    var i = start
    val sb = StringBuilder()
    while (i < lines.size) {
        val line = lines[i]
        if (line.isBlank() || HEADING.matches(line) || line.startsWith(">") ||
            BULLET.matches(line) || NUMBERED.matches(line)
        ) {
            break
        }
        if (sb.isNotEmpty()) sb.append(' ')
        sb.append(line.trim())
        i++
    }
    out += MdBlock.Paragraph(parseInline(sb.toString()))
    return i
}

/** Flatten blocks to plain text (headings, paragraphs, items) for search. */
internal fun plainText(blocks: List<MdBlock>): String =
    blocks.joinToString("\n") { block ->
        when (block) {
            is MdBlock.Heading -> spansText(block.spans)
            is MdBlock.Paragraph -> spansText(block.spans)
            is MdBlock.Quote -> spansText(block.spans)
            is MdBlock.BulletList -> block.items.joinToString("\n") { spansText(it) }
            is MdBlock.NumberedList -> block.items.joinToString("\n") { spansText(it) }
        }
    }

private fun spansText(spans: List<MdSpan>): String =
    spans.joinToString("") { span ->
        when (span) {
            is MdSpan.Text -> span.text
            is MdSpan.Bold -> span.text
            is MdSpan.Italic -> span.text
            is MdSpan.Code -> span.text
            is MdSpan.Link -> span.text
        }
    }
