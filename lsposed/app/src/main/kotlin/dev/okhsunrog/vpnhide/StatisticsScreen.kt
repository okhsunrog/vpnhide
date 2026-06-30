package dev.okhsunrog.vpnhide

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.BarChart
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.okhsunrog.vpnhide.generated.HookIds
import dev.okhsunrog.vpnhide.ui.components.EnhancedButton
import dev.okhsunrog.vpnhide.ui.components.EnhancedCard
import dev.okhsunrog.vpnhide.ui.components.EnhancedOutlinedButton
import dev.okhsunrog.vpnhide.ui.components.GroupedCard
import dev.okhsunrog.vpnhide.ui.components.IconBubble
import dev.okhsunrog.vpnhide.ui.components.MetricTile
import dev.okhsunrog.vpnhide.ui.components.SectionHeader
import dev.okhsunrog.vpnhide.ui.theme.AppColors
import kotlinx.coroutines.delay

@Composable
fun StatisticsScreen(modifier: Modifier = Modifier) {
    val scope = rememberCoroutineScope()
    val state by StatisticsCache.state.collectAsState()
    val loadError by StatisticsCache.error.collectAsState()
    var detailApp by remember { mutableStateOf<AppProbeStats?>(null) }
    var captureBaseline by remember { mutableStateOf<Map<Pair<Long, Long>, Long>?>(null) }
    var captureStartMs by remember { mutableLongStateOf(0L) }
    var nowMs by remember { mutableLongStateOf(0L) }

    LaunchedEffect(Unit) {
        StatisticsCache.ensureLoaded(scope)
    }
    // Tick the elapsed clock while a capture session is active.
    LaunchedEffect(captureBaseline != null) {
        while (captureBaseline != null) {
            nowMs = System.currentTimeMillis()
            delay(1000)
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

        val selfPackage = LocalContext.current.packageName
        val appStats = remember(s, selfPackage) { buildAppProbeStats(s, selfPackage) }
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
        val shownApps = capture?.apps ?: appStats

        detailApp?.let { app ->
            AppProbeDetailDialog(app = app, onDismiss = { detailApp = null })
        }

        StatisticsHeroCard(state = s, appCount = appStats.size, methodCount = methodCount)
        Spacer(Modifier.height(20.dp))

        CaptureControlCard(
            capturing = capturing,
            elapsedMs = if (capturing) (nowMs - captureStartMs).coerceAtLeast(0L) else 0L,
            capturedAppCount = capture?.apps?.size ?: 0,
            onStart = {
                captureBaseline = snapshotCounters(s)
                captureStartMs = System.currentTimeMillis()
                nowMs = captureStartMs
            },
            onStop = { captureBaseline = null },
            onRefresh = { StatisticsCache.refresh(scope) },
        )
        Spacer(Modifier.height(20.dp))

        SectionHeader(stringResource(R.string.statistics_backends))
        Spacer(Modifier.height(8.dp))
        Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
            s.backends.forEachIndexed { index, backend ->
                BackendSummaryCard(backend, index = index, count = s.backends.size)
            }
        }

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

        if (capturing || shownApps.isNotEmpty()) {
            Spacer(Modifier.height(20.dp))
            SectionHeader(
                stringResource(
                    if (capturing) R.string.statistics_capture_apps_header else R.string.statistics_apps_header,
                ),
            )
            Spacer(Modifier.height(2.dp))
            Text(
                text =
                    stringResource(
                        if (capturing) R.string.statistics_capture_hint else R.string.statistics_apps_caption,
                    ),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(8.dp))
            if (capturing && shownApps.isEmpty()) {
                StatusBanner(
                    text = stringResource(R.string.statistics_capture_empty),
                    containerColor = StatusColors.infoContainer(),
                    contentColor = MaterialTheme.colorScheme.onSurface,
                )
            } else {
                Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
                    shownApps.forEachIndexed { index, app ->
                        AppProbeCard(app, index = index, count = shownApps.size, onClick = { detailApp = app })
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
            Spacer(Modifier.height(16.dp))
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    MetricTile(
                        label = stringResource(R.string.statistics_total_events),
                        value = formatStatCount(state.totalCount),
                        accent = StatusColors.infoAccent,
                        modifier = Modifier.weight(1f),
                    )
                    MetricTile(
                        label = stringResource(R.string.statistics_active_backends),
                        value = "${state.activeBackendCount}/${state.backends.size}",
                        accent = if (state.hasAnyData) StatusColors.successDot else StatusColors.warningAccent,
                        modifier = Modifier.weight(1f),
                    )
                }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    MetricTile(
                        label = stringResource(R.string.statistics_apps_metric),
                        value = appCount.toString(),
                        accent = StatusColors.successDot,
                        modifier = Modifier.weight(1f),
                    )
                    MetricTile(
                        label = stringResource(R.string.statistics_methods_metric),
                        value = methodCount.toString(),
                        accent = StatusColors.successDot,
                        modifier = Modifier.weight(1f),
                    )
                }
            }
        }
    }
}

@Composable
private fun CaptureControlCard(
    capturing: Boolean,
    elapsedMs: Long,
    capturedAppCount: Int,
    onStart: () -> Unit,
    onStop: () -> Unit,
    onRefresh: () -> Unit,
) {
    EnhancedCard(modifier = Modifier.fillMaxWidth(), color = AppColors.cardContainer) {
        Column(
            modifier = Modifier.padding(16.dp).fillMaxWidth(),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Text(
                text = stringResource(R.string.statistics_capture_title),
                style = MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.Bold,
            )
            if (!capturing) {
                Text(
                    text = stringResource(R.string.statistics_capture_intro),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                EnhancedButton(onClick = onStart, modifier = Modifier.fillMaxWidth()) {
                    Text(stringResource(R.string.statistics_capture_start))
                }
            } else {
                Text(
                    text =
                        stringResource(
                            R.string.statistics_capture_active,
                            formatElapsed(elapsedMs),
                            capturedAppCount,
                        ),
                    style = MaterialTheme.typography.bodyMedium,
                    fontWeight = FontWeight.Medium,
                )
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    EnhancedButton(onClick = onRefresh, modifier = Modifier.weight(1f)) {
                        Text(stringResource(R.string.action_refresh))
                    }
                    EnhancedOutlinedButton(onClick = onStop, modifier = Modifier.weight(1f)) {
                        Text(stringResource(R.string.statistics_capture_stop))
                    }
                }
            }
        }
    }
}

private fun formatElapsed(ms: Long): String {
    val totalSeconds = ms / 1000
    return "%d:%02d".format(totalSeconds / 60, totalSeconds % 60)
}

@Composable
private fun BackendSummaryCard(
    backend: BackendStatistics,
    index: Int,
    count: Int,
) {
    val health = backendHealth(backend)
    val visual = backendHealthVisual(health)
    GroupedCard(
        index = index,
        count = count,
        modifier = Modifier.fillMaxWidth(),
        color = AppColors.cardContainer,
    ) {
        Row(
            modifier = Modifier.padding(16.dp).fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            BackendBadge(
                text = backendBadge(backend.backend),
                accentColor = visual.accent,
                containerColor = visual.container,
            )
            Spacer(Modifier.width(14.dp))
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
                )
            }
            Spacer(Modifier.width(10.dp))
            Column(horizontalAlignment = Alignment.End) {
                Text(
                    text = formatStatCount(backend.totalCount),
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.Bold,
                    color = visual.accent,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    text = stringResource(R.string.statistics_events),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun AppProbeCard(
    app: AppProbeStats,
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
                Column(Modifier.weight(1f)) {
                    Text(
                        text = appLabel(app),
                        style = MaterialTheme.typography.titleSmall,
                        fontWeight = FontWeight.Bold,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Text(
                        text = appSurfacesText(app),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
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
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(appLabel(app)) },
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
private fun BackendBadge(
    text: String,
    accentColor: Color,
    containerColor: Color,
) {
    Box(
        modifier =
            Modifier
                .size(46.dp)
                .clip(CircleShape)
                .background(containerColor),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = text,
            style = MaterialTheme.typography.labelLarge,
            fontWeight = FontWeight.Bold,
            color = accentColor,
            maxLines = 1,
        )
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

private fun backendBadge(backend: HookIds.Backend): String =
    when (backend) {
        HookIds.Backend.KMOD -> "K"
        HookIds.Backend.KPM -> "KPM"
        HookIds.Backend.ZYGISK -> "Z"
        HookIds.Backend.LSPOSED -> "J"
    }

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
