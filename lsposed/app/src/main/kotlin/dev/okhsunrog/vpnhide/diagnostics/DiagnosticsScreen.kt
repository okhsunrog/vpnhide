package dev.okhsunrog.vpnhide.diagnostics

import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.Uri
import android.os.Build
import android.util.Log
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.FiberManualRecord
import androidx.compose.material.icons.filled.FileDownload
import androidx.compose.material.icons.filled.Stop
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.okhsunrog.vpnhide.DashboardCache
import dev.okhsunrog.vpnhide.FileSaveShareRow
import dev.okhsunrog.vpnhide.NativeCoreStatusCard
import dev.okhsunrog.vpnhide.R
import dev.okhsunrog.vpnhide.StatusBanner
import dev.okhsunrog.vpnhide.StatusColors
import dev.okhsunrog.vpnhide.core.LogcatAutoStop
import dev.okhsunrog.vpnhide.debug.DebugExportOutcome
import dev.okhsunrog.vpnhide.debug.LogcatRecorder
import dev.okhsunrog.vpnhide.debug.exportDebug
import dev.okhsunrog.vpnhide.generated.IfaceLists
import dev.okhsunrog.vpnhide.hook.HookLog
import dev.okhsunrog.vpnhide.nativeStatusSnapshot
import dev.okhsunrog.vpnhide.next
import dev.okhsunrog.vpnhide.shareFileViaProvider
import dev.okhsunrog.vpnhide.startup.StartupCoordinator
import dev.okhsunrog.vpnhide.ui.components.ButtonSpinner
import dev.okhsunrog.vpnhide.ui.components.EnhancedButton
import dev.okhsunrog.vpnhide.ui.components.EnhancedCard
import dev.okhsunrog.vpnhide.ui.components.GroupedCard
import dev.okhsunrog.vpnhide.ui.components.SectionHeader
import dev.okhsunrog.vpnhide.ui.components.container
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.BufferedReader
import java.io.File
import java.io.InputStreamReader
import java.net.NetworkInterface
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream

@Composable
fun DiagnosticsScreen(
    selfNeedsRestart: Boolean,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val snapshot by DiagnosticsCache.snapshot.collectAsStateWithLifecycle()
    LaunchedEffect(selfNeedsRestart) { DashboardCache.ensureLoaded(context, selfNeedsRestart) }
    Column(modifier.fillMaxSize().padding(horizontal = 16.dp).verticalScroll(rememberScrollState())) {
        Spacer(Modifier.height(8.dp))
        AutonomousDiagnosticStatusCard(snapshot) { retryDiagnosticsAndDashboard(context, selfNeedsRestart) }
        Spacer(Modifier.height(12.dp))
        NativeCoreStatusCard(snapshot.sources.core.nativeStatusSnapshot())
        Spacer(Modifier.height(12.dp))
        AutonomousDiagnosticRows(snapshot)
        Spacer(Modifier.height(16.dp))
    }
}

/** Capture the source before opening Android's picker; its callback can arrive after recreation. */
@Composable
private fun rememberZipSaveLauncher(errorLabel: String): (File) -> Unit {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var pendingPath by rememberSaveable { mutableStateOf<String?>(null) }
    val launcher =
        rememberLauncherForActivityResult(
            ActivityResultContracts.CreateDocument("application/zip"),
        ) { uri: Uri? ->
            val path = pendingPath
            pendingPath = null
            if (uri != null && path != null) {
                scope.launch(Dispatchers.IO) {
                    runCatching {
                        val out = checkNotNull(context.contentResolver.openOutputStream(uri))
                        out.use { File(path).inputStream().use { input -> input.copyTo(it) } }
                    }.onFailure { HookLog.e("VpnHide: $errorLabel save failed: ${it.message}") }
                }
            }
        }
    return { file ->
        pendingPath = file.absolutePath
        launcher.launch(file.name)
    }
}

@Composable
fun DebugToolsSection(
    selfNeedsRestart: Boolean?,
    modifier: Modifier = Modifier,
) {
    var showModal by rememberSaveable { mutableStateOf(false) }

    Column(modifier = modifier.fillMaxWidth()) {
        EnhancedCard(
            modifier = Modifier.fillMaxWidth(),
            color = MaterialTheme.colorScheme.surfaceVariant,
            shape = MaterialTheme.shapes.medium,
        ) {
            Column(modifier = Modifier.padding(16.dp)) {
                Text(
                    text = stringResource(R.string.debug_export_card_title),
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.Bold,
                )
                Spacer(Modifier.height(4.dp))
                Text(
                    text = stringResource(R.string.debug_export_card_description),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Spacer(Modifier.height(12.dp))
                // The card button only ever opens the modal — the whole export flow
                // (progress, then Save/Share, then re-export with changed options)
                // lives inside the sheet, so nothing shifts under the user's finger.
                EnhancedButton(
                    onClick = { showModal = true },
                    enabled = selfNeedsRestart != null,
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Icon(Icons.Default.FileDownload, contentDescription = null, modifier = Modifier.size(18.dp))
                    Spacer(Modifier.width(8.dp))
                    Text(stringResource(R.string.btn_export_debug))
                }
            }
        }

        Spacer(Modifier.height(16.dp))

        // Core owns the recorder; recording does not depend on a foreign VPN.
        LogcatRecordCard(selfNeedsRestart = selfNeedsRestart)
    }

    if (showModal) {
        DebugExportSheet(selfNeedsRestart = selfNeedsRestart, onDismiss = { showModal = false })
    }
}

/**
 * The debug-export bottom sheet's own composable, so every piece of its state — the
 * collected file, the option toggles, and the capture gate — lives in the sheet's own
 * composition scope instead of the section's. Called only from `if (showModal)`, it is
 * torn down on dismiss and rebuilt fresh on the next open: a reopened sheet never shows
 * a stale collected file, a stale Save/Share row, or stale toggle choices left over from
 * the previous run.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun DebugExportSheet(
    selfNeedsRestart: Boolean?,
    onDismiss: () -> Unit,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var exporting by remember { mutableStateOf(false) }
    var outcome by remember { mutableStateOf<DebugExportOutcome?>(null) }
    val resultFile = (outcome as? DebugExportOutcome.Written)?.file

    val snapshot by DiagnosticsCache.snapshot.collectAsStateWithLifecycle()
    val recommendation = exportRecommendation(snapshot)
    var overrides by remember { mutableStateOf(ExportOverrides()) }
    var advanced by remember { mutableStateOf(false) }
    val (optKernelImage, optJavaFiles) = overrides.resolve(recommendation)

    // Every export kind is now a .zip (carrying state.json), so one MIME type fits all.
    val saveLauncher = rememberZipSaveLauncher("debug-export")

    // Changing any toggle invalidates a result produced with the old recipe.
    val clearResult = { outcome = null }
    ModalBottomSheet(onDismissRequest = { if (!exporting) onDismiss() }) {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .verticalScroll(rememberScrollState())
                    .padding(horizontal = 24.dp)
                    .padding(bottom = 24.dp),
        ) {
            Text(
                text = stringResource(R.string.debug_export_modal_title),
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Bold,
            )
            Spacer(Modifier.height(12.dp))
            Text(
                stringResource(R.string.debug_export_kernel_reason, stringResource(exportReasonResource(recommendation.kernelReason))),
                style = MaterialTheme.typography.bodySmall,
            )
            Text(
                stringResource(R.string.debug_export_java_reason, stringResource(exportReasonResource(recommendation.javaReason))),
                style = MaterialTheme.typography.bodySmall,
            )
            TextButton(onClick = { advanced = !advanced }, enabled = !exporting) {
                Text(stringResource(if (advanced) R.string.debug_export_advanced_hide else R.string.debug_export_advanced))
            }
            if (advanced) {
                if (overrides !=
                    ExportOverrides()
                ) {
                    Text(stringResource(R.string.debug_export_manual), style = MaterialTheme.typography.bodySmall)
                }
                ExportToggle(
                    title = stringResource(R.string.debug_export_opt_kernel),
                    description = stringResource(R.string.debug_export_opt_kernel_desc),
                    checked = optKernelImage,
                    enabled = !exporting,
                    onCheckedChange = {
                        overrides = overrides.copy(kernel = it)
                        clearResult()
                    },
                )
                ExportToggle(
                    title = stringResource(R.string.debug_export_java),
                    description = stringResource(R.string.debug_export_java_desc),
                    checked = optJavaFiles,
                    enabled = !exporting,
                    onCheckedChange = {
                        overrides = overrides.copy(java = it)
                        clearResult()
                    },
                )
                TextButton(onClick = {
                    overrides = ExportOverrides()
                    clearResult()
                }, enabled = !exporting) {
                    Text(stringResource(R.string.debug_export_reset))
                }
            }
            Spacer(Modifier.height(16.dp))

            val doExport = {
                selfNeedsRestart?.let { restartState ->
                    val kernel = optKernelImage
                    val java = optJavaFiles
                    val plan =
                        "snapshot=${snapshot.revision}; recommended=$recommendation; " +
                            "overrides=$overrides; selected kernel=$kernel java=$java"
                    exporting = true
                    scope.launch {
                        outcome = null
                        try {
                            outcome = exportDebug(context, restartState, kernel, java, plan)
                        } finally {
                            exporting = false
                        }
                    }
                }
                Unit
            }

            val file = resultFile
            if (!exporting) outcome?.let { ExportOutcomeBanner(it) }
            (outcome as? DebugExportOutcome.Written)?.attachments?.let {
                Text(
                    stringResource(R.string.debug_export_actual, it.kernelImages, it.frameworkFiles, it.javaSkipped),
                    style = MaterialTheme.typography.bodySmall,
                )
            }

            when {
                // In-progress: a disabled progress button, no competing actions.
                exporting -> {
                    EnhancedButton(onClick = {}, enabled = false, modifier = Modifier.fillMaxWidth()) {
                        ButtonSpinner()
                        Spacer(Modifier.width(8.dp))
                        Text(stringResource(R.string.btn_export_debug_running))
                    }
                }

                // Configure phase: the one primary action. Held disabled while a
                // fresh gate re-check is in flight so the user can't collect on a
                // stale "routed" verdict before the VPN-off warning resolves.
                file == null -> {
                    EnhancedButton(
                        onClick = doExport,
                        enabled = selfNeedsRestart != null,
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Text(stringResource(R.string.debug_export_modal_confirm))
                    }
                }

                // Result phase: Save/Share are primary; re-running is a de-emphasized
                // secondary action, so there is no ambiguous "Export" to double-tap.
                else -> {
                    FileSaveShareRow(
                        saveLabel = stringResource(R.string.btn_save_debug),
                        shareLabel = stringResource(R.string.btn_share_debug),
                        sharePrimary = true,
                        onSave = { saveLauncher(file) },
                        onShare = { shareFileViaProvider(context, file, "application/zip") },
                    )
                    TextButton(
                        onClick = doExport,
                        modifier = Modifier.align(Alignment.CenterHorizontally),
                    ) {
                        Text(stringResource(R.string.debug_export_collect_again))
                    }
                }
            }
        }
    }
}

@Composable
private fun ExportToggle(
    title: String,
    description: String,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    enabled: Boolean = true,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(text = title, style = MaterialTheme.typography.bodyLarge)
            Text(
                text = description,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Spacer(Modifier.width(12.dp))
        Switch(checked = checked, onCheckedChange = onCheckedChange, enabled = enabled)
    }
}

@Composable
private fun LogcatRecordCard(selfNeedsRestart: Boolean?) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val state by LogcatRecorder.state.collectAsStateWithLifecycle()
    // A session core is still recording from before an app restart shows up as running.
    LaunchedEffect(Unit) { LogcatRecorder.refresh(context) }

    // Tick every second while recording so the elapsed counter updates
    // even when sizeBytes happens to hold steady.
    var nowMs by remember { mutableStateOf(System.currentTimeMillis()) }
    LaunchedEffect(state) {
        if (state is LogcatRecorder.State.Recording) {
            while (true) {
                nowMs = System.currentTimeMillis()
                kotlinx.coroutines.delay(1000)
            }
        }
    }

    val saveLauncher =
        rememberZipSaveLauncher("logcat")

    EnhancedCard(
        modifier = Modifier.fillMaxWidth(),
        color = MaterialTheme.colorScheme.surfaceVariant,
        shape = MaterialTheme.shapes.medium,
    ) {
        Column(modifier = Modifier.padding(16.dp)) {
            Text(
                text = stringResource(R.string.logcat_card_title),
                style = MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.Bold,
            )
            Spacer(Modifier.height(4.dp))
            Text(
                text = stringResource(R.string.logcat_card_description),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(12.dp))

            when (val s = state) {
                is LogcatRecorder.State.Recording -> {
                    val elapsed = (nowMs - s.startMs).coerceAtLeast(0L) / 1000
                    Text(
                        text =
                            stringResource(
                                R.string.logcat_recording_status,
                                formatElapsed(elapsed),
                                formatSize(s.sizeBytes),
                            ),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.error,
                        fontWeight = FontWeight.Bold,
                    )
                    if (s.maxBytes > 0) {
                        Text(
                            text = stringResource(R.string.logcat_recording_limits, formatSize(s.maxBytes), s.maxDurationMs / 60_000),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    Text(
                        text = stringResource(R.string.logcat_debug_lease_note),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Spacer(Modifier.height(12.dp))
                    EnhancedButton(
                        onClick = {
                            scope.launch { LogcatRecorder.stop(context) }
                        },
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Icon(
                            Icons.Default.Stop,
                            contentDescription = null,
                            modifier = Modifier.size(18.dp),
                        )
                        Spacer(Modifier.width(8.dp))
                        Text(stringResource(R.string.logcat_btn_stop))
                    }
                }

                is LogcatRecorder.State.Undelivered -> {
                    logcatEndNotice(s.failure, s.autoStop, s.maxBytes, s.maxDurationMs)?.let { notice ->
                        Text(
                            text = notice,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.error,
                        )
                        Spacer(Modifier.height(8.dp))
                    }
                    Text(
                        text = stringResource(R.string.logcat_undelivered_title),
                        style = MaterialTheme.typography.bodyMedium,
                        fontWeight = FontWeight.Bold,
                    )
                    Text(
                        text =
                            stringResource(
                                R.string.logcat_undelivered_body,
                                formatStartTime(s.startMs),
                                formatElapsed(s.durationMs / 1000),
                                formatSize(s.sizeBytes),
                            ),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Spacer(Modifier.height(12.dp))
                    EnhancedButton(
                        onClick = {
                            val restartState = selfNeedsRestart ?: return@EnhancedButton
                            scope.launch { LogcatRecorder.saveUndelivered(context, restartState) }
                        },
                        enabled = selfNeedsRestart != null,
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Text(stringResource(R.string.logcat_btn_save_recording))
                    }
                    TextButton(
                        onClick = { scope.launch { LogcatRecorder.discardUndelivered() } },
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Text(stringResource(R.string.logcat_btn_discard_recording))
                    }
                }

                is LogcatRecorder.State.Stopped -> {
                    logcatEndNotice(s.failure, s.autoStop, s.maxBytes, s.maxDurationMs)?.let { notice ->
                        Text(
                            text = notice,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.error,
                        )
                        Spacer(Modifier.height(8.dp))
                    }
                    val last = s.lastFile
                    if (last != null && last.exists()) {
                        Text(
                            text =
                                stringResource(
                                    R.string.logcat_last_recording,
                                    formatElapsed(s.lastDurationMs / 1000),
                                    formatSize(last.length()),
                                ),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        Spacer(Modifier.height(8.dp))
                        FileSaveShareRow(
                            saveLabel = stringResource(R.string.btn_save),
                            shareLabel = stringResource(R.string.btn_share_debug),
                            sharePrimary = false,
                            onSave = { saveLauncher(last) },
                            onShare = { shareFileViaProvider(context, last, "application/zip") },
                        )
                        Spacer(Modifier.height(8.dp))
                    }

                    EnhancedButton(
                        onClick = {
                            val restartState = selfNeedsRestart ?: return@EnhancedButton
                            scope.launch { LogcatRecorder.start(context, restartState) }
                        },
                        enabled = selfNeedsRestart != null,
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Icon(
                            Icons.Default.FiberManualRecord,
                            contentDescription = null,
                            modifier = Modifier.size(18.dp),
                        )
                        Spacer(Modifier.width(8.dp))
                        Text(stringResource(R.string.logcat_btn_start))
                    }
                }
            }
        }
    }
}

@Composable
private fun logcatEndNotice(
    failure: LogcatRecorder.Failure?,
    autoStop: LogcatAutoStop?,
    maxBytes: Long,
    maxDurationMs: Long,
): String? {
    failure?.let {
        val res =
            when (it.stage) {
                LogcatRecorder.Failure.Stage.Start -> R.string.logcat_start_failed
                LogcatRecorder.Failure.Stage.Stop -> R.string.logcat_stop_failed
                LogcatRecorder.Failure.Stage.Discard -> R.string.logcat_discard_failed
            }
        return stringResource(res, it.status)
    }
    return when (autoStop) {
        LogcatAutoStop.SizeCap -> stringResource(R.string.logcat_auto_stopped_size, formatSize(maxBytes))
        LogcatAutoStop.DurationCap -> stringResource(R.string.logcat_auto_stopped_duration, maxDurationMs / 60_000)
        LogcatAutoStop.LogcatExited -> stringResource(R.string.logcat_auto_stopped_exited)
        LogcatAutoStop.CoreRestarted -> stringResource(R.string.logcat_auto_stopped_core_restarted)
        null -> null
    }
}

private fun formatStartTime(ms: Long): String =
    java.text.DateFormat
        .getDateTimeInstance(java.text.DateFormat.SHORT, java.text.DateFormat.SHORT)
        .format(java.util.Date(ms))

private fun formatElapsed(seconds: Long): String {
    val m = seconds / 60
    val s = seconds % 60
    return "%d:%02d".format(m, s)
}

private fun formatSize(bytes: Long): String {
    if (bytes < 1024) return "$bytes B"
    val kb = bytes / 1024.0
    if (kb < 1024.0) return "%.1f KB".format(kb)
    val mb = kb / 1024.0
    return "%.1f MB".format(mb)
}

@Composable
private fun ExportOutcomeBanner(outcome: DebugExportOutcome) {
    val text =
        when (outcome) {
            is DebugExportOutcome.Failed -> {
                stringResource(R.string.debug_export_failed, outcome.reason)
            }

            is DebugExportOutcome.Written -> {
                if (outcome.errors.isEmpty()) {
                    stringResource(R.string.debug_export_complete)
                } else {
                    stringResource(R.string.debug_export_partial, outcome.errors.joinToString("\n"))
                }
            }
        }
    val color =
        when (outcome) {
            is DebugExportOutcome.Failed -> {
                StatusColors.errorContainer()
            }

            is DebugExportOutcome.Written -> {
                if (outcome.errors.isEmpty()) {
                    StatusColors.successContainer()
                } else {
                    StatusColors
                        .warningContainer()
                }
            }
        }
    StatusBanner(text = text, containerColor = color, contentColor = MaterialTheme.colorScheme.onSurface)
    Spacer(Modifier.height(12.dp))
}
