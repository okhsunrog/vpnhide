package dev.okhsunrog.vpnhide.help

import org.commonmark.ext.gfm.tables.TablesExtension
import org.commonmark.node.Code
import org.commonmark.node.FencedCodeBlock
import org.commonmark.node.HardLineBreak
import org.commonmark.node.IndentedCodeBlock
import org.commonmark.node.Node
import org.commonmark.node.SoftLineBreak
import org.commonmark.node.Text
import org.commonmark.parser.Parser

/**
 * The offline guide is authored in CommonMark (+ GFM tables). We parse with the
 * commonmark library — robust and spec-compliant for tables, nested lists,
 * fenced code and links — but keep our own Compose renderer over the AST (see
 * [MarkdownText]) so link routing (`vpnhide://`, `article:`, relative `.md`) and
 * CJK soft-break joining stay under our control.
 */
internal const val ARTICLE_SCHEME = "article:"
internal const val IN_APP_SCHEME = "vpnhide://"

private val markdownParser: Parser =
    Parser.builder().extensions(listOf(TablesExtension.create())).build()

internal fun parseMarkdown(markdown: String): Node = markdownParser.parse(markdown)

/**
 * Flatten a parsed document to plain text for search (words only; block breaks
 * become spaces). Covers every node type, table cells included, so search never
 * misses content the visual renderer shows.
 */
internal fun plainText(node: Node): String {
    val sb = StringBuilder()
    collectText(node, sb)
    return sb.toString().replace(Regex("[ \\t]*\\n+[ \\t]*"), "\n").trim()
}

private fun collectText(
    node: Node,
    sb: StringBuilder,
) {
    var child: Node? = node.firstChild
    while (child != null) {
        when (child) {
            is Text -> {
                sb.append(child.literal)
            }

            is Code -> {
                sb.append(child.literal)
            }

            is FencedCodeBlock -> {
                sb.append(child.literal)
            }

            is IndentedCodeBlock -> {
                sb.append(child.literal)
            }

            is SoftLineBreak, is HardLineBreak -> {
                sb.append(' ')
            }

            else -> {
                collectText(child, sb)
                if (child.isBlock()) sb.append('\n')
            }
        }
        child = child.next
    }
}

private fun Node.isBlock(): Boolean = this is org.commonmark.node.Block

/**
 * Map a Markdown link destination to what the app should do with it: a relative
 * `*.md` link (how articles cross-reference each other, so the link also works on
 * GitHub) becomes an `article:<id>` in-app route; `vpnhide://`, `article:` and
 * absolute URLs pass through unchanged.
 */
internal fun normalizeHelpHref(destination: String): String {
    val dest = destination.trim()
    if (dest.startsWith(ARTICLE_SCHEME) || dest.startsWith(IN_APP_SCHEME)) return dest
    if (dest.startsWith("http://") || dest.startsWith("https://")) return dest
    val withoutAnchor = dest.substringBefore('#')
    if (withoutAnchor.endsWith(".md")) {
        val id = withoutAnchor.substringAfterLast('/').removeSuffix(".md")
        if (id.isNotEmpty()) return "$ARTICLE_SCHEME$id"
    }
    return dest
}

/** CJK ideographs, kana, and CJK/fullwidth punctuation — scripts written without spaces. */
internal fun Char.isCjk(): Boolean =
    this in '　'..'〿' || // CJK symbols and punctuation
        this in '぀'..'ヿ' || // hiragana + katakana
        this in '㐀'..'鿿' || // CJK unified ideographs (+ ext A)
        this in '＀'..'￯' // halfwidth and fullwidth forms
