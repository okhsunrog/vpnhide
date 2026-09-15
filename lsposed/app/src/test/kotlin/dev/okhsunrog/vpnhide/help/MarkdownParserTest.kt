package dev.okhsunrog.vpnhide.help

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class MarkdownParserTest {
    @Test
    fun `heading levels are parsed`() {
        val blocks = parseMarkdown("# One\n\n## Two\n\n### Three")
        assertEquals(
            listOf(1, 2, 3),
            blocks.filterIsInstance<MdBlock.Heading>().map { it.level },
        )
    }

    @Test
    fun `soft-wrapped lines fold into one paragraph`() {
        val blocks = parseMarkdown("alpha\nbeta\ngamma")
        assertEquals(1, blocks.size)
        val p = blocks.single() as MdBlock.Paragraph
        assertEquals("alpha beta gamma", (p.spans.single() as MdSpan.Text).text)
    }

    @Test
    fun `cjk soft-wrap joins without inserting a space`() {
        // The Chinese article is hard-wrapped mid-sentence; CJK has no spaces.
        val blocks = parseMarkdown("保存后立即\n生效")
        val p = blocks.single() as MdBlock.Paragraph
        assertEquals("保存后立即生效", (p.spans.single() as MdSpan.Text).text)
    }

    @Test
    fun `blank line separates two paragraphs`() {
        val blocks = parseMarkdown("one\n\ntwo")
        assertEquals(2, blocks.filterIsInstance<MdBlock.Paragraph>().size)
    }

    @Test
    fun `bullet items keep their wrapped continuation lines`() {
        val md = "- first item that\n  wraps on\n  three lines\n- second item"
        val list = parseMarkdown(md).single() as MdBlock.BulletList
        assertEquals(2, list.items.size)
        assertEquals("first item that wraps on three lines", (list.items[0].single() as MdSpan.Text).text)
        assertEquals("second item", (list.items[1].single() as MdSpan.Text).text)
    }

    @Test
    fun `numbered lists are recognised separately from bullets`() {
        val list = parseMarkdown("1. one\n2. two").single()
        assertTrue(list is MdBlock.NumberedList)
        assertEquals(2, (list as MdBlock.NumberedList).items.size)
    }

    @Test
    fun `quote lines join into one block`() {
        val q = parseMarkdown("> line one\n> line two").single() as MdBlock.Quote
        assertEquals("line one line two", (q.spans.single() as MdSpan.Text).text)
    }

    @Test
    fun `inline bold italic and code are parsed`() {
        val spans = parseInline("a **b** c *d* e `f`")
        assertTrue(spans.any { it is MdSpan.Bold && it.text == "b" })
        assertTrue(spans.any { it is MdSpan.Italic && it.text == "d" })
        assertTrue(spans.any { it is MdSpan.Code && it.text == "f" })
    }

    @Test
    fun `escaped asterisk inside bold is a literal star`() {
        val spans = parseInline("a **\\*** mark")
        assertEquals("*", (spans.first { it is MdSpan.Bold } as MdSpan.Bold).text)
    }

    @Test
    fun `links carry label and href`() {
        val spans = parseInline("open [the tab](vpnhide://hiding) now")
        val link = spans.filterIsInstance<MdSpan.Link>().single()
        assertEquals("the tab", link.text)
        assertEquals("vpnhide://hiding", link.href)
    }

    @Test
    fun `unclosed markers stay literal`() {
        val spans = parseInline("2 * 3 = 6 and [broken(")
        assertEquals("2 * 3 = 6 and [broken(", spans.joinToString("") { (it as MdSpan.Text).text })
    }

    @Test
    fun `plainText flattens headings paragraphs and items with link labels`() {
        val text = plainText(parseMarkdown("# Title\n\nbody **word**\n\n- [item](x) one"))
        assertEquals("Title\nbody word\nitem one", text)
    }
}
