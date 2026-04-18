package dev.okhsunrog.vpnhide

import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.graphics.drawable.Drawable
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsDraggedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
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
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.graphics.drawable.toBitmap
import io.github.oikvpqya.compose.fastscroller.VerticalScrollbar
import io.github.oikvpqya.compose.fastscroller.indicator.IndicatorConstants
import io.github.oikvpqya.compose.fastscroller.material3.defaultMaterialScrollbarStyle
import io.github.oikvpqya.compose.fastscroller.rememberScrollbarAdapter
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

internal data class PortsEntry(
    val packageName: String,
    val label: String,
    val icon: Drawable?,
    val isSystem: Boolean,
    val observer: Boolean = false,
)

@Composable
fun PortsHidingScreen(
    searchQuery: String,
    showSystem: Boolean,
    showRussianOnly: Boolean,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val pm = context.packageManager

    var moduleInstalled by remember { mutableStateOf<Boolean?>(null) }
    var allApps by remember { mutableStateOf<List<PortsEntry>>(emptyList()) }
    var loading by remember { mutableStateOf(true) }
    var saving by remember { mutableStateOf(false) }
    var dirty by remember { mutableStateOf(false) }
    var snackMessage by remember { mutableStateOf<String?>(null) }
    val snackbarHostState = remember { SnackbarHostState() }

    LaunchedEffect(snackMessage) {
        snackMessage?.let {
            snackbarHostState.showSnackbar(it)
            snackMessage = null
        }
    }

    LaunchedEffect(Unit) {
        withContext(Dispatchers.IO) {
            // Read module.prop rather than [ -d ]: on KSU-Next a pending-removal
            // module keeps the directory with a `remove` flag file until reboot;
            // an unreadable module.prop signals "not really installed anymore".
            val (exitCode, _) = suExec("cat $PORTS_MODULE_DIR/module.prop >/dev/null 2>&1")
            val installed = exitCode == 0
            moduleInstalled = installed
            if (!installed) {
                loading = false
                return@withContext
            }

            // observers.txt stores package names (resolved to UIDs at apply time
            // inside the module script). Read them directly — no UID mapping needed.
            val (_, raw) = suExec("cat $PORTS_OBSERVERS_FILE 2>/dev/null || true")
            val observerNames =
                raw
                    .lines()
                    .map { it.trim() }
                    .filter { it.isNotEmpty() && !it.startsWith("#") }
                    .toSet()

            val selfPkg = context.packageName
            val installedApps = pm.getInstalledApplications(PackageManager.GET_META_DATA)
            val entries =
                installedApps
                    .filter { it.packageName != selfPkg }
                    .map { info ->
                        val label =
                            try {
                                pm.getApplicationLabel(info).toString()
                            } catch (_: Exception) {
                                info.packageName
                            }
                        val icon =
                            try {
                                pm.getApplicationIcon(info)
                            } catch (_: Exception) {
                                null
                            }
                        val isSystem = (info.flags and ApplicationInfo.FLAG_SYSTEM) != 0
                        val pkg = info.packageName
                        PortsEntry(
                            packageName = pkg,
                            label = label,
                            icon = icon,
                            isSystem = isSystem,
                            observer = pkg in observerNames,
                        )
                    }.sortedBy { it.label.lowercase() }

            allApps = entries
            loading = false
        }
    }

    if (moduleInstalled == false) {
        NotInstalledCard(modifier = modifier)
        return
    }
    // moduleInstalled == null → still detecting; the `loading` spinner below covers it.

    val filteredApps =
        remember(allApps, searchQuery, showSystem, showRussianOnly) {
            val q = searchQuery.trim().lowercase()
            allApps.filter { app ->
                (showSystem || !app.isSystem || app.observer) &&
                    (!showRussianOnly || isRussianApp(app.packageName, app.label)) &&
                    (q.isEmpty() || app.label.lowercase().contains(q) || app.packageName.lowercase().contains(q))
            }
        }

    val observerCount = remember(allApps) { allApps.count { it.observer } }

    Column(modifier = modifier.fillMaxSize()) {
        if (loading) {
            Box(
                modifier = Modifier.fillMaxSize(),
                contentAlignment = Alignment.Center,
            ) {
                CircularProgressIndicator()
            }
        } else {
            val listState = rememberLazyListState()
            Box(modifier = Modifier.weight(1f)) {
                LazyColumn(
                    state = listState,
                    modifier = Modifier.fillMaxSize(),
                ) {
                    item {
                        Box(modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp)) {
                            HelpAccordion(
                                prefKey = "apps_ports",
                                title = stringResource(R.string.ports_help_title),
                            ) {
                                Text(
                                    text = stringResource(R.string.ports_hint_role),
                                    style = MaterialTheme.typography.bodyMedium,
                                )
                                Text(
                                    text = stringResource(R.string.ports_hint_safe),
                                    style = MaterialTheme.typography.bodyMedium,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                                Text(
                                    text = stringResource(R.string.ports_hint_reboot),
                                    style = MaterialTheme.typography.bodyMedium,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                        }
                    }
                    items(filteredApps, key = { it.packageName }) { app ->
                        PortsAppRow(
                            app = app,
                            onToggle = {
                                allApps =
                                    allApps.map {
                                        if (it.packageName != app.packageName) {
                                            it
                                        } else {
                                            it.copy(observer = !it.observer)
                                        }
                                    }
                                dirty = true
                            },
                        )
                    }
                }
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
                        val firstChar =
                            filteredApps
                                .getOrNull(listState.firstVisibleItemIndex)
                                ?.label
                                ?.firstOrNull()
                                ?.uppercase() ?: ""
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
            Surface(tonalElevation = 3.dp) {
                Row(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 16.dp, vertical = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = stringResource(R.string.ports_count, observerCount),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.weight(1f),
                    )
                    Button(
                        onClick = {
                            saving = true
                            dirty = false
                        },
                        enabled = dirty && !saving,
                    ) {
                        Text(stringResource(R.string.btn_save))
                    }
                }
            }
        }

        SnackbarHost(snackbarHostState)
    }

    if (saving) {
        LaunchedEffect(Unit) {
            val observerPkgs = allApps.filter { it.observer }.map { it.packageName }.sorted()
            val header = context.getString(R.string.save_header_comment)
            try {
                val (exitCode, _) = suExecAsync(buildPortsSaveCommand(header, observerPkgs))
                if (exitCode == 0) {
                    snackMessage = context.getString(R.string.ports_save_success, observerPkgs.size)
                } else if (exitCode == -1) {
                    snackMessage = context.getString(R.string.save_failed_root)
                    dirty = true
                } else {
                    snackMessage = context.getString(R.string.save_failed_exit, exitCode)
                    dirty = true
                }
            } catch (e: Exception) {
                snackMessage = context.getString(R.string.save_failed_error, e.message ?: "")
                dirty = true
            }
            saving = false
        }
    }
}

private fun buildPortsSaveCommand(
    header: String,
    observerPkgs: List<String>,
): String {
    // observers.txt stores package names (one per line). UID resolution lives
    // entirely inside vpnhide_ports_apply.sh so app reinstalls (which rotate
    // the UID) get picked up automatically on the next apply.
    val body = (listOf(header) + observerPkgs).joinToString(separator = "\n", postfix = "\n")
    val b64 = android.util.Base64.encodeToString(body.toByteArray(), android.util.Base64.NO_WRAP)
    return listOf(
        "mkdir -p /data/adb/vpnhide_ports",
        "echo '$b64' | base64 -d > $PORTS_OBSERVERS_FILE",
        "chmod 644 $PORTS_OBSERVERS_FILE",
        "sh $PORTS_APPLY_SCRIPT",
    ).joinToString(" && ")
}

@Composable
private fun NotInstalledCard(modifier: Modifier = Modifier) {
    Box(
        modifier = modifier.fillMaxSize().padding(24.dp),
        contentAlignment = Alignment.Center,
    ) {
        Card(
            colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant),
        ) {
            Column(
                modifier = Modifier.padding(20.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text(
                    text = stringResource(R.string.ports_module_not_installed_title),
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.Bold,
                )
                Text(
                    text = stringResource(R.string.ports_module_not_installed_body),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

@Composable
private fun PortsAppRow(
    app: PortsEntry,
    onToggle: () -> Unit,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onToggle)
                .padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        app.icon?.let { drawable ->
            Image(
                bitmap = drawable.toBitmap(48, 48).asImageBitmap(),
                contentDescription = null,
                modifier = Modifier.size(40.dp),
            )
            Spacer(Modifier.width(12.dp))
        }
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = app.label,
                style = MaterialTheme.typography.bodyLarge,
                fontWeight = FontWeight.Medium,
            )
            Text(
                text = app.packageName,
                style = MaterialTheme.typography.bodySmall,
                fontFamily = FontFamily.Monospace,
                fontSize = 12.sp,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(4.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                PortsChip(
                    label = stringResource(R.string.ports_chip),
                    enabled = app.observer,
                    onClick = onToggle,
                )
            }
        }
    }
}

@Composable
private fun PortsChip(
    label: String,
    enabled: Boolean,
    onClick: () -> Unit,
) {
    val containerColor = if (enabled) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceVariant
    val contentColor = if (enabled) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurfaceVariant
    Surface(
        shape = RoundedCornerShape(4.dp),
        color = containerColor,
        modifier = Modifier.clickable(onClick = onClick),
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
