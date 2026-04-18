package dev.okhsunrog.vpnhide

import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.graphics.drawable.Drawable
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsDraggedAsState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
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

data class AppEntry(
    val packageName: String,
    val label: String,
    val icon: Drawable?,
    val isSystem: Boolean,
    val kmod: Boolean = false,
    val zygisk: Boolean = false,
    val lsposed: Boolean = false,
) {
    val anySelected get() = kmod || zygisk || lsposed
}

/** Which installed modules are present (detected once at load). */
data class InstalledModules(
    val kmod: Boolean = false,
    val zygisk: Boolean = false,
)

internal enum class Layer { KMOD, ZYGISK, LSPOSED }

@Composable
fun AppPickerScreen(
    searchQuery: String,
    showSystem: Boolean,
    showRussianOnly: Boolean,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val pm = context.packageManager

    var allApps by remember { mutableStateOf<List<AppEntry>>(emptyList()) }
    var installed by remember { mutableStateOf(InstalledModules()) }
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
            // Detect which native modules are installed
            val (_, detectOut) =
                suExec(
                    "echo kmod=\$([ -d $KMOD_MODULE_DIR ] && echo 1 || echo 0);" +
                        "echo zygisk=\$([ -d $ZYGISK_MODULE_DIR ] && echo 1 || echo 0)",
                )
            val flags =
                detectOut
                    .lines()
                    .filter { it.contains("=") }
                    .associate {
                        val (k, v) = it.split("=", limit = 2)
                        k to (v == "1")
                    }
            installed =
                InstalledModules(
                    kmod = flags["kmod"] == true,
                    zygisk = flags["zygisk"] == true,
                )

            // Read 3 separate target lists
            fun readTargets(path: String): Set<String> {
                val (_, raw) = suExec("cat $path 2>/dev/null || true")
                return raw
                    .lines()
                    .map { it.trim() }
                    .filter { it.isNotEmpty() && !it.startsWith("#") }
                    .toSet()
            }
            val kmodTargets = readTargets(KMOD_TARGETS)
            val zygiskTargets = readTargets(ZYGISK_TARGETS)
            val lsposedTargets = readTargets(LSPOSED_TARGETS)

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
                        AppEntry(
                            packageName = pkg,
                            label = label,
                            icon = icon,
                            isSystem = isSystem,
                            kmod = pkg in kmodTargets,
                            zygisk = pkg in zygiskTargets,
                            lsposed = pkg in lsposedTargets,
                        )
                    }.sortedBy { it.label.lowercase() }

            allApps = entries
            loading = false
        }
    }

    val filteredApps =
        remember(allApps, searchQuery, showSystem, showRussianOnly) {
            val q = searchQuery.trim().lowercase()
            allApps.filter { app ->
                (showSystem || !app.isSystem || app.anySelected) &&
                    (!showRussianOnly || isRussianApp(app.packageName, app.label)) &&
                    (q.isEmpty() || app.label.lowercase().contains(q) || app.packageName.lowercase().contains(q))
            }
        }

    val selectedCount = remember(allApps) { allApps.count { it.anySelected } }

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
                                prefKey = "apps_tun",
                                title = stringResource(R.string.apps_help_title),
                            ) {
                                Text(
                                    text = stringResource(R.string.apps_hint_toggles),
                                    style = MaterialTheme.typography.bodyMedium,
                                )
                                Text(
                                    text = stringResource(R.string.apps_hint_zygisk),
                                    style = MaterialTheme.typography.bodyMedium,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                        }
                    }
                    items(filteredApps, key = { it.packageName }) { app ->
                        AppRow(
                            app = app,
                            installed = installed,
                            onToggle = { layer ->
                                allApps =
                                    allApps.map {
                                        if (it.packageName != app.packageName) {
                                            it
                                        } else {
                                            when (layer) {
                                                Layer.KMOD -> it.copy(kmod = !it.kmod)
                                                Layer.ZYGISK -> it.copy(zygisk = !it.zygisk)
                                                Layer.LSPOSED -> it.copy(lsposed = !it.lsposed)
                                            }
                                        }
                                    }
                                dirty = true
                            },
                            onToggleAll = {
                                allApps =
                                    allApps.map {
                                        if (it.packageName != app.packageName) {
                                            it
                                        } else {
                                            val newState = !it.anySelected
                                            it.copy(
                                                kmod = if (installed.kmod) newState else false,
                                                zygisk = if (installed.zygisk) newState else false,
                                                lsposed = newState,
                                            )
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
                        text = stringResource(R.string.selected_count, selectedCount),
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

    // Save effect
    if (saving) {
        LaunchedEffect(Unit) {
            // Always include self in all lists (self is hidden from UI but must stay in targets)
            val selfPkg = context.packageName
            val kmodPkgs = (allApps.filter { it.kmod }.map { it.packageName } + selfPkg).distinct().sorted()
            val zygiskPkgs = (allApps.filter { it.zygisk }.map { it.packageName } + selfPkg).distinct().sorted()
            val lsposedPkgs = (allApps.filter { it.lsposed }.map { it.packageName } + selfPkg).distinct().sorted()
            val header = context.getString(R.string.save_header_comment)

            try {
                val (exitCode, _) =
                    suExecAsync(
                        buildSaveCommand(header, kmodPkgs, zygiskPkgs, lsposedPkgs),
                    )
                val totalSelected = allApps.count { it.anySelected }
                if (exitCode == 0) {
                    snackMessage = context.getString(R.string.save_success, totalSelected)
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

private fun buildSaveCommand(
    header: String,
    kmodPkgs: List<String>,
    zygiskPkgs: List<String>,
    lsposedPkgs: List<String>,
): String {
    fun encodeBody(pkgs: List<String>): String {
        val body = "$header\n" + pkgs.joinToString("\n") + if (pkgs.isNotEmpty()) "\n" else ""
        return android.util.Base64.encodeToString(body.toByteArray(), android.util.Base64.NO_WRAP)
    }

    val parts = mutableListOf<String>()

    // Write kmod targets
    val kmodB64 = encodeBody(kmodPkgs)
    parts += "if [ -d /data/adb/vpnhide_kmod ]; then echo '$kmodB64' | base64 -d > $KMOD_TARGETS && chmod 644 $KMOD_TARGETS; fi"

    // Write zygisk targets
    val zygiskB64 = encodeBody(zygiskPkgs)
    parts += "if [ -d /data/adb/vpnhide_zygisk ]; then echo '$zygiskB64' | base64 -d > $ZYGISK_TARGETS && chmod 644 $ZYGISK_TARGETS; fi"
    parts += "cp $ZYGISK_TARGETS $ZYGISK_MODULE_TARGETS 2>/dev/null; true"

    // Write lsposed targets (always — the dir is created by service.sh or us)
    val lsposedB64 = encodeBody(lsposedPkgs)
    parts += "mkdir -p /data/adb/vpnhide_lsposed"
    parts += "echo '$lsposedB64' | base64 -d > $LSPOSED_TARGETS && chmod 644 $LSPOSED_TARGETS"

    // Resolve kmod UIDs -> /proc/vpnhide_targets
    if (kmodPkgs.isNotEmpty()) {
        parts += buildUidResolver(kmodPkgs, PROC_TARGETS)
    } else {
        parts += "echo > $PROC_TARGETS 2>/dev/null; true"
    }

    // Resolve lsposed UIDs -> /data/system/vpnhide_uids.txt
    if (lsposedPkgs.isNotEmpty()) {
        parts += buildUidResolver(lsposedPkgs, SS_UIDS_FILE)
        parts += "chmod 644 $SS_UIDS_FILE 2>/dev/null"
        parts += "chcon u:object_r:system_data_file:s0 $SS_UIDS_FILE 2>/dev/null"
    } else {
        parts += "echo > $SS_UIDS_FILE 2>/dev/null; true"
    }

    return parts.joinToString(" ; ")
}

private fun buildUidResolver(
    packages: List<String>,
    outputFile: String,
): String =
    buildString {
        append("ALL_PKGS=\"\$(pm list packages -U 2>/dev/null)\"")
        append("; UIDS=\"\"")
        for (pkg in packages) {
            append("; U=\$(echo \"\$ALL_PKGS\" | grep '^package:$pkg ' | sed 's/.*uid://')")
            append("; if [ -n \"\$U\" ]; then if [ -z \"\$UIDS\" ]; then UIDS=\"\$U\"; else UIDS=\"\$UIDS")
            append("\n")
            append("\$U\"; fi; fi")
        }
        append("; if [ -n \"\$UIDS\" ]; then echo \"\$UIDS\" > $outputFile 2>/dev/null")
        append("; else echo > $outputFile 2>/dev/null; fi")
    }

@Composable
private fun AppRow(
    app: AppEntry,
    installed: InstalledModules,
    onToggle: (Layer) -> Unit,
    onToggleAll: () -> Unit,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onToggleAll)
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
                LayerChip("L", app.lsposed, true) { onToggle(Layer.LSPOSED) }
                if (installed.kmod) {
                    LayerChip("K", app.kmod, true) { onToggle(Layer.KMOD) }
                }
                if (installed.zygisk) {
                    LayerChip("Z", app.zygisk, true) { onToggle(Layer.ZYGISK) }
                }
            }
        }
    }
}

@Composable
private fun LayerChip(
    label: String,
    enabled: Boolean,
    available: Boolean,
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
