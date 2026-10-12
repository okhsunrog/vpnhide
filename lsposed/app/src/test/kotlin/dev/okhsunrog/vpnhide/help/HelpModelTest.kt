package dev.okhsunrog.vpnhide.help

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
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
    fun `buildFaq keeps entries for present articles and drops the rest`() {
        val m =
            HelpManifestDto(
                faq =
                    listOf(
                        HelpFaqDto("configure-hiding", mapOf("en" to "Detects VPN", "ru" to "Видит VPN")),
                        HelpFaqDto("missing", mapOf("en" to "Nope")),
                    ),
            )
        val faq = buildFaq(m, "ru", setOf("configure-hiding"))
        assertEquals(1, faq.size)
        assertEquals("Видит VPN", faq.single().label)
        assertEquals("configure-hiding", faq.single().articleId)
    }

    @Test
    fun `buildFaq drops a locale-gated entry outside its locales`() {
        val m =
            HelpManifestDto(
                faq =
                    listOf(
                        HelpFaqDto(
                            article = "configure-hiding",
                            label = mapOf("en" to "Accelerator", "zh" to "加速器"),
                            locales = listOf("en", "zh"),
                        ),
                    ),
            )
        assertTrue(buildFaq(m, "ru", setOf("configure-hiding")).isEmpty())
        assertEquals(1, buildFaq(m, "en", setOf("configure-hiding")).size)
    }

    @Test
    fun `buildGuide drops a locale-gated article outside its locales`() {
        val m =
            HelpManifestDto(
                sections =
                    listOf(
                        HelpSectionDto(
                            id = "s",
                            title = mapOf("en" to "S"),
                            articles =
                                listOf(
                                    HelpArticleDto("always", mapOf("en" to "A")),
                                    HelpArticleDto("enzh", mapOf("en" to "B"), locales = listOf("en", "zh")),
                                ),
                        ),
                    ),
            )
        assertEquals(
            listOf("always"),
            buildGuide(m, "ru")
                .sections
                .single()
                .articles
                .map { it.id },
        )
        assertEquals(
            setOf("always", "enzh"),
            buildGuide(m, "en")
                .sections
                .single()
                .articles
                .map { it.id }
                .toSet(),
        )
    }

    @Test
    fun `articleKeywords picks the locale then falls back to english`() {
        val a = HelpArticleDto("x", keywords = mapOf("en" to listOf("bank"), "ru" to listOf("банк")))
        assertEquals(listOf("банк"), articleKeywords(a, "ru"))
        assertEquals(listOf("bank"), articleKeywords(a, "zh"))
    }

    @Test
    fun `search matches title and body and ignores short queries`() {
        val guide = buildGuide(manifest, "en")
        val body = mapOf("configure-hiding" to "Give the bank the Java and Native roles.")
        val noKeywords = { _: String -> "" }

        val byTitle = searchGuide(guide, "hiding", { body[it].orEmpty() }, noKeywords)
        assertEquals("configure-hiding", byTitle.single().article.id)

        val byBody = searchGuide(guide, "native roles", { body[it].orEmpty() }, noKeywords)
        assertTrue(byBody.single().snippet.contains("Native roles"))

        assertTrue(searchGuide(guide, "z", { body[it].orEmpty() }, noKeywords).isEmpty())
    }

    @Test
    fun `search matches a multi-word query by token even when the phrase is not contiguous`() {
        val guide = buildGuide(manifest, "en")
        // Neither the title "Set up hiding" nor an empty body contains the contiguous
        // phrase "hiding set", but both tokens are present in the title.
        val hits = searchGuide(guide, "hiding set", { "" }, { "" })
        assertEquals("configure-hiding", hits.single().article.id)
    }

    @Test
    fun `search ranks title and keyword matches above body-only matches`() {
        val m =
            HelpManifestDto(
                sections =
                    listOf(
                        HelpSectionDto(
                            id = "s",
                            title = mapOf("en" to "S"),
                            articles =
                                listOf(
                                    HelpArticleDto("overview", mapOf("en" to "Overview")),
                                    HelpArticleDto("first", mapOf("en" to "First app")),
                                ),
                        ),
                    ),
            )
        val guide = buildGuide(m, "en")
        val body = mapOf("overview" to "A bank may still see the VPN.", "first" to "Hide from your bank.")
        val keywords = mapOf("overview" to "", "first" to "bank")

        val ids = searchGuide(guide, "bank", { body[it].orEmpty() }, { keywords[it].orEmpty() }).map { it.article.id }
        // "first" matches by keyword (higher tier) though "overview" comes first in TOC order.
        assertEquals(listOf("first", "overview"), ids)
    }

    @Test
    fun `keyword-only match never leaks the alias list into the snippet`() {
        val guide = buildGuide(manifest, "en")
        val body = mapOf("configure-hiding" to "Give the bank the Java and Native roles.")
        // The alias "accelerator" is only in the keywords, not the body.
        val keywords = mapOf("configure-hiding" to "accelerator")

        val hit = searchGuide(guide, "accelerator", { body[it].orEmpty() }, { keywords[it].orEmpty() }).single()
        assertEquals("configure-hiding", hit.article.id)
        assertFalse(hit.snippet.contains("accelerator"))
    }
}
