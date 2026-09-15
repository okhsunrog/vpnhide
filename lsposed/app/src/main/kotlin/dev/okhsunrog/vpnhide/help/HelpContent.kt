package dev.okhsunrog.vpnhide.help

import android.content.Context
import dev.okhsunrog.vpnhide.LogTags
import dev.okhsunrog.vpnhide.VpnHideLog
import kotlinx.serialization.json.Json
import java.io.IOException

/**
 * The offline guide, loaded from the APK assets synced by `syncHelpAssets`:
 * `help/manifest.json` plus `help/<locale>/<id>.md`. Holds the localized
 * [guide] (table of contents), the parsed article [blocks] for rendering, and
 * pre-flattened plain text for [search]. Built once per open by [loadHelpContent]
 * on a background dispatcher — the assets are small, so there is no app-scoped
 * cache to keep in sync.
 */
internal class HelpContent(
    val guide: HelpGuide,
    val faq: List<HelpFaqEntry>,
    private val blocksById: Map<String, List<MdBlock>>,
    private val bodyById: Map<String, String>,
    private val searchTextById: Map<String, String>,
) {
    fun blocks(articleId: String): List<MdBlock> = blocksById[articleId].orEmpty()

    fun search(query: String): List<HelpSearchHit> =
        searchGuide(
            guide = guide,
            query = query,
            matchTextOf = { searchTextById[it].orEmpty() },
            snippetTextOf = { bodyById[it].orEmpty() },
        )
}

private val helpJson = Json { ignoreUnknownKeys = true }

/**
 * Read and parse the guide for the given Android [language] (falling back to
 * English content per article when a localized file is missing). Blocking; call
 * off the main thread. Returns an empty guide if the manifest can't be read.
 */
internal fun loadHelpContent(
    context: Context,
    language: String,
): HelpContent {
    val locale = resolveHelpLocale(language)
    val manifestText = readAsset(context, "help/manifest.json")
    if (manifestText == null) {
        VpnHideLog.w(LogTags.STARTUP, "help manifest missing from assets")
        return HelpContent(HelpGuide(locale, emptyList()), emptyList(), emptyMap(), emptyMap(), emptyMap())
    }
    val manifest: HelpManifestDto = helpJson.decodeFromString(manifestText)
    val guide = buildGuide(manifest, locale)
    val dtoById = manifest.sections.flatMap { it.articles }.associateBy { it.id }
    val blocksById = mutableMapOf<String, List<MdBlock>>()
    val bodyById = mutableMapOf<String, String>()
    val searchTextById = mutableMapOf<String, String>()
    for (section in guide.sections) {
        for (article in section.articles) {
            val body =
                readAsset(context, "help/$locale/${article.id}.md")
                    ?: readAsset(context, "help/en/${article.id}.md")
                    ?: ""
            val blocks = parseMarkdown(body)
            blocksById[article.id] = blocks
            val plain = plainText(blocks)
            bodyById[article.id] = plain
            // The searchable text also carries the article's keyword aliases so a
            // query matches how people phrase things — but the shown snippet is cut
            // from bodyById, never from these aliases.
            val keywords = dtoById[article.id]?.let { articleKeywords(it, locale) }.orEmpty()
            searchTextById[article.id] = (plain + "\n" + keywords.joinToString(" ")).trim()
        }
    }
    val faq = buildFaq(manifest, locale, blocksById.keys)
    return HelpContent(guide, faq, blocksById, bodyById, searchTextById)
}

private fun readAsset(
    context: Context,
    path: String,
): String? =
    try {
        context.assets
            .open(path)
            .bufferedReader()
            .use { it.readText() }
    } catch (e: IOException) {
        VpnHideLog.w(LogTags.STARTUP, "help asset unavailable: $path (${e.message})")
        null
    }
