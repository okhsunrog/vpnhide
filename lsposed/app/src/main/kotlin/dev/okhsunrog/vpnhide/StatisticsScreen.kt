package dev.okhsunrog.vpnhide

import android.graphics.drawable.Drawable
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Android
import androidx.compose.material.icons.filled.BarChart
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.core.graphics.drawable.toBitmap
import dev.okhsunrog.vpnhide.generated.HookIds
import dev.okhsunrog.vpnhide.ui.components.EnhancedButton
import dev.okhsunrog.vpnhide.ui.components.EnhancedCard
import dev.okhsunrog.vpnhide.ui.components.EnhancedOutlinedButton
import dev.okhsunrog.vpnhide.ui.components.GroupedCard
import dev.okhsunrog.vpnhide.ui.components.IconBubble
import dev.okhsunrog.vpnhide.ui.components.SectionHeader
import dev.okhsunrog.vpnhide.ui.theme.AppColors
import kotlinx.coroutines.delay

// How often the live capture session re-reads the backend counters. Comfortably
// longer than a typical root-snapshot read so polls don't stack up.
private const val CAPTURE_POLL_MS = 2000L

@Composable
fun StatisticsScreen(modifier: Modifier = Modifier) {
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    val state by StatisticsCache.state.collectAsState()
    val loadError by StatisticsCache.error.collectAsState()
    val installedApps by AppListCache.apps.collectAsState()
    var detailApp by remember { mutableStateOf<AppProbeStats?>(null) }
    var captureBaseline by remember { mutableStateOf<Map<Pair<Long, Long>, Long>?>(null) }
    var captureStartMs by remember { mutableLongStateOf(0L) }
    // Results of the last finished session, frozen at Stop. Non-null means we're
    // in the "stopped" state: the live session is over but its probes stay on
    // screen for review until the user starts a new capture or clears them.
    var frozenCapture by remember { mutableStateOf<FrozenCapture?>(null) }
    var nowMs by remember { mutableLongStateOf(0L) }

    LaunchedEffect(Unit) {
        StatisticsCache.ensureLoaded(scope)
        // Resolve UID → app icon + friendly label for the per-app list, the
        // same package scan the Protection tab uses (cached app-wide).
        AppListCache.ensureLoaded(scope, context)
    }
    // Tick the elapsed clock while a capture session is active.
    LaunchedEffect(captureBaseline != null) {
        while (captureBaseline != null) {
            nowMs = System.currentTimeMillis()
            delay(1000)
        }
    }
    // Live capture: re-read the counters every couple of seconds while a session
    // is active so probes appear on their own as the user exercises a target app
    // — no manual refresh. The coroutine keeps running while the app is
    // backgrounded (composition is retained), so polling continues while the
    // user is inside the target app. Skip a tick if a read is still in flight so
    // a slow root snapshot doesn't get cancelled-and-restarted forever.
    LaunchedEffect(captureBaseline != null) {
        while (captureBaseline != null) {
            delay(CAPTURE_POLL_MS)
            if (!StatisticsCache.loading.value) StatisticsCache.refresh(scope)
        }
    }

    Column(
        modifier =
            modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp),
    ) {
        Spacer(Modifier.height(12.dp))

        val s = state
        val error = loadError
        if (error != null) {
            StatisticsLoadErrorCard(
                previousDataVisible = s != null,
                onRetry = { StatisticsCache.refresh(scope) },
            )
            Spacer(Modifier.height(12.dp))
            if (s == null) return@Column
        }

        if (s == null) {
            Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
                CircularProgressIndicator()
            }
            return@Column
        }

        val selfPackage = context.packageName
        val appStats = remember(s, selfPackage) { buildAppProbeStats(s, selfPackage) }
        val appsByPackage =
            remember(installedApps) { installedApps?.associateBy(AppSummary::packageName).orEmpty() }
        val methodCount = remember(appStats) { appStats.flatMap { it.byMethod.keys }.toSet().size }

        val capturing = captureBaseline != null
        // Memoize the diff so the once-per-second elapsed-clock tick (which flips
        // nowMs and recomposes the whole screen) does not rebuild the per-app
        // rollup every second; it only changes when the baseline or stats do.
        val capture =
            remember(captureBaseline, s, selfPackage) {
                captureBaseline?.let { diffCapture(it, s, selfPackage) }
            }
        val backendReset = capture?.backendReset == true
        // Backend restarted mid-session (a counter dropped): re-baseline so the
        // deltas restart from the fresh counter values instead of going negative.
        LaunchedEffect(backendReset) {
            if (backendReset) {
                captureBaseline = snapshotCounters(s)
                captureStartMs = System.currentTimeMillis()
            }
        }
        // Three-state session: live deltas while capturing, the frozen result
        // once stopped, and the all-time list when idle.
        val captureMode =
            when {
                capturing -> CaptureMode.Live
                frozenCapture != null -> CaptureMode.Stopped
                else -> CaptureMode.Idle
            }
        val sessionApps =
            when (captureMode) {
                CaptureMode.Live -> capture?.apps.orEmpty()
                CaptureMode.Stopped -> frozenCapture?.apps.orEmpty()
                CaptureMode.Idle -> null
            }
        val shownApps = sessionApps ?: appStats

        detailApp?.let { app ->
            AppProbeDetailDialog(
                app = app,
                appName = resolveAppSummary(app, appsByPackage)?.label ?: appLabel(app),
                onDismiss = { detailApp = null },
            )
        }

        StatisticsHeroCard(state = s, appCount = appStats.size, methodCount = methodCount)
        Spacer(Modifier.height(20.dp))

        CaptureControlCard(
            mode = captureMode,
            elapsedMs =
                when (captureMode) {
                    CaptureMode.Live -> (nowMs - captureStartMs).coerceAtLeast(0L)
                    CaptureMode.Stopped -> frozenCapture?.durationMs ?: 0L
                    CaptureMode.Idle -> 0L
                },
            sessionAppCount = sessionApps?.size ?: 0,
            onStart = {
                captureBaseline = snapshotCounters(s)
                captureStartMs = System.currentTimeMillis()
                nowMs = captureStartMs
                frozenCapture = null
            },
            onStop = {
                // Freeze the current deltas so they survive the stop, then end
                // the live session (clearing the baseline halts the poll loop).
                frozenCapture =
                    FrozenCapture(
                        apps = capture?.apps.orEmpty(),
                        durationMs = (System.currentTimeMillis() - captureStartMs).coerceAtLeast(0L),
                    )
                captureBaseline = null
            },
            onClear = { frozenCapture = null },
        )

        if (!s.hasAnyData) {
            Spacer(Modifier.height(12.dp))
            StatusBanner(
                text =
                    stringResource(
                        if (s.backends.any { it.unavailableReason != null }) {
                            R.string.statistics_no_data_supported_backends
                        } else {
                            R.string.statistics_no_data
                        },
                    ),
                containerColor = StatusColors.infoContainer(),
                contentColor = MaterialTheme.colorScheme.onSurface,
            )
        }

        val showingSession = captureMode != CaptureMode.Idle
        if (showingSession || shownApps.isNotEmpty()) {
            Spacer(Modifier.height(20.dp))
            SectionHeader(
                stringResource(
                    if (showingSession) R.string.statistics_capture_apps_header else R.string.statistics_apps_header,
                ),
            )
            Spacer(Modifier.height(2.dp))
            Text(
                text =
                    stringResource(
                        if (showingSession) R.string.statistics_capture_hint else R.string.statistics_apps_caption,
                    ),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(8.dp))
            if (showingSession && shownApps.isEmpty()) {
                StatusBanner(
                    text =
                        stringResource(
                            if (captureMode == CaptureMode.Stopped) {
                                R.string.statistics_capture_stopped_empty
                            } else {
                                R.string.statistics_capture_empty
                            },
                        ),
                    containerColor = StatusColors.infoContainer(),
                    contentColor = MaterialTheme.colorScheme.onSurface,
                )
            } else {
                Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
                    shownApps.forEachIndexed { index, app ->
                        AppProbeCard(
                            app = app,
                            summary = resolveAppSummary(app, appsByPackage),
                            index = index,
                            count = shownApps.size,
                            onClick = { detailApp = app },
                        )
                    }
                }
            }
        }

        // The active native backend can't report counters (Zygisk): keep the
        // explanatory note so the per-app list reading "Java only" makes sense.
        s.backends
            .firstOrNull { it.unavailableReason != null }
            ?.unavailableReason
            ?.let { reason ->
                Spacer(Modifier.height(12.dp))
                StatusBanner(
                    text = statisticsUnavailableText(reason),
                    containerColor = StatusColors.infoContainer(),
                    contentColor = MaterialTheme.colorScheme.onSurface,
                )
            }

        Spacer(Modifier.height(16.dp))
    }
}

@Composable
private fun StatisticsLoadErrorCard(
    previousDataVisible: Boolean,
    onRetry: () -> Unit,
) {
    EnhancedCard(
        modifier = Modifier.fillMaxWidth(),
        color = StatusColors.errorContainer(),
        contentColor = MaterialTheme.colorScheme.onSurface,
    ) {
        Column(Modifier.padding(16.dp)) {
            Text(
                text = stringResource(R.string.statistics_load_failed_title),
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Bold,
                color = StatusColors.errorHeader(),
            )
            Spacer(Modifier.height(6.dp))
            Text(
                text =
                    stringResource(
                        if (previousDataVisible) {
                            R.string.statistics_refresh_failed_message
                        } else {
                            R.string.statistics_load_failed_message
                        },
                    ),
                style = MaterialTheme.typography.bodyMedium,
            )
            Spacer(Modifier.height(12.dp))
            EnhancedButton(onClick = onRetry) {
                Text(stringResource(R.string.vpn_off_retry))
            }
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun StatisticsHeroCard(
    state: StatisticsState,
    appCount: Int,
    methodCount: Int,
) {
    EnhancedCard(
        modifier = Modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.extraLarge,
        color = AppColors.cardContainer,
    ) {
        Column(modifier = Modifier.padding(18.dp).fillMaxWidth()) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                IconBubble(
                    icon = Icons.Default.BarChart,
                    tint = StatusColors.infoAccent,
                    container = StatusColors.infoContainer(),
                )
                Spacer(Modifier.width(16.dp))
                Column(Modifier.weight(1f)) {
                    Text(
                        text = stringResource(R.string.statistics_title),
                        style = MaterialTheme.typography.headlineSmall,
                        fontWeight = FontWeight.Bold,
                    )
                    Spacer(Modifier.height(2.dp))
                    Text(
                        text = stringResource(R.string.statistics_subtitle),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            Spacer(Modifier.height(18.dp))
            // Headline: the grand total + its unit kept together as one block (so
            // "events" is never stranded), with the apps/methods summary flowing
            // onto the same line when it fits and wrapping below when it doesn't.
            // The inter-item gap forces the longer Russian strings to wrap on a
            // narrow screen instead of clipping.
            FlowRow(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(16.dp),
                verticalArrangement = Arrangement.spacedBy(4.dp),
                itemVerticalAlignment = Alignment.Bottom,
            ) {
                Row(verticalAlignment = Alignment.Bottom) {
                    Text(
                        text = formatStatCount(state.totalCount),
                        style = MaterialTheme.typography.displaySmall,
                        fontWeight = FontWeight.Bold,
                        color = StatusColors.infoAccent,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.alignByBaseline(),
                    )
                    Spacer(Modifier.width(8.dp))
                    Text(
                        text = stringResource(R.string.statistics_events),
                        style = MaterialTheme.typography.titleMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.alignByBaseline(),
                    )
                }
                Text(
                    text = stringResource(R.string.statistics_apps_methods, appCount, methodCount),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            // Per-backend breakdown folded into the hero so there's no separate
            // mid-screen "Backends" section: one quiet row per active backend
            // (health dot + name + hooks + its event count).
            if (state.backends.isNotEmpty()) {
                Spacer(Modifier.height(16.dp))
                Column(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .clip(MaterialTheme.shapes.medium)
                            .background(AppColors.cardContainerStrong),
                ) {
                    state.backends.forEachIndexed { index, backend ->
                        if (index > 0) {
                            HorizontalDivider(
                                modifier = Modifier.padding(horizontal = 14.dp),
                                color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.5f),
                            )
                        }
                        BackendStripRow(backend)
                    }
                }
            }
        }
    }
}

// One compact line in the hero's backend strip: a health-coloured dot, the
// backend name with its "OK · hooks: N" detail, and its event count (or "—"
// when the backend can't report counters, e.g. Zygisk).
@Composable
private fun BackendStripRow(backend: BackendStatistics) {
    val visual = backendHealthVisual(backendHealth(backend))
    Row(
        modifier = Modifier.fillMaxWidth().padding(horizontal = 14.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(
            modifier =
                Modifier
                    .size(9.dp)
                    .clip(CircleShape)
                    .background(visual.accent),
        )
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Text(
                text = backendName(backend.backend),
                style = MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.Bold,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                text = backendDetailText(backend, visual),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Spacer(Modifier.width(10.dp))
        Text(
            text = if (backend.unavailableReason != null) "—" else formatStatCount(backend.totalCount),
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.Bold,
            color =
                if (backend.unavailableReason != null) {
                    MaterialTheme.colorScheme.onSurfaceVariant
                } else {
                    visual.accent
                },
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

private enum class CaptureMode { Idle, Live, Stopped }

@Composable
private fun CaptureControlCard(
    mode: CaptureMode,
    elapsedMs: Long,
    sessionAppCount: Int,
    onStart: () -> Unit,
    onStop: () -> Unit,
    onClear: () -> Unit,
) {
    EnhancedCard(modifier = Modifier.fillMaxWidth(), color = AppColors.cardContainer) {
        Column(
            modifier = Modifier.padding(16.dp).fillMaxWidth(),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(
                text = stringResource(R.string.statistics_capture_title),
                style = MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.Bold,
            )
            when (mode) {
                CaptureMode.Idle -> {
                    CaptureIdleBody(onStart = onStart)
                }

                CaptureMode.Live -> {
                    CaptureLiveBody(
                        elapsedMs = elapsedMs,
                        sessionAppCount = sessionAppCount,
                        onStop = onStop,
                    )
                }

                CaptureMode.Stopped -> {
                    CaptureStoppedBody(
                        elapsedMs = elapsedMs,
                        sessionAppCount = sessionAppCount,
                        onStart = onStart,
                        onClear = onClear,
                    )
                }
            }
        }
    }
}

@Composable
private fun CaptureIdleBody(onStart: () -> Unit) {
    Text(
        text = stringResource(R.string.statistics_capture_intro),
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        CaptureStep(1, stringResource(R.string.statistics_capture_step1))
        CaptureStep(2, stringResource(R.string.statistics_capture_step2))
        CaptureStep(3, stringResource(R.string.statistics_capture_step3))
    }
    EnhancedButton(onClick = onStart, modifier = Modifier.fillMaxWidth()) {
        Text(stringResource(R.string.statistics_capture_start))
    }
}

@Composable
private fun CaptureLiveBody(
    elapsedMs: Long,
    sessionAppCount: Int,
    onStop: () -> Unit,
) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Box(
            modifier =
                Modifier
                    .size(8.dp)
                    .clip(CircleShape)
                    .background(StatusColors.successDot),
        )
        Spacer(Modifier.width(8.dp))
        Text(
            text =
                stringResource(
                    R.string.statistics_capture_live,
                    formatElapsed(elapsedMs),
                    sessionAppCount,
                ),
            style = MaterialTheme.typography.bodyMedium,
            fontWeight = FontWeight.Medium,
        )
    }
    Text(
        text = stringResource(R.string.statistics_capture_active_hint),
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    EnhancedOutlinedButton(onClick = onStop, modifier = Modifier.fillMaxWidth()) {
        Text(stringResource(R.string.statistics_capture_stop))
    }
}

@Composable
private fun CaptureStoppedBody(
    elapsedMs: Long,
    sessionAppCount: Int,
    onStart: () -> Unit,
    onClear: () -> Unit,
) {
    Text(
        text =
            stringResource(
                R.string.statistics_capture_captured,
                formatElapsed(elapsedMs),
                sessionAppCount,
            ),
        style = MaterialTheme.typography.bodyMedium,
        fontWeight = FontWeight.Medium,
    )
    Text(
        text = stringResource(R.string.statistics_capture_stopped_note),
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        EnhancedButton(onClick = onStart, modifier = Modifier.weight(1f)) {
            Text(stringResource(R.string.statistics_capture_new))
        }
        EnhancedOutlinedButton(onClick = onClear, modifier = Modifier.weight(1f)) {
            Text(stringResource(R.string.statistics_capture_clear))
        }
    }
}

// One numbered line in the capture how-to. The number sits in a small tinted
// circle so the three-step flow reads at a glance.
@Composable
private fun CaptureStep(
    number: Int,
    text: String,
) {
    Row(verticalAlignment = Alignment.Top) {
        Box(
            modifier =
                Modifier
                    .size(20.dp)
                    .clip(CircleShape)
                    .background(StatusColors.infoContainer()),
            contentAlignment = Alignment.Center,
        ) {
            Text(
                text = number.toString(),
                style = MaterialTheme.typography.labelSmall,
                fontWeight = FontWeight.Bold,
                color = StatusColors.infoAccent,
            )
        }
        Spacer(Modifier.width(10.dp))
        Text(
            text = text,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

private fun formatElapsed(ms: Long): String {
    val totalSeconds = ms / 1000
    return "%d:%02d".format(totalSeconds / 60, totalSeconds % 60)
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun AppProbeCard(
    app: AppProbeStats,
    summary: AppSummary?,
    index: Int,
    count: Int,
    onClick: () -> Unit,
) {
    GroupedCard(
        index = index,
        count = count,
        modifier = Modifier.fillMaxWidth(),
        color = AppColors.cardContainer,
        onClick = onClick,
    ) {
        Column(modifier = Modifier.padding(14.dp).fillMaxWidth()) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                AppStatAvatar(summary?.icon)
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f)) {
                    Text(
                        text = summary?.label ?: appLabel(app),
                        style = MaterialTheme.typography.titleSmall,
                        fontWeight = FontWeight.Bold,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                    AppProbeSubtitle(app = app, resolved = summary != null)
                }
                Spacer(Modifier.width(12.dp))
                Text(
                    text = formatStatCount(app.total),
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.Bold,
                    color = StatusColors.infoAccent,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            if (app.byMethod.isNotEmpty()) {
                Spacer(Modifier.height(10.dp))
                FlowRow(
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                    verticalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    app.byMethod.entries
                        .sortedByDescending { it.value }
                        .forEach { (method, methodCount) ->
                            MethodChip(method = method, count = methodCount)
                        }
                }
            }
        }
    }
}

// Tap-through detail: the exact per-hook breakdown the backends report, so a
// user can see precisely which VPN-detection techniques an app uses (the card
// only shows the folded methods). The friendly method label is the primary
// line; the hook's technical note is the precise secondary line.
@Composable
private fun AppProbeDetailDialog(
    app: AppProbeStats,
    appName: String,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(appName) },
        text = {
            Column(
                modifier =
                    Modifier
                        .heightIn(max = 480.dp)
                        .verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(14.dp),
            ) {
                // Grouped: surface (Java API / Native / Packages, coloured) →
                // detection method (with an explanation of how it reveals a VPN)
                // → the exact hooks behind it when a method folds several.
                app.byHook.entries
                    .groupBy { DetectionMethod.of(it.key).surface }
                    .entries
                    .sortedByDescending { (_, hooks) -> hooks.sumOf { it.value } }
                    .forEach { (surface, surfaceHooks) ->
                        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                            Text(
                                text = surfaceLabel(surface),
                                style = MaterialTheme.typography.labelMedium,
                                fontWeight = FontWeight.Bold,
                                color = surfaceAccentColor(surface),
                            )
                            surfaceHooks
                                .groupBy { DetectionMethod.of(it.key) }
                                .entries
                                .sortedByDescending { (_, hooks) -> hooks.sumOf { it.value } }
                                .forEach { (method, methodHooks) ->
                                    MethodDetailBlock(method = method, hooks = methodHooks)
                                }
                        }
                    }
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text(stringResource(R.string.contact_close)) }
        },
    )
}

@Composable
private fun MethodDetailBlock(
    method: DetectionMethod,
    hooks: List<Map.Entry<HookIds.Hook, Long>>,
) {
    val total = hooks.sumOf { it.value }
    Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                text = stringResource(method.labelRes),
                modifier = Modifier.weight(1f),
                style = MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.Bold,
            )
            Spacer(Modifier.width(10.dp))
            Text(
                text = formatStatCount(total),
                style = MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.Bold,
                color = StatusColors.infoAccent,
            )
        }
        Text(
            text = stringResource(method.descriptionRes),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        // When a method folds several hooks, list the exact ones behind it.
        if (hooks.size > 1) {
            hooks.sortedByDescending { it.value }.forEach { (hook, count) ->
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        text = "·  ${hook.note}",
                        modifier = Modifier.weight(1f),
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Spacer(Modifier.width(8.dp))
                    Text(
                        text = formatStatCount(count),
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }
    }
}

@Composable
private fun MethodChip(
    method: DetectionMethod,
    count: Long,
) {
    Row(
        modifier =
            Modifier
                .clip(MaterialTheme.shapes.small)
                .background(surfaceContainerColor(method.surface))
                .padding(horizontal = 10.dp, vertical = 5.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = stringResource(method.labelRes),
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurface,
        )
        Spacer(Modifier.width(6.dp))
        Text(
            text = formatStatCount(count),
            style = MaterialTheme.typography.labelMedium,
            fontWeight = FontWeight.Bold,
            color = MaterialTheme.colorScheme.onSurface,
        )
    }
}

@Composable
private fun surfaceContainerColor(surface: MethodSurface): Color =
    when (surface) {
        MethodSurface.Java -> StatusColors.infoContainer()
        MethodSurface.Native -> StatusColors.successContainer()
        MethodSurface.Package -> StatusColors.neutralContainer()
    }

@Composable
private fun surfaceAccentColor(surface: MethodSurface): Color =
    when (surface) {
        MethodSurface.Java -> StatusColors.infoAccent
        MethodSurface.Native -> StatusColors.successDot
        MethodSurface.Package -> StatusColors.neutralAccent
    }

@Composable
private fun surfaceLabel(surface: MethodSurface): String =
    stringResource(
        when (surface) {
            MethodSurface.Java -> R.string.surface_java
            MethodSurface.Native -> R.string.surface_native
            MethodSurface.Package -> R.string.surface_package
        },
    )

// First installed-app summary that matches one of this UID's packages, so the
// row can show a real icon + label instead of the bare package name. UIDs with
// no installed match (system uids, uninstalled packages) resolve to null and
// fall back to the package/uid text + a neutral placeholder avatar.
private fun resolveAppSummary(
    app: AppProbeStats,
    byPackage: Map<String, AppSummary>,
): AppSummary? = app.packageNames.firstNotNullOfOrNull { byPackage[it] }

@Composable
private fun AppStatAvatar(icon: Drawable?) {
    if (icon != null) {
        Image(
            bitmap = icon.toBitmap(48, 48).asImageBitmap(),
            contentDescription = null,
            modifier = Modifier.size(40.dp),
        )
    } else {
        Box(
            modifier =
                Modifier
                    .size(40.dp)
                    .clip(RoundedCornerShape(10.dp))
                    .background(AppColors.neutralAccentContainer),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                imageVector = Icons.Default.Android,
                contentDescription = null,
                tint = StatusColors.neutralAccent,
                modifier = Modifier.size(22.dp),
            )
        }
    }
}

// Secondary line under the app label: the package name(s) in monospace once the
// app is resolved (mirrors the Protection rows), otherwise the detection
// surfaces (Java · Native · Apps) since there's no friendlier identity to show.
@Composable
private fun AppProbeSubtitle(
    app: AppProbeStats,
    resolved: Boolean,
) {
    if (resolved && app.packageNames.isNotEmpty()) {
        Text(
            text = app.packageNames.joinToString(", "),
            style = MaterialTheme.typography.bodySmall,
            fontFamily = FontFamily.Monospace,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    } else {
        Text(
            text = appSurfacesText(app),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
private fun appLabel(app: AppProbeStats): String =
    app.packageNames
        .takeIf { it.isNotEmpty() }
        ?.joinToString(", ")
        ?: stringResource(R.string.statistics_unknown_uid, app.uid)

@Composable
private fun appSurfacesText(app: AppProbeStats): String {
    // Resolve labels up front: stringResource can't be called inside the
    // joinToString transform (not a @Composable context).
    val java = stringResource(R.string.surface_java)
    val native = stringResource(R.string.surface_native)
    val pkg = stringResource(R.string.surface_package)
    return app.surfaces.sorted().joinToString(" · ") { surface ->
        when (surface) {
            MethodSurface.Java -> java
            MethodSurface.Native -> native
            MethodSurface.Package -> pkg
        }
    }
}

@Composable
private fun backendName(backend: HookIds.Backend): String =
    stringResource(
        when (backend) {
            HookIds.Backend.KMOD -> R.string.dashboard_backend_kmod
            HookIds.Backend.KPM -> R.string.dashboard_backend_kpm
            HookIds.Backend.ZYGISK -> R.string.dashboard_backend_zygisk
            HookIds.Backend.LSPOSED -> R.string.dashboard_backend_lsposed
        },
    )

private enum class BackendHealth { Ok, Partial, Error, NoData, Unavailable }

private fun backendHealth(backend: BackendStatistics): BackendHealth {
    val status = backend.status
    return when {
        backend.unavailableReason != null -> BackendHealth.Unavailable

        status == null && backend.rows.isEmpty() -> BackendHealth.NoData

        status == null -> BackendHealth.Partial

        status.error ==
            HookIds.StatusError.OK.code
                .toLong()
        -> BackendHealth.Ok

        status.error ==
            HookIds.StatusError.PARTIAL_HOOKS.code
                .toLong()
        -> BackendHealth.Partial

        else -> BackendHealth.Error
    }
}

@Composable
private fun backendDetailText(
    backend: BackendStatistics,
    visual: HealthVisual,
): String =
    backend.unavailableReason?.let { stringResource(R.string.statistics_backend_unavailable_detail) }
        ?: stringResource(
            R.string.statistics_backend_detail,
            visual.label,
            backend.hookedCount,
        )

@Composable
private fun statisticsUnavailableText(reason: StatisticsUnavailableReason): String =
    when (reason) {
        StatisticsUnavailableReason.ZygiskNativeStats -> stringResource(R.string.statistics_zygisk_native_unavailable)
    }

private data class HealthVisual(
    val label: String,
    val accent: Color,
    val container: Color,
)

@Composable
private fun backendHealthVisual(health: BackendHealth): HealthVisual =
    when (health) {
        BackendHealth.Ok -> {
            HealthVisual(
                label = stringResource(R.string.statistics_status_ok),
                accent = StatusColors.successDot,
                container = StatusColors.successContainer(),
            )
        }

        BackendHealth.Partial -> {
            HealthVisual(
                label = stringResource(R.string.statistics_status_partial),
                accent = StatusColors.warningAccent,
                container = StatusColors.warningContainer(),
            )
        }

        BackendHealth.Error -> {
            HealthVisual(
                label = stringResource(R.string.statistics_status_error),
                accent = StatusColors.errorAccent,
                container = StatusColors.errorContainer(),
            )
        }

        BackendHealth.NoData -> {
            HealthVisual(
                label = stringResource(R.string.statistics_status_no_data),
                accent = StatusColors.neutralAccent,
                container = AppColors.neutralAccentContainer,
            )
        }

        BackendHealth.Unavailable -> {
            HealthVisual(
                label = stringResource(R.string.statistics_status_unavailable),
                accent = StatusColors.infoAccent,
                container = StatusColors.infoContainer(),
            )
        }
    }
