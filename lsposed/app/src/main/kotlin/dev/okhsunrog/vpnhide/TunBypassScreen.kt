package dev.okhsunrog.vpnhide

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
import kotlinx.coroutines.launch

internal data class BypassEntry(
    val packageName: String,
    val label: String,
    val icon: android.graphics.drawable.Drawable?,
    val isSystem: Boolean,
    val userIds: List<Int> = emptyList(),
    val kmod: Boolean = false,
)

@Composable
fun TunBypassScreen(
    searchQuery: String,
    showSystem: Boolean,
    showRussianOnly: Boolean,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()

    val cachedApps by AppListCache.apps.collectAsState()
    val userNames by AppListCache.userNames.collectAsState()
    val targets by TargetsCache.snapshot.collectAsState()

    var allApps by remember { mutableStateOf<List<BypassEntry>>(emptyList()) }
    var saving by remember { mutableStateOf(false) }
    var dirty by remember { mutableStateOf(false) }
    var snackMessage by remember { mutableStateOf<String?>(null) }
    val snackbarHostState = remember { SnackbarHostState() }

    LaunchedEffect(snackMessage) {
        snackMessage?.let {
            snackbarHostState.showSnackbar(message = it, duration = SnackbarDuration.Long)
            snackMessage = null
        }
    }

    LaunchedEffect(Unit) {
        TargetsCache.ensureLoaded(scope, context)
    }

    LaunchedEffect(cachedApps, targets) {
        if (dirty) return@LaunchedEffect
        val apps = cachedApps ?: return@LaunchedEffect
        val t = targets ?: return@LaunchedEffect
        val selfPkg = context.packageName
        allApps = apps.filter { it.packageName != selfPkg }.map { app ->
            BypassEntry(
                packageName = app.packageName,
                label = app.label,
                icon = app.icon,
                isSystem = app.isSystem,
                userIds = app.userIds,
                kmod = app.packageName in t.kmodDirectTargets,
            )
        }
        dirty = false
    }

    val loading = cachedApps == null || targets == null
    val filteredApps = remember(allApps, searchQuery, showSystem, showRussianOnly) {
        val q = searchQuery.trim().lowercase()
        allApps.filter { app ->
            (showSystem || !app.isSystem || app.kmod) &&
            (!showRussianOnly || isRussianApp(app.packageName, app.label)) &&
            (q.isEmpty() || app.label.lowercase().contains(q) || app.packageName.lowercase().contains(q))
        }
    }

    val selectedCount = remember(allApps) { allApps.count { it.kmod } }

    Column(modifier = modifier.fillMaxSize()) {
        if (loading) {
            Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                CircularProgressIndicator()
            }
        } else {
            val listState = rememberLazyListState()
            Box(modifier = Modifier.weight(1f)) {
                LazyColumn(state = listState, modifier = Modifier.fillMaxSize()) {
                    item {
                        Box(modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp)) {
                            HelpAccordion(
                                prefKey = "apps_bypass",
                                title = stringResource(R.string.bypass_help_title),
                            ) {
                                Text(
                                    text = stringResource(R.string.bypass_hint_logic),
                                    style = MaterialTheme.typography.bodyMedium,
                                )
                                Text(
                                    text = stringResource(R.string.bypass_hint_kmod),
                                    style = MaterialTheme.typography.bodyMedium,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                        }
                    }
                    items(filteredApps, key = { it.packageName }) { app ->
                        BypassAppRow(
                            app = app,
                            userNames = userNames,
                            onToggle = {
                                allApps = allApps.map {
                                    if (it.packageName != app.packageName) it else it.copy(kmod = !it.kmod)
                                }
                                dirty = true
                            }
                        )
                    }
                }
                
                val interactionSource = remember { MutableInteractionSource() }
                val isDragging by interactionSource.collectIsDraggedAsState()
                val indicatorAlpha by animateFloatAsState(if (isDragging) 1f else 0f, label = "alpha")
                VerticalScrollbar(
                    adapter = rememberScrollbarAdapter(scrollState = listState),
                    interactionSource = interactionSource,
                    style = defaultMaterialScrollbarStyle(),
                    modifier = Modifier.align(Alignment.TopEnd).fillMaxHeight(),
                    indicator = { position, isVisible ->
                        val firstChar = filteredApps.getOrNull(listState.firstVisibleItemIndex)?.label?.firstOrNull()?.uppercase() ?: ""
                        Box(
                            modifier = Modifier.align(Alignment.TopEnd).padding(end = 8.dp).graphicsLayer {
                                translationY = position; alpha = indicatorAlpha
                            }
                        ) {
                            Surface(
                                shape = RoundedCornerShape(4.dp),
                                color = if (isVisible) MaterialTheme.colorScheme.primary else Color.Transparent,
                                modifier = Modifier.size(48.dp)
                            ) {
                                Box(contentAlignment = Alignment.Center) {
                                    Text(firstChar, color = MaterialTheme.colorScheme.onPrimary, style = MaterialTheme.typography.titleMedium)
                                }
                            }
                        }
                    }
                )
            }

            Surface(tonalElevation = 3.dp) {
                Row(
                    modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp),
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
    }

    if (saving) {
        LaunchedEffect(Unit) {
            val selfPkg = context.packageName
            val kmodPkgs = (allApps.filter { it.kmod }.map { it.packageName } + selfPkg).distinct().sorted()
            val header = context.getString(R.string.save_header_comment)

            try {
                val (exitCode, _) = suExecAsync(buildBypassSaveCommand(header, kmodPkgs))
                if (exitCode == 0) {
                    snackMessage = context.getString(R.string.save_success, selectedCount)
                    DashboardCache.invalidate()
                    TargetsCache.refresh(scope, context)
                } else {
                    snackMessage = context.getString(R.string.save_failed_exit, exitCode)
                    dirty = true
                }
            } catch (e: Exception) {
                snackMessage = e.message
                dirty = true
            }
            saving = false
        }
    }
}

private fun buildBypassSaveCommand(header: String, kmodPkgs: List<String>): String {
    val body = "$header\n" + kmodPkgs.joinToString("\n") + if (kmodPkgs.isNotEmpty()) "\n" else ""
    val b64 = android.util.Base64.encodeToString(body.toByteArray(), android.util.Base64.NO_WRAP)
    
    val parts = mutableListOf<String>()
    parts += "if [ -d /data/adb/vpnhide_kmod ]; then echo '$b64' | base64 -d > $KMOD_DIRECT_TARGETS && chmod 644 $KMOD_DIRECT_TARGETS; fi"
    
    if (kmodPkgs.isNotEmpty()) {
        parts += buildUidResolver(kmodPkgs, PROC_DIRECT_TARGETS)
    } else {
        parts += "echo > $PROC_DIRECT_TARGETS 2>/dev/null; true"
    }
    
    return parts.joinToString(" ; ")
}

@Composable
private fun BypassAppRow(
    app: BypassEntry,
    userNames: Map<Int, String>,
    onToggle: () -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth().clickable(onClick = onToggle).padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        app.icon?.let { drawable ->
            Image(bitmap = drawable.toBitmap(48, 48).asImageBitmap(), contentDescription = null, modifier = Modifier.size(40.dp))
            Spacer(Modifier.width(12.dp))
        }
        Column(modifier = Modifier.weight(1f)) {
            Text(text = labelWithUsers(app.label, app.userIds, userNames), style = MaterialTheme.typography.bodyLarge, fontWeight = FontWeight.Medium)
            Text(text = app.packageName, style = MaterialTheme.typography.bodySmall, fontFamily = FontFamily.Monospace, fontSize = 12.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Spacer(Modifier.height(4.dp))
            Surface(
                shape = RoundedCornerShape(4.dp),
                color = if (app.kmod) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceVariant,
                modifier = Modifier.clickable(onClick = onToggle)
            ) {
                Text(
                    text = "K",
                    style = MaterialTheme.typography.labelSmall,
                    fontWeight = FontWeight.Bold,
                    color = if (app.kmod) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
                )
            }
        }
    }
}
