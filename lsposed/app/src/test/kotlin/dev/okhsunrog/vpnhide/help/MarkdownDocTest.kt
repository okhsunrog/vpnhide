package dev.okhsunrog.vpnhide.help

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class MarkdownDocTest {
    @Test
    fun `plainText flattens headings, paragraphs, lists and link labels`() {
        val text = plainText(parseMarkdown("# Title\n\nbody **word**\n\n- [item](x.md) one"))
        assertTrue(text.contains("Title"))
        assertTrue(text.contains("body word"))
        assertTrue(text.contains("item one"))
    }

    @Test
    fun `plainText includes table cell text so search covers tables`() {
        val md = "| Vector | Covered |\n|---|---|\n| 目标 | yes |"
        val text = plainText(parseMarkdown(md))
        assertTrue(text.contains("目标"))
        assertTrue(text.contains("yes"))
    }

    @Test
    fun `relative md links become in-app article routes`() {
        assertEquals("article:configure-hiding", normalizeHelpHref("configure-hiding.md"))
        assertEquals("article:game-accelerators", normalizeHelpHref("game-accelerators.md#roles"))
        assertEquals("article:x", normalizeHelpHref("docs/x.md"))
    }

    @Test
    fun `in-app, article and absolute links pass through`() {
        assertEquals("vpnhide://hiding", normalizeHelpHref("vpnhide://hiding"))
        assertEquals("article:foo", normalizeHelpHref("article:foo"))
        assertEquals("https://example.com", normalizeHelpHref("https://example.com"))
        assertEquals("#roles", normalizeHelpHref("#roles"))
    }

    @Test
    fun `isCjk recognises ideographs and fullwidth punctuation but not latin`() {
        assertTrue('目'.isCjk())
        assertTrue('，'.isCjk())
        assertTrue('。'.isCjk())
        assertFalse('a'.isCjk())
        assertFalse(' '.isCjk())
    }
}
