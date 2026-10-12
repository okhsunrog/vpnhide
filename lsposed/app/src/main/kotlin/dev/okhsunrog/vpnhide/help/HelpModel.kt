package dev.okhsunrog.vpnhide.help

import kotlinx.serialization.Serializable

/**
 * The `docs/help/manifest.json` shape (synced into assets as `help/manifest.json`).
 * Titles are per-locale maps keyed by "en"/"ru"/"zh". Unknown keys (the leading
 * `_comment`) are ignored by the loader's Json. Kept as thin DTOs; the UI works
 * with the localized [HelpGuide] built by [buildGuide].
 */
@Serializable
internal data class HelpManifestDto(
    val schema: Int = 1,
    val faq: List<HelpFaqDto> = emptyList(),
    val sections: List<HelpSectionDto> = emptyList(),
)

@Serializable
internal data class HelpSectionDto(
    val id: String,
    val title: Map<String, String> = emptyMap(),
    val articles: List<HelpArticleDto> = emptyList(),
)

@Serializable
internal data class HelpArticleDto(
    val id: String,
    val title: Map<String, String> = emptyMap(),
    val keywords: Map<String, List<String>> = emptyMap(),
    // Null = every locale; otherwise only these (e.g. game-accelerators is en/zh —
    // the RU audience's motivation is RKN/banks, not accelerators).
    val locales: List<String>? = null,
)

@Serializable
internal data class HelpFaqDto(
    val article: String,
    val label: Map<String, String> = emptyMap(),
    // Null = every locale; otherwise only these (e.g. the accelerator question is
    // en/zh, since the RU article deliberately has no accelerator section).
    val locales: List<String>? = null,
)

/** Locales we ship guide content for; everything else falls back to English. */
internal val HELP_LOCALES = listOf("en", "ru", "zh")

internal data class HelpGuide(
    val locale: String,
    val sections: List<HelpSection>,
) {
    fun article(id: String): HelpArticleRef? = sections.firstNotNullOfOrNull { s -> s.articles.firstOrNull { it.id == id } }
}

internal data class HelpSection(
    val id: String,
    val title: String,
    val articles: List<HelpArticleRef>,
)

internal data class HelpArticleRef(
    val id: String,
    val sectionId: String,
    val title: String,
)

/** A tappable "common question" on the table of contents that opens [articleId]. */
internal data class HelpFaqEntry(
    val label: String,
    val articleId: String,
)

/** Localized FAQ chips, dropping any that point at an article not in [presentArticleIds]. */
internal fun buildFaq(
    manifest: HelpManifestDto,
    locale: String,
    presentArticleIds: Set<String>,
): List<HelpFaqEntry> =
    manifest.faq
        .filter { it.article in presentArticleIds && (it.locales == null || locale in it.locales) }
        .map { HelpFaqEntry(localized(it.label, locale), it.article) }

/** Search-alias keywords for an article in [locale], English otherwise. */
internal fun articleKeywords(
    article: HelpArticleDto,
    locale: String,
): List<String> = article.keywords[locale] ?: article.keywords["en"].orEmpty()

/** Map an Android language tag to a shipped help locale, English otherwise. */
internal fun resolveHelpLocale(language: String): String =
    language
        .lowercase()
        .substringBefore('-')
        .substringBefore('_')
        .takeIf { it in HELP_LOCALES } ?: "en"

/** Pick [locale] from a per-locale map, falling back to English then anything. */
internal fun localized(
    byLocale: Map<String, String>,
    locale: String,
): String = byLocale[locale] ?: byLocale["en"] ?: byLocale.values.firstOrNull().orEmpty()

/**
 * Build the localized table of contents. Sections with no articles are dropped —
 * the manifest lists the full section set for ordering, but an empty section has
 * nothing to open, so it never reaches the TOC.
 */
internal fun buildGuide(
    manifest: HelpManifestDto,
    locale: String,
): HelpGuide {
    val sections =
        manifest.sections.mapNotNull { section ->
            val articles =
                section.articles
                    .filter { it.locales == null || locale in it.locales }
                    .map { HelpArticleRef(it.id, section.id, localized(it.title, locale)) }
            if (articles.isEmpty()) {
                null
            } else {
                HelpSection(section.id, localized(section.title, locale), articles)
            }
        }
    return HelpGuide(locale, sections)
}

internal data class HelpSearchHit(
    val article: HelpArticleRef,
    val snippet: String,
)

/**
 * Case-insensitive ranked search over article titles, bodies and keyword aliases.
 * The query is split into whitespace tokens (each >= 2 chars) and an article
 * matches only when *every* token appears in its title, body or keywords — so a
 * multi-word query no longer needs the exact phrase to sit contiguously in the
 * text. Hits are ranked (title matches above keyword matches above body-only
 * ones), and ties keep table-of-contents order. [bodyOf] supplies the body, which
 * is both matched and used to cut the snippet; [keywordsOf] supplies the alias
 * text, matched only and never shown in the snippet. Both are parameters so this
 * stays pure and testable, with asset IO in the caller. A query shorter than two
 * characters, or with no usable token, returns nothing.
 */
internal fun searchGuide(
    guide: HelpGuide,
    query: String,
    bodyOf: (String) -> String,
    keywordsOf: (String) -> String,
): List<HelpSearchHit> {
    val needle = query.trim().lowercase()
    if (needle.length < 2) return emptyList()
    val tokens = needle.split(WHITESPACE).filter { it.length >= 2 }
    if (tokens.isEmpty()) return emptyList()

    val ranked = mutableListOf<RankedHit>()
    var tocIndex = 0
    for (section in guide.sections) {
        for (article in section.articles) {
            val title = article.title.lowercase()
            val body = bodyOf(article.id)
            val bodyLower = body.lowercase()
            val keywords = keywordsOf(article.id).lowercase()
            val matches = tokens.all { it in title || it in bodyLower || it in keywords }
            if (matches) {
                val score = scoreHit(needle, tokens, title, keywords, bodyLower)
                val anchor = tokens.mapNotNull { bodyLower.indexOf(it).takeIf { i -> i >= 0 } }.minOrNull() ?: -1
                ranked += RankedHit(score, tocIndex, HelpSearchHit(article, snippetAround(body, anchor)))
            }
            tocIndex++
        }
    }
    return ranked
        .sortedWith(compareByDescending<RankedHit> { it.score }.thenBy { it.tocIndex })
        .map { it.hit }
}

private val WHITESPACE = Regex("\\s+")

private data class RankedHit(
    val score: Int,
    val tocIndex: Int,
    val hit: HelpSearchHit,
)

/**
 * Higher is more relevant: a whole-phrase title hit ranks above an all-token title
 * hit, above any-token; a keyword hit adds a middle tier; an incidental body match
 * scores lowest. Ordering within a score tier is left to the caller (TOC order).
 */
private fun scoreHit(
    needle: String,
    tokens: List<String>,
    title: String,
    keywords: String,
    body: String,
): Int {
    var score =
        when {
            needle in title -> 100
            tokens.all { it in title } -> 60
            tokens.any { it in title } -> 25
            else -> 0
        }
    if (needle in keywords || tokens.all { it in keywords }) score += 30
    if (needle in body) score += 5
    return score
}

private const val SNIPPET_RADIUS = 48

private fun snippetAround(
    body: String,
    idx: Int,
): String {
    if (body.isEmpty()) return ""
    if (idx < 0) return body.take(SNIPPET_RADIUS * 2).trim()
    val start = (idx - SNIPPET_RADIUS).coerceAtLeast(0)
    val end = (idx + SNIPPET_RADIUS).coerceAtMost(body.length)
    val prefix = if (start > 0) "…" else ""
    val suffix = if (end < body.length) "…" else ""
    return prefix + body.substring(start, end).trim() + suffix
}
