package dev.okhsunrog.vpnhide

import android.content.res.Resources
import android.graphics.drawable.Drawable
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsDraggedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.wrapContentHeight
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.graphics.drawable.toBitmap
import dev.okhsunrog.vpnhide.ui.components.EnhancedButton
import io.github.oikvpqya.compose.fastscroller.VerticalScrollbar
import io.github.oikvpqya.compose.fastscroller.indicator.IndicatorConstants
import io.github.oikvpqya.compose.fastscroller.material3.defaultMaterialScrollbarStyle
import io.github.oikvpqya.compose.fastscroller.rememberScrollbarAdapter

/**
 * Common per-row fields the Apps picker needs. Role-specific wrappers
 * layer their own toggle flags on top of these, but the list scaffold —
 * filtering, scrollbar, save lifecycle, row chrome — only ever touches this
 * interface.
 */
internal interface TargetEntry {
    val packageName: String
    val label: String
    val icon: Drawable?
    val isSystem: Boolean
    val userIds: List<Int>

    /** True if the row has at least one role/layer selected. Drives the
     * "keep selected system apps visible even when system apps are hidden"
     * filter rule and the system-app filter exemption. */
    val anySelected: Boolean

    /** Selection state from the last loaded/saved config. Unsaved edits should
     * not move rows between Configured and Other apps before the user saves. */
    val groupSelected: Boolean get() = anySelected
}

/** Result of merging the cached app list with a screen's target snapshot.
 * [resaveNeeded] lets a screen request the Save button start enabled —
 * App hiding uses it to persist an auto-fixed hidden+observer conflict. */
internal data class MergeResult<T : TargetEntry>(
    val entries: List<T>,
    val resaveNeeded: Boolean = false,
)

/**
 * Everything a Save needs beyond the row entries themselves. The scaffold
 * supplies the self package (always a hidden Java/native target) and current
 * debug flag; UID resolution happens in the native activator.
 */
internal data class SaveContext(
    val selfPkg: String,
    val debug: Boolean,
)

/**
 * Shared scaffold for app-role picker screens. Owns all the
 * machinery they had copy-pasted: the cached-apps / targets subscription,
 * the dirty-guarded merge, search/system/Russian/configured filtering, the alphabetical
 * fast-scrollbar, the bottom save bar, the snackbar, and the save lifecycle
 * (including the exit-code → message mapping). Screens supply only what is
 * genuinely screen-specific via the parameters below.
 *
 * @param merge map the cached app list + target snapshot into typed rows.
 * @param moduleMissing optional gate: when it returns true the picker shows
 *   [moduleMissingContent] instead of the list (Ports, when its module isn't
 *   installed).
 * @param countText bottom-bar status text (e.g. "12 selected").
 * @param row renders one row; call `onChange` with the updated entry to mark
 *   the list dirty.
 * @param persist persist [entries] through the canonical-config repository.
 * @param successMessage snackbar text shown after a successful save.
 */
@Composable
internal fun <T : TargetEntry> TargetPickerScreen(
    searchQuery: String,
    showSystem: Boolean,
    showRussianOnly: Boolean,
    sortMode: TargetListSortMode,
    onToggleSystem: () -> Unit,
    onToggleRussianOnly: () -> Unit,
    onSortModeChange: (TargetListSortMode) -> Unit,
    modifier: Modifier,
    helpPrefKey: String,
    helpTitle: String,
    help: @Composable (TargetsSnapshot) -> Unit,
    merge: (apps: List<AppSummary>, targets: TargetsSnapshot, selfPkg: String) -> MergeResult<T>,
    countText: (entries: List<T>, resources: Resources) -> String,
    persist: suspend (entries: List<T>, ctx: SaveContext) -> CanonicalWriteResult,
    successMessage: (entries: List<T>, resources: Resources) -> String,
    selectionChangeError:
        (current: List<T>, candidate: List<T>, targets: TargetsSnapshot, selfPkg: String, resources: Resources) -> String? =
        { _, _, _, _, _ -> null },
    selectionSaveError: (entries: List<T>, targets: TargetsSnapshot, selfPkg: String, resources: Resources) -> String? =
        { _, _, _, _ -> null },
    moduleMissing: (TargetsSnapshot) -> Boolean = { false },
    moduleMissingContent: @Composable (Modifier) -> Unit = {},
    row: @Composable (entry: T, userNames: Map<Int, String>, targets: TargetsSnapshot, onChange: (T) -> Unit) -> Unit,
) {
    val context = LocalContext.current
    val resources = LocalResources.current
    val scope = rememberCoroutineScope()

    val cachedApps by AppListCache.apps.collectAsState()
    val appListError by AppListCache.error.collectAsState()
    val userNames by AppListCache.userNames.collectAsState()
    val lockedProfiles by AppListCache.lockedProfiles.collectAsState()
    val targets by TargetsCache.snapshot.collectAsState()
    val targetsError by TargetsCache.error.collectAsState()

    var allApps by remember { mutableStateOf<List<T>>(emptyList()) }
    var saving by remember { mutableStateOf(false) }
    var dirty by remember { mutableStateOf(false) }
    var snackMessage by remember { mutableStateOf<String?>(null) }
    var snackDuration by remember { mutableStateOf(SnackbarDuration.Long) }
    val snackbarHostState = remember { SnackbarHostState() }

    LaunchedEffect(snackMessage, snackDuration) {
        snackMessage?.let {
            snackbarHostState.showSnackbar(
                message = it,
                duration = snackDuration,
            )
            snackMessage = null
        }
    }

    // Both loads are idempotent (no-op when already loaded / in flight).
    // AppListCache is normally prewarmed at startup, but ensuring it here too
    // keeps the picker self-sufficient instead of silently depending on that.
    LaunchedEffect(Unit) {
        AppListCache.ensureLoaded(scope, context)
        TargetsCache.ensureLoaded(scope, context)
    }

    // Surface either cache's failure: a failed app-list scan used to leave
    // the picker stuck on an endless spinner (it had no error state at all).
    if ((targetsError != null && targets == null) || (appListError != null && cachedApps == null)) {
        val packageScanFailed = appListError != null && cachedApps == null
        TargetsLoadErrorCard(
            onRetry = {
                AppListCache.refresh(scope, context)
                TargetsCache.refresh(scope, context)
            },
            title =
                stringResource(
                    if (packageScanFailed) R.string.profile_scan_failed_title else R.string.targets_load_failed_title,
                ),
            message =
                stringResource(
                    if (packageScanFailed) R.string.profile_scan_failed_message else R.string.targets_load_failed_message,
                ),
            modifier = modifier,
        )
        return
    }

    // While `dirty` is true the user has unsaved checkbox edits — don't
    // overwrite them with a fresh snapshot. Caches can refresh under us
    // (ON_RESUME, another screen calling `TargetsCache.refresh()`) and
    // silently dropping the edits is the worst outcome.
    LaunchedEffect(cachedApps, targets) {
        if (dirty) return@LaunchedEffect
        val apps = cachedApps ?: return@LaunchedEffect
        val t = targets ?: return@LaunchedEffect
        val merged = merge(apps, t, context.packageName)
        allApps = merged.entries
        dirty = merged.resaveNeeded
    }

    val loading = cachedApps == null || targets == null

    targets?.let { t ->
        if (moduleMissing(t)) {
            moduleMissingContent(modifier)
            return
        }
    }

    val visibleApps =
        remember(allApps, searchQuery, showSystem, showRussianOnly, sortMode) {
            visibleTargetEntries(
                entries = allApps,
                searchQuery = searchQuery,
                showSystem = showSystem,
                showRussianOnly = showRussianOnly,
                sortMode = sortMode,
            )
        }
    val visibleSections = remember(visibleApps, sortMode) { targetListSections(visibleApps, sortMode) }

    val onChange: (T) -> Unit = { updated ->
        val candidate = allApps.map { if (it.packageName == updated.packageName) updated else it }
        val error =
            targets?.let { currentTargets ->
                selectionChangeError(allApps, candidate, currentTargets, context.packageName, resources)
            }
        if (error != null) {
            snackDuration = SnackbarDuration.Long
            snackMessage = error
        } else {
            allApps = candidate
            dirty = true
        }
    }

    Column(modifier = modifier.fillMaxSize()) {
        if (appListError != null && cachedApps != null) {
            StatusBanner(
                text = stringResource(R.string.profile_scan_stale_message),
                containerColor = StatusColors.warningContainer(),
                contentColor = StatusColors.warningHeader(),
                modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
            )
        }
        if (lockedProfiles.isNotEmpty()) {
            StatusBanner(
                text =
                    stringResource(
                        R.string.profile_scan_locked_skipped_message,
                        lockedProfiles.joinToString(", "),
                    ),
                containerColor = StatusColors.warningContainer(),
                contentColor = StatusColors.warningHeader(),
                modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
            )
        }
        if (loading) {
            Box(
                modifier = Modifier.fillMaxSize(),
                contentAlignment = Alignment.Center,
            ) {
                CircularProgressIndicator()
            }
        } else {
            val listState = rememberLazyListState()
            val currentTargets = targets
            val indexLabels =
                remember(visibleSections, currentTargets) {
                    targetListIndexLabels(visibleSections, hasHelpItem = currentTargets != null)
                }
            Box(modifier = Modifier.weight(1f)) {
                LazyColumn(
                    state = listState,
                    modifier = Modifier.fillMaxSize(),
                ) {
                    if (currentTargets != null) {
                        item(key = "help") {
                            Box(modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp)) {
                                HelpAccordion(prefKey = helpPrefKey, title = helpTitle) {
                                    help(currentTargets)
                                }
                            }
                        }
                    }
                    item(key = "filters") {
                        TargetFilterChips(
                            showSystem = showSystem,
                            showRussianOnly = showRussianOnly,
                            sortMode = sortMode,
                            onToggleSystem = onToggleSystem,
                            onToggleRussianOnly = onToggleRussianOnly,
                            onSortModeChange = onSortModeChange,
                            modifier = Modifier.padding(start = 12.dp, end = 12.dp, top = 2.dp, bottom = 4.dp),
                        )
                    }
                    visibleSections.forEach { section ->
                        section.group?.let { group ->
                            item(key = "group_${group.name}") {
                                TargetGroupHeader(group = group, count = section.entries.size)
                            }
                        }
                        items(section.entries, key = { it.packageName }) { app ->
                            if (currentTargets != null) {
                                row(app, userNames, currentTargets, onChange)
                            }
                        }
                    }
                }
                AppListScrollbar(
                    listState = listState,
                    firstVisibleLabel = {
                        firstVisibleTargetLabel(indexLabels, listState.firstVisibleItemIndex)
                    },
                )
            }
            SnackbarHost(
                hostState = snackbarHostState,
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp),
            )
            Surface(tonalElevation = 3.dp) {
                Row(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 16.dp, vertical = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = countText(allApps, resources),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.weight(1f),
                    )
                    EnhancedButton(
                        onClick = {
                            val error =
                                targets?.let { currentTargets ->
                                    selectionSaveError(allApps, currentTargets, context.packageName, resources)
                                }
                            if (error != null) {
                                snackDuration = SnackbarDuration.Long
                                snackMessage = error
                            } else {
                                saving = true
                                dirty = false
                            }
                        },
                        enabled = dirty && !saving,
                    ) {
                        Text(stringResource(R.string.btn_save))
                    }
                }
            }
        }
    }

    if (saving) {
        LaunchedEffect(Unit) {
            val entries = allApps
            val selfPkg = context.packageName
            val ctx =
                SaveContext(
                    selfPkg,
                    targets?.canonicalConfig?.debugSwitch ?: (targets?.canonicalConfig?.debug ?: false),
                )
            try {
                val result = persist(entries, ctx)
                when (result.exitCode) {
                    0 -> {
                        val capacityWarning = parseNativeTargetCapacityWarning(result.output)
                        if (capacityWarning != null) {
                            snackDuration = SnackbarDuration.Long
                            snackMessage =
                                resources.getString(
                                    R.string.save_native_target_capacity,
                                    capacityWarning.capacity,
                                    capacityWarning.total,
                                    capacityWarning.dropped,
                                )
                        } else {
                            snackDuration = SnackbarDuration.Short
                            snackMessage = successMessage(entries, resources)
                        }
                        TargetsCache.refreshAfterSave(scope, context)
                    }

                    -1 -> {
                        snackDuration = SnackbarDuration.Long
                        snackMessage = resources.getString(R.string.save_failed_root)
                        dirty = true
                    }

                    else -> {
                        snackDuration = SnackbarDuration.Long
                        snackMessage = resources.getString(R.string.save_failed_exit, result.exitCode)
                        dirty = true
                    }
                }
            } catch (e: Exception) {
                snackDuration = SnackbarDuration.Long
                snackMessage = resources.getString(R.string.save_failed_error, e.message ?: "")
                dirty = true
            }
            saving = false
        }
    }
}

@Composable
private fun TargetGroupHeader(
    group: TargetListGroup,
    count: Int,
) {
    val title =
        when (group) {
            TargetListGroup.Configured -> stringResource(R.string.target_group_configured)
            TargetListGroup.OtherApps -> stringResource(R.string.target_group_other_apps)
        }
    Text(
        text = stringResource(R.string.target_group_header, title, count),
        style = MaterialTheme.typography.labelLarge,
        color = MaterialTheme.colorScheme.primary,
        fontWeight = FontWeight.SemiBold,
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(start = 16.dp, top = 14.dp, end = 16.dp, bottom = 6.dp),
    )
}

/**
 * Filter chips shown inline above the Apps list: sort order, show-system, and
 * RU-only. These used to be a top-bar filter dropdown; moving them into the
 * list frees the app bar so its remaining actions (Search, Refresh, Settings)
 * fit on narrow / high-density screens without crowding. The row scrolls
 * horizontally so the chips always stay on a single line (never wrap).
 */
@Composable
internal fun TargetFilterChips(
    showSystem: Boolean,
    showRussianOnly: Boolean,
    sortMode: TargetListSortMode,
    onToggleSystem: () -> Unit,
    onToggleRussianOnly: () -> Unit,
    onSortModeChange: (TargetListSortMode) -> Unit,
    modifier: Modifier = Modifier,
) {
    val configuredFirst = sortMode == TargetListSortMode.ConfiguredFirst
    Row(
        modifier =
            modifier
                .fillMaxWidth()
                .horizontalScroll(rememberScrollState()),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        // "Configured first" sort is the default, so its chip is selected out of
        // the box; tapping it off falls back to alphabetical order.
        FilterChip(
            selected = configuredFirst,
            onClick = {
                onSortModeChange(
                    if (configuredFirst) {
                        TargetListSortMode.Alphabetical
                    } else {
                        TargetListSortMode.ConfiguredFirst
                    },
                )
            },
            label = { Text(stringResource(R.string.sort_configured_first)) },
        )
        FilterChip(
            selected = showSystem,
            onClick = onToggleSystem,
            label = { Text(stringResource(R.string.filter_show_system)) },
        )
        FilterChip(
            selected = showRussianOnly,
            onClick = onToggleRussianOnly,
            label = { Text(stringResource(R.string.filter_russian_only)) },
        )
    }
}

/**
 * Alphabetical fast-scrollbar shown only while dragging. Extracted verbatim
 * from the three picker screens — the indicator bubble shows the first
 * letter of the row currently at the top of the viewport.
 */
@Composable
internal fun BoxScope.AppListScrollbar(
    listState: LazyListState,
    firstVisibleLabel: () -> String,
) {
    val interactionSource = remember { MutableInteractionSource() }
    val isDragging by interactionSource.collectIsDraggedAsState()
    val indicatorAlpha by animateFloatAsState(
        if (isDragging) 1f else 0f,
        label = "indicatorAlpha",
    )
    VerticalScrollbar(
        adapter = rememberScrollbarAdapter(scrollState = listState),
        interactionSource = interactionSource,
        style = defaultMaterialScrollbarStyle(),
        enablePressToScroll = false,
        modifier =
            Modifier
                .align(Alignment.TopEnd)
                .fillMaxHeight(),
        indicator = { position, isVisible ->
            val firstChar = firstVisibleLabel()
            Box(
                modifier =
                    Modifier
                        .align(Alignment.TopEnd)
                        .padding(end = IndicatorConstants.Default.PADDING)
                        .graphicsLayer {
                            val y = -(IndicatorConstants.Default.MIN_HEIGHT / 2).toPx()
                            translationY = (y + position).coerceAtLeast(0f)
                            alpha = indicatorAlpha
                        },
            ) {
                val indicatorColor =
                    if (isVisible) MaterialTheme.colorScheme.primary else Color.Transparent
                val textColor =
                    if (isVisible) MaterialTheme.colorScheme.onPrimary else Color.Transparent
                Box(
                    modifier =
                        Modifier
                            .defaultMinSize(
                                minHeight = IndicatorConstants.Default.MIN_HEIGHT,
                                minWidth = IndicatorConstants.Default.MIN_WIDTH,
                            ).graphicsLayer {
                                clip = true
                                shape = IndicatorConstants.Default.SHAPE
                            }.drawBehind { drawRect(indicatorColor) },
                )
                Text(
                    text = firstChar,
                    color = textColor,
                    textAlign = TextAlign.Center,
                    style = MaterialTheme.typography.titleMedium,
                    modifier =
                        Modifier
                            .align(Alignment.CenterEnd)
                            .wrapContentHeight()
                            .padding(end = IndicatorConstants.Default.PADDING)
                            .width(IndicatorConstants.Default.MIN_HEIGHT),
                )
            }
        },
    )
}

/**
 * Shared row chrome: app icon, label (with profile suffix), monospace
 * package name, and a chip strip. The per-screen `chips` slot and the
 * row-level click behaviour (passed via [modifier]) are all that differ.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun TargetRowShell(
    label: String,
    packageName: String,
    icon: Drawable?,
    userIds: List<Int>,
    userNames: Map<Int, String>,
    modifier: Modifier = Modifier,
    chips: @Composable () -> Unit,
) {
    Row(
        modifier =
            modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        icon?.let { drawable ->
            Image(
                bitmap = drawable.toBitmap(48, 48).asImageBitmap(),
                contentDescription = null,
                modifier = Modifier.size(40.dp),
            )
            Spacer(Modifier.width(12.dp))
        }
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = labelWithUsers(label, userIds, userNames),
                style = MaterialTheme.typography.bodyLarge,
                fontWeight = FontWeight.Medium,
            )
            Text(
                text = packageName,
                style = MaterialTheme.typography.bodySmall,
                fontFamily = FontFamily.Monospace,
                fontSize = 12.sp,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(4.dp))
            FlowRow(
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                chips()
            }
        }
    }
}

/**
 * Toggle chip used by every picker row (Java, Native, Apps, Ports).
 * [available] gates interactivity without changing the visual — used when
 * a role's backend module isn't installed.
 */
@Composable
internal fun TargetChip(
    label: String,
    enabled: Boolean,
    available: Boolean = true,
    onClick: () -> Unit,
) {
    val containerColor = if (enabled) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceVariant
    val contentColor = if (enabled) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurfaceVariant
    Surface(
        shape = RoundedCornerShape(4.dp),
        color = containerColor,
        modifier = Modifier.clickable(enabled = available, onClick = onClick),
    ) {
        Text(
            text = label,
            style = MaterialTheme.typography.labelSmall,
            fontWeight = FontWeight.Bold,
            color = contentColor,
            modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
        )
    }
}
