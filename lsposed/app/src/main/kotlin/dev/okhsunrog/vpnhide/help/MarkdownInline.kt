package dev.okhsunrog.vpnhide.help

/**
 * Inline span parser for the Markdown subset (see [MdBlock]). Scans a single
 * logical line into text / bold / italic / code / link spans, honouring `\`
 * escapes so a literal `*` can appear inside bold (`**\***`).
 *
 * A marker that never closes (`2 * 3`, a bare `[`) is treated as literal text:
 * the token readers return -1 and the opener character folds into the text
 * buffer, so span order is preserved.
 */
internal fun parseInline(text: String): List<MdSpan> {
    val spans = mutableListOf<MdSpan>()
    val buf = StringBuilder()

    fun flush() {
        if (buf.isNotEmpty()) {
            spans += MdSpan.Text(buf.toString())
            buf.clear()
        }
    }
    var i = 0
    while (i < text.length) {
        val c = text[i]
        val next =
            when {
                c == '\\' && i + 1 < text.length -> {
                    buf.append(text[i + 1])
                    i + 2
                }

                c == '`' -> {
                    emit(text, i + 1, "`", spans, ::flush) { MdSpan.Code(it) }
                }

                text.startsWith("**", i) -> {
                    emit(text, i + 2, "**", spans, ::flush) { MdSpan.Bold(unescape(it)) }
                }

                c == '*' -> {
                    emit(text, i + 1, "*", spans, ::flush) { MdSpan.Italic(unescape(it)) }
                }

                c == '[' -> {
                    emitLink(text, i, spans, ::flush)
                }

                else -> {
                    -1
                }
            }
        if (next < 0) {
            buf.append(c)
            i++
        } else {
            i = next
        }
    }
    flush()
    return spans
}

/**
 * If [delim] closes after [from], flush pending text and emit [make] of the raw
 * inner text, returning the index past the closer. Returns -1 when [delim] never
 * closes, so the caller keeps the opener as literal text.
 */
private inline fun emit(
    text: String,
    from: Int,
    delim: String,
    spans: MutableList<MdSpan>,
    flush: () -> Unit,
    make: (String) -> MdSpan,
): Int {
    val end = findClose(text, from, delim)
    if (end < 0) return -1
    flush()
    spans += make(text.substring(from, end))
    return end + delim.length
}

/** `[label](href)`; returns -1 when the shape doesn't match (literal `[`). */
private fun emitLink(
    text: String,
    start: Int,
    spans: MutableList<MdSpan>,
    flush: () -> Unit,
): Int {
    val close = findClose(text, start + 1, "]")
    if (close < 0 || close + 1 >= text.length || text[close + 1] != '(') return -1
    val parenEnd = text.indexOf(')', close + 2)
    if (parenEnd < 0) return -1
    flush()
    spans += MdSpan.Link(unescape(text.substring(start + 1, close)), text.substring(close + 2, parenEnd).trim())
    return parenEnd + 1
}

/** Index of [delim] at or after [from], skipping `\`-escaped chars; -1 if none. */
private fun findClose(
    text: String,
    from: Int,
    delim: String,
): Int {
    var i = from
    while (i < text.length) {
        if (text[i] == '\\' && i + 1 < text.length) {
            i += 2
            continue
        }
        if (text.startsWith(delim, i)) return i
        i++
    }
    return -1
}

/** Resolve `\x` escapes to the literal `x`. */
private fun unescape(s: String): String {
    if ('\\' !in s) return s
    val sb = StringBuilder(s.length)
    var i = 0
    while (i < s.length) {
        if (s[i] == '\\' && i + 1 < s.length) {
            sb.append(s[i + 1])
            i += 2
        } else {
            sb.append(s[i])
            i++
        }
    }
    return sb.toString()
}
