package dev.okhsunrog.vpnhide.help

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
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
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.okhsunrog.vpnhide.R
import dev.okhsunrog.vpnhide.ui.components.EnhancedCard
import dev.okhsunrog.vpnhide.ui.theme.AppColors
import kotlinx.coroutines.Dispatchers

private const val ARTICLE_SCHEME = "article:"
private const val IN_APP_SCHEME = "vpnhide://"

/**
 * The offline guide overlay: a table of contents with search that opens into a
 * rendered article. [initialArticleId] opens straight to an article (a
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
    val content by produceState<HelpContent?>(initialValue = null, language) {
        value = kotlinx.coroutines.withContext(Dispatchers.IO) { loadHelpContent(context, language) }
    }
    var articleId by rememberSaveable { mutableStateOf(initialArticleId) }
    val uriHandler = LocalUriHandler.current

    val onLink: (String) -> Unit = { href ->
        when {
            href.startsWith(ARTICLE_SCHEME) -> articleId = href.removePrefix(ARTICLE_SCHEME)
            href.startsWith(IN_APP_SCHEME) -> onNavigate(href)
            else -> runCatching { uriHandler.openUri(href) }
        }
    }

    BackHandler { if (articleId != null) articleId = null else onClose() }

    val loaded = content
    val article = loaded?.guide?.article(articleId ?: "")
    Scaffold(
        containerColor = AppColors.screenBackground,
        modifier = modifier.fillMaxSize(),
        topBar = {
            TopAppBar(
                title = { Text(article?.title ?: stringResource(R.string.help_title)) },
                navigationIcon = {
                    IconButton(onClick = { if (articleId != null) articleId = null else onClose() }) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = null)
                    }
                },
            )
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

            article != null -> {
                Column(
                    modifier =
                        Modifier
                            .fillMaxSize()
                            .padding(inner)
                            .verticalScroll(rememberScrollState())
                            .padding(horizontal = 16.dp, vertical = 12.dp),
                ) { MarkdownText(loaded.blocks(article.id), onLink) }
            }

            else -> {
                HelpTableOfContents(loaded, Modifier.padding(inner)) { articleId = it }
            }
        }
    }
}

@Composable
private fun HelpTableOfContents(
    content: HelpContent,
    modifier: Modifier,
    onOpenArticle: (String) -> Unit,
) {
    var query by rememberSaveable { mutableStateOf("") }
    val hits = remember(query) { content.search(query) }
    LazyColumn(
        modifier = modifier.fillMaxSize(),
        contentPadding =
            androidx.compose.foundation.layout
                .PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        item(key = "search") {
            OutlinedTextField(
                value = query,
                onValueChange = { query = it },
                singleLine = true,
                leadingIcon = { Icon(Icons.Filled.Search, contentDescription = null) },
                placeholder = { Text(stringResource(R.string.help_search_hint)) },
                modifier = Modifier.fillMaxWidth(),
            )
        }
        if (query.isNotBlank()) {
            if (hits.isEmpty()) {
                item(key = "empty") { Text(stringResource(R.string.help_search_empty)) }
            }
            items(hits, key = { "hit_${it.article.id}" }) { hit ->
                HelpArticleRow(hit.article.title, hit.snippet) { onOpenArticle(hit.article.id) }
            }
        } else {
            content.guide.sections.forEach { section ->
                item(key = "sec_${section.id}") {
                    Text(
                        text = section.title,
                        style = MaterialTheme.typography.titleSmall,
                        fontWeight = FontWeight.Bold,
                        modifier = Modifier.padding(top = 8.dp, bottom = 2.dp),
                    )
                }
                items(section.articles, key = { "art_${it.id}" }) { ref ->
                    HelpArticleRow(ref.title, null) { onOpenArticle(ref.id) }
                }
            }
        }
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
