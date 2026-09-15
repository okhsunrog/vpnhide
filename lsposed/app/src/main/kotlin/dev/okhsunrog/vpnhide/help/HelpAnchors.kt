package dev.okhsunrog.vpnhide.help

import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.layout.positionInWindow
import kotlin.math.roundToInt

/**
 * Tracks where each heading sits inside the scrolling article so an in-article
 * link (`#slug`, or `other.md#slug` after navigation) can scroll to it. Headings
 * register their laid-out coordinates and [contentRoot] is the scrolling content;
 * [offsetOf] returns a heading's pixel offset from the content top — the value to
 * pass to `ScrollState.animateScrollTo`. Registered by [MarkdownText]; read by the
 * article host in HelpScreen.
 */
internal class HelpAnchorRegistry {
    var contentRoot: LayoutCoordinates? = null
    private val headings = mutableMapOf<String, LayoutCoordinates>()

    fun register(
        slug: String,
        coordinates: LayoutCoordinates,
    ) {
        if (slug.isNotEmpty()) headings[slug] = coordinates
    }

    /**
     * Offset of [slug]'s heading from the top of the scrolling content, or null if
     * the heading is unknown or its layout isn't attached yet. Both coordinates
     * move together as the article scrolls, so their difference is scroll-invariant.
     */
    fun offsetOf(slug: String): Int? {
        val root = contentRoot ?: return null
        val heading = headings[slug] ?: return null
        if (!root.isAttached || !heading.isAttached) return null
        return (heading.positionInWindow().y - root.positionInWindow().y).roundToInt()
    }
}

internal val LocalHelpAnchors = staticCompositionLocalOf<HelpAnchorRegistry?> { null }

/**
 * GitHub-style heading slug: lower-cased, punctuation dropped, runs of whitespace
 * collapsed to single hyphens. Keeps Unicode letters (so RU and CJK headings get
 * usable anchors), matching how article authors write `#some-heading` links.
 */
internal fun headingSlug(text: String): String =
    text
        .trim()
        .lowercase()
        .replace(Regex("[^\\p{L}\\p{N} _-]"), "")
        .replace(Regex("\\s+"), "-")
