package dev.okhsunrog.vpnhide.help

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class HelpModelTest {
    @Test
    fun `locale resolves supported languages and falls back to english`() {
        assertEquals("ru", resolveHelpLocale("ru"))
        assertEquals("zh", resolveHelpLocale("zh-rCN"))
        assertEquals("zh", resolveHelpLocale("ZH"))
        assertEquals("en", resolveHelpLocale("de"))
        assertEquals("en", resolveHelpLocale(""))
    }

    @Test
    fun `localized picks the locale then english then anything`() {
        val map = mapOf("en" to "E", "ru" to "R")
        assertEquals("R", localized(map, "ru"))
        assertEquals("E", localized(map, "zh"))
        assertEquals("", localized(emptyMap(), "en"))
    }

    private val manifest =
        HelpManifestDto(
            sections =
                listOf(
                    HelpSectionDto(
                        id = "configure",
                        title = mapOf("en" to "Set up", "ru" to "Настройка"),
                        articles = listOf(HelpArticleDto("configure-hiding", mapOf("en" to "Set up hiding"))),
                    ),
                    HelpSectionDto(id = "empty", title = mapOf("en" to "Empty"), articles = emptyList()),
                ),
        )

    @Test
    fun `buildGuide localizes titles and drops empty sections`() {
        val guide = buildGuide(manifest, "ru")
        assertEquals(1, guide.sections.size)
        assertEquals("Настройка", guide.sections.single().title)
        // Article title has no ru entry -> english fallback.
        assertEquals(
            "Set up hiding",
            guide.sections
                .single()
                .articles
                .single()
                .title,
        )
        assertEquals("configure", guide.article("configure-hiding")?.sectionId)
        assertNull(guide.article("missing"))
    }

    @Test
    fun `search matches title and body and ignores short queries`() {
        val guide = buildGuide(manifest, "en")
        val bodies = mapOf("configure-hiding" to "Give the bank the Java and Native roles.")

        val byTitle = searchGuide(guide, "hiding") { bodies[it].orEmpty() }
        assertEquals("configure-hiding", byTitle.single().article.id)

        val byBody = searchGuide(guide, "native roles") { bodies[it].orEmpty() }
        assertTrue(byBody.single().snippet.contains("Native roles"))

        assertTrue(searchGuide(guide, "z") { bodies[it].orEmpty() }.isEmpty())
    }
}
