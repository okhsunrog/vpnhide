package dev.okhsunrog.vpnhide.help

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import dev.okhsunrog.vpnhide.ContactModal
import dev.okhsunrog.vpnhide.R
import dev.okhsunrog.vpnhide.ui.components.AppSearchTopBar
import dev.okhsunrog.vpnhide.ui.components.EnhancedCard
import dev.okhsunrog.vpnhide.ui.theme.AppColors
import kotlinx.coroutines.Dispatchers

/** Frames to wait for a heading to lay out before an anchor scroll gives up (~1s). */
private const val ANCHOR_SCROLL_MAX_FRAMES = 60

/** Small gap left above the target heading after an anchor scroll, in pixels. */
private const val ANCHOR_SCROLL_GAP_PX = 8

/**
 * The offline guide overlay: a table of contents with "common questions" chips
 * that opens into a rendered article. [initialArticleId] opens straight to an
 * article (a
 * contextual entry); null opens the table of contents. [onNavigate] receives a
 * `vpnhide://` link so the host can leave help and go to the right screen;
 * [onClose] dismisses the overlay.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun HelpScreen(
    initialArticleId: String?,
    onClose: () -> Unit,
    onNavigate: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val language = LocalConfiguration.current.locales[0].language
    val helpAssetBaseDir = remember(language) { "help/${resolveHelpLocale(language)}" }
    val content by produceState<HelpContent?>(initialValue = null, language) {
        value = kotlinx.coroutines.withContext(Dispatchers.IO) { loadHelpContent(context, language) }
    }
    var articleId by rememberSaveable { mutableStateOf(initialArticleId) }
    var searchActive by rememberSaveable { mutableStateOf(false) }
    var query by rememberSaveable { mutableStateOf("") }
    var contactOpen by rememberSaveable { mutableStateOf(false) }
    var pendingAnchor by rememberSaveable { mutableStateOf<String?>(null) }
    val uriHandler = LocalUriHandler.current

    if (contactOpen) ContactModal(onDismiss = { contactOpen = false })

    val onLink: (String) -> Unit = { href ->
        when {
            href.startsWith("#") -> {
                pendingAnchor = href.removePrefix("#")
            }

            href.startsWith(ARTICLE_SCHEME) -> {
                val rest = href.removePrefix(ARTICLE_SCHEME)
                articleId = rest.substringBefore('#')
                pendingAnchor = rest.substringAfter('#', "").ifEmpty { null }
            }

            href.startsWith(IN_APP_SCHEME) -> {
                onNavigate(href)
            }

            else -> {
                runCatching { uriHandler.openUri(href) }
            }
        }
    }

    BackHandler {
        when {
            searchActive -> {
                searchActive = false
                query = ""
            }

            articleId != null -> {
                articleId = null
            }

            else -> {
                onClose()
            }
        }
    }

    val loaded = content
    val article = loaded?.guide?.article(articleId ?: "")
    Scaffold(
        containerColor = AppColors.screenBackground,
        modifier = modifier.fillMaxSize(),
        topBar = {
            if (searchActive && loaded != null) {
                AppSearchTopBar(
                    query = query,
                    onQueryChange = { query = it },
                    onClose = {
                        searchActive = false
                        query = ""
                    },
                    placeholder = stringResource(R.string.help_search_placeholder),
                )
            } else {
                TopAppBar(
                    title = { Text(article?.title ?: stringResource(R.string.help_title)) },
                    navigationIcon = {
                        IconButton(onClick = { if (articleId != null) articleId = null else onClose() }) {
                            Icon(
                                Icons.AutoMirrored.Filled.ArrowBack,
                                contentDescription = stringResource(R.string.action_back),
                            )
                        }
                    },
                    actions = {
                        if (loaded != null && article == null) {
                            IconButton(onClick = { searchActive = true }) {
                                Icon(
                                    Icons.Filled.Search,
                                    contentDescription = stringResource(R.string.help_search_placeholder),
                                )
                            }
                        }
                    },
                    colors =
                        TopAppBarDefaults.topAppBarColors(
                            containerColor = AppColors.topBarContainer,
                            titleContentColor = MaterialTheme.colorScheme.onSurface,
                            navigationIconContentColor = MaterialTheme.colorScheme.onSurfaceVariant,
                            actionIconContentColor = MaterialTheme.colorScheme.onSurfaceVariant,
                        ),
                )
            }
        },
    ) { inner ->
        when {
            loaded == null -> {
                Column(
                    modifier = Modifier.fillMaxSize().padding(inner),
                    verticalArrangement = Arrangement.Center,
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) { CircularProgressIndicator() }
            }

            searchActive -> {
                HelpSearchResults(loaded, query, Modifier.padding(inner)) {
                    articleId = it
                    searchActive = false
                    query = ""
                }
            }

            article != null -> {
                val scrollState = remember(article.id) { ScrollState(0) }
                val anchors = remember(article.id) { HelpAnchorRegistry() }
                LaunchedEffect(article.id, pendingAnchor) {
                    val slug = pendingAnchor ?: return@LaunchedEffect
                    var offset: Int? = null
                    var frames = 0
                    while (offset == null && frames < ANCHOR_SCROLL_MAX_FRAMES) {
                        withFrameNanos { }
                        offset = anchors.offsetOf(slug)
                        frames++
                    }
                    offset?.let { scrollState.animateScrollTo((it - ANCHOR_SCROLL_GAP_PX).coerceAtLeast(0)) }
                    pendingAnchor = null
                }
                Column(
                    modifier =
                        Modifier
                            .fillMaxSize()
                            .padding(inner)
                            .verticalScroll(scrollState)
                            .padding(horizontal = 16.dp, vertical = 12.dp),
                ) { MarkdownText(loaded.doc(article.id), onLink, context, helpAssetBaseDir, anchors) }
            }

            else -> {
                HelpTableOfContents(
                    content = loaded,
                    modifier = Modifier.padding(inner),
                    onContact = { contactOpen = true },
                    onOpenArticle = { articleId = it },
                )
            }
        }
    }
}

@Composable
private fun HelpTableOfContents(
    content: HelpContent,
    modifier: Modifier,
    onContact: () -> Unit,
    onOpenArticle: (String) -> Unit,
) {
    LazyColumn(
        modifier = modifier.fillMaxSize(),
        contentPadding = PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        if (content.faq.isNotEmpty()) {
            item(key = "faq_title") { TocSectionLabel(stringResource(R.string.help_faq_title)) }
            item(key = "faq_chips") {
                Row(
                    modifier = Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    content.faq.forEach { entry ->
                        FilterChip(
                            selected = false,
                            onClick = { onOpenArticle(entry.articleId) },
                            label = { Text(entry.label) },
                        )
                    }
                }
            }
        }
        content.guide.sections.forEach { section ->
            item(key = "sec_${section.id}") { TocSectionLabel(section.title) }
            items(section.articles, key = { "art_${it.id}" }) { ref ->
                HelpArticleRow(ref.title, null) { onOpenArticle(ref.id) }
            }
        }
        item(key = "contact_footer") {
            HelpArticleRow(
                title = stringResource(R.string.help_no_answer_title),
                snippet = stringResource(R.string.help_no_answer_body),
                onClick = onContact,
            )
        }
    }
}

@Composable
private fun TocSectionLabel(text: String) {
    Text(
        text = text,
        style = MaterialTheme.typography.titleSmall,
        fontWeight = FontWeight.Bold,
        modifier = Modifier.padding(top = 8.dp, bottom = 2.dp),
    )
}

@Composable
private fun HelpSearchResults(
    content: HelpContent,
    query: String,
    modifier: Modifier,
    onOpenArticle: (String) -> Unit,
) {
    // Backed by title + body + the manifest keyword aliases, so a query matches
    // how people phrase things, not only the doc's wording; the snippet is always
    // cut from the body, never the aliases.
    if (query.trim().length < 2) {
        HelpSearchNotice(stringResource(R.string.help_search_prompt), modifier)
        return
    }
    val hits = content.search(query)
    if (hits.isEmpty()) {
        HelpSearchNotice(stringResource(R.string.help_search_empty), modifier)
        return
    }
    LazyColumn(
        modifier = modifier.fillMaxSize(),
        contentPadding = PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        items(hits, key = { "hit_${it.article.id}" }) { hit ->
            HelpArticleRow(hit.article.title, hit.snippet) { onOpenArticle(hit.article.id) }
        }
    }
}

@Composable
private fun HelpSearchNotice(
    text: String,
    modifier: Modifier,
) {
    Column(
        modifier = modifier.fillMaxSize().padding(32.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            text = text,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
    }
}

@Composable
private fun HelpArticleRow(
    title: String,
    snippet: String?,
    onClick: () -> Unit,
) {
    EnhancedCard(modifier = Modifier.fillMaxWidth(), onClick = onClick) {
        Column(modifier = Modifier.padding(14.dp)) {
            Text(title, style = MaterialTheme.typography.bodyLarge, fontWeight = FontWeight.Medium)
            if (snippet != null) {
                Text(
                    text = snippet,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(top = 2.dp),
                )
            }
        }
    }
}
