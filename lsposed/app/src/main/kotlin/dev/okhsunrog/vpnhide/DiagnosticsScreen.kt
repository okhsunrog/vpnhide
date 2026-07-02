package dev.okhsunrog.vpnhide

import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.Uri
import android.os.Build
import android.util.Log
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.FiberManualRecord
import androidx.compose.material.icons.filled.FileDownload
import androidx.compose.material.icons.filled.Stop
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.okhsunrog.vpnhide.checks.CheckOutput
import dev.okhsunrog.vpnhide.generated.IfaceLists
import dev.okhsunrog.vpnhide.ui.components.EnhancedButton
import dev.okhsunrog.vpnhide.ui.components.EnhancedCard
import dev.okhsunrog.vpnhide.ui.components.GroupedCard
import dev.okhsunrog.vpnhide.ui.components.SectionHeader
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
    val scope = rememberCoroutineScope()

    val diagState by DiagnosticsCache.state.collectAsState()
    val summaryFmt = stringResource(R.string.summary_format)

    // Kick off the diagnostics run once per process. If selfNeedsRestart
    // is true we skip — hooks aren't applied to this app yet, results
    // would be meaningless. DiagnosticsCache.run is idempotent: no-op
    // when Ready/Running.
    LaunchedEffect(selfNeedsRestart) {
        if (!selfNeedsRestart) {
            DiagnosticsCache.run(scope, context)
        }
    }

    val results = (diagState as? DiagnosticsCache.State.Ready)?.results
    // Native probes that couldn't run (ECONNREFUSED from socket()) are
    // represented as passed=null by nativeCheck. Java-level checks never
    // produce that state, so this test isolates the "app has no network
    // permission" banner from everything else.
    val networkBlocked = results?.native?.any { it.passed == null } == true
    val summary =
        results?.let { r ->
            val score = r.all.score()
            String.format(summaryFmt, score.passed, score.total)
        }

    Column(
        modifier =
            modifier
                .fillMaxSize()
                .padding(horizontal = 16.dp)
                .verticalScroll(rememberScrollState()),
    ) {
        Spacer(Modifier.height(8.dp))

        when {
            selfNeedsRestart -> {
                StatusBanner(
                    text = stringResource(R.string.banner_added_self),
                    containerColor = MaterialTheme.colorScheme.tertiaryContainer,
                    contentColor = MaterialTheme.colorScheme.onTertiaryContainer,
                )
            }

            diagState is DiagnosticsCache.State.VpnOff -> {
                VpnOffPrompt(
                    onRetry = {
                        DiagnosticsCache.retry(scope, context)
                        DashboardCache.refresh(scope, context, selfNeedsRestart)
                    },
                )
            }

            diagState is DiagnosticsCache.State.Failed -> {
                DiagnosticsFailedPrompt(
                    onRetry = {
                        DiagnosticsCache.retry(scope, context)
                        DashboardCache.refresh(scope, context, selfNeedsRestart)
                    },
                )
            }

            diagState is DiagnosticsCache.State.Running ||
                diagState is DiagnosticsCache.State.NotRun -> {
                Box(
                    modifier = Modifier.fillMaxWidth().padding(vertical = 32.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    CircularProgressIndicator()
                }
            }

            diagState is DiagnosticsCache.State.Ready -> {
                StatusBanner(
                    text = stringResource(R.string.banner_ready),
                    containerColor = StatusColors.successContainer(),
                    contentColor = MaterialTheme.colorScheme.onSurface,
                )

                if (networkBlocked) {
                    Spacer(Modifier.height(6.dp))
                    StatusBanner(
                        text = stringResource(R.string.banner_network_blocked),
                        containerColor = StatusColors.errorContainer(),
                        contentColor = MaterialTheme.colorScheme.onSurface,
                    )
                }

                if (summary != null) {
                    Spacer(Modifier.height(12.dp))
                    Text(
                        text = summary,
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.Bold,
                    )
                }

                results?.let { r ->
                    Spacer(Modifier.height(16.dp))

                    SectionHeader(stringResource(R.string.section_native))
                    Spacer(Modifier.height(6.dp))
                    Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
                        r.nativeAll.forEachIndexed { i, check ->
                            CheckCard(check, index = i, count = r.nativeAll.size)
                        }
                    }

                    Spacer(Modifier.height(16.dp))

                    SectionHeader(stringResource(R.string.section_java))
                    Spacer(Modifier.height(6.dp))
                    Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
                        r.java.forEachIndexed { i, check ->
                            CheckCard(check, index = i, count = r.java.size)
                        }
                    }
                }
            }
        }

        Spacer(Modifier.height(16.dp))
    }
}

@Composable
fun DebugToolsSection(
    selfNeedsRestart: Boolean?,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val cm = context.getSystemService(ConnectivityManager::class.java)
    val scope = rememberCoroutineScope()
    var exporting by remember { mutableStateOf(false) }
    var debugZipFile by remember { mutableStateOf<File?>(null) }

    val saveLauncher =
        rememberLauncherForActivityResult(
            ActivityResultContracts.CreateDocument("application/zip"),
        ) { uri: Uri? ->
            val zip = debugZipFile ?: return@rememberLauncherForActivityResult
            if (uri != null) {
                // Copy off the main thread and swallow IO errors — a large zip
                // would otherwise block the UI and a write failure would crash.
                scope.launch(Dispatchers.IO) {
                    runCatching {
                        context.contentResolver.openOutputStream(uri)?.use { out ->
                            zip.inputStream().use { it.copyTo(out) }
                        }
                    }.onFailure { HookLog.e("VpnHide: debug-zip save failed: ${it.message}") }
                }
            }
        }

    Column(
        modifier = modifier.fillMaxWidth(),
    ) {
        LogcatRecordCard(selfNeedsRestart = selfNeedsRestart)

        Spacer(Modifier.height(16.dp))

        KernelImageExportCard()

        Spacer(Modifier.height(16.dp))

        // Collect button
        val zip = debugZipFile
        if (zip == null) {
            EnhancedButton(
                onClick = {
                    val restartState = selfNeedsRestart ?: return@EnhancedButton
                    exporting = true
                    scope.launch {
                        debugZipFile = exportDebugZip(cm, context, restartState)
                        exporting = false
                    }
                },
                enabled = !exporting && selfNeedsRestart != null,
                modifier = Modifier.fillMaxWidth(),
            ) {
                if (exporting) {
                    CircularProgressIndicator(
                        modifier = Modifier.size(18.dp),
                        strokeWidth = 2.dp,
                        color = MaterialTheme.colorScheme.onPrimary,
                    )
                    Spacer(Modifier.width(8.dp))
                }
                Text(
                    if (exporting) {
                        stringResource(R.string.btn_export_debug_running)
                    } else {
                        stringResource(R.string.btn_export_debug)
                    },
                )
            }
        } else {
            FileSaveShareRow(
                saveLabel = stringResource(R.string.btn_save_debug),
                shareLabel = stringResource(R.string.btn_share_debug),
                sharePrimary = true,
                onSave = { saveLauncher.launch(zip.name) },
                onShare = { shareFileViaProvider(context, zip, "application/zip") },
            )
        }
    }
}

@Composable
private fun KernelImageExportCard() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var exporting by remember { mutableStateOf(false) }
    var kernelImagesFile by remember { mutableStateOf<File?>(null) }

    val saveLauncher =
        rememberLauncherForActivityResult(
            ActivityResultContracts.CreateDocument("application/zip"),
        ) { uri: Uri? ->
            val zip = kernelImagesFile ?: return@rememberLauncherForActivityResult
            if (uri != null) {
                scope.launch(Dispatchers.IO) {
                    runCatching {
                        context.contentResolver.openOutputStream(uri)?.use { out ->
                            zip.inputStream().use { it.copyTo(out) }
                        }
                    }.onFailure { HookLog.e("VpnHide: kernel-image save failed: ${it.message}") }
                }
            }
        }

    EnhancedCard(
        modifier = Modifier.fillMaxWidth(),
        color = MaterialTheme.colorScheme.surfaceVariant,
        shape = MaterialTheme.shapes.medium,
    ) {
        Column(modifier = Modifier.padding(16.dp)) {
            Text(
                text = stringResource(R.string.kernel_images_card_title),
                style = MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.Bold,
            )
            Spacer(Modifier.height(4.dp))
            Text(
                text = stringResource(R.string.kernel_images_card_description),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(12.dp))

            val zip = kernelImagesFile
            if (zip == null || !zip.exists()) {
                EnhancedButton(
                    onClick = {
                        exporting = true
                        scope.launch {
                            kernelImagesFile = exportKernelImagesZip(context)
                            exporting = false
                        }
                    },
                    enabled = !exporting,
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    if (exporting) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(18.dp),
                            strokeWidth = 2.dp,
                            color = MaterialTheme.colorScheme.onPrimary,
                        )
                    } else {
                        Icon(
                            Icons.Default.FileDownload,
                            contentDescription = null,
                            modifier = Modifier.size(18.dp),
                        )
                    }
                    Spacer(Modifier.width(8.dp))
                    Text(
                        if (exporting) {
                            stringResource(R.string.kernel_images_btn_export_running)
                        } else {
                            stringResource(R.string.kernel_images_btn_export)
                        },
                    )
                }
            } else {
                FileSaveShareRow(
                    saveLabel = stringResource(R.string.btn_save_kernel_images),
                    shareLabel = stringResource(R.string.btn_share_debug),
                    sharePrimary = true,
                    onSave = { saveLauncher.launch(zip.name) },
                    onShare = { shareFileViaProvider(context, zip, "application/zip") },
                )
            }
        }
    }
}

@Composable
private fun LogcatRecordCard(selfNeedsRestart: Boolean?) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val state by LogcatRecorder.state.collectAsState()

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
        rememberLauncherForActivityResult(
            ActivityResultContracts.CreateDocument("application/zip"),
        ) { uri: Uri? ->
            val src = (state as? LogcatRecorder.State.Stopped)?.lastFile ?: return@rememberLauncherForActivityResult
            if (uri != null) {
                scope.launch(Dispatchers.IO) {
                    runCatching {
                        context.contentResolver.openOutputStream(uri)?.use { out ->
                            src.inputStream().use { it.copyTo(out) }
                        }
                    }.onFailure { HookLog.e("VpnHide: logcat save failed: ${it.message}") }
                }
            }
        }

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

                is LogcatRecorder.State.Stopped -> {
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
                            onSave = { saveLauncher.launch(last.name) },
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
private fun CheckCard(
    r: CheckResult,
    index: Int = -1,
    count: Int = 1,
) {
    val actualColor =
        when (r.passed) {
            true -> StatusColors.successContainer()
            false -> StatusColors.errorContainer()
            null -> MaterialTheme.colorScheme.surfaceVariant
        }

    val badgeText =
        stringResource(
            when (r.passed) {
                true -> R.string.badge_pass
                false -> R.string.badge_fail
                null -> R.string.badge_info
            },
        )

    val badgeColor =
        when (r.passed) {
            true -> StatusColors.successBadge
            false -> StatusColors.errorAccent
            null -> MaterialTheme.colorScheme.onSurfaceVariant
        }

    GroupedCard(
        index = index,
        count = count,
        modifier = Modifier.fillMaxWidth(),
        color = actualColor,
    ) {
        Column(modifier = Modifier.padding(12.dp)) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = r.name,
                    style = MaterialTheme.typography.bodyLarge,
                    fontWeight = FontWeight.Bold,
                    modifier = Modifier.weight(1f),
                )
                Text(
                    text = badgeText,
                    fontWeight = FontWeight.Bold,
                    fontSize = 13.sp,
                    color = badgeColor,
                )
            }
            Spacer(Modifier.height(4.dp))
            Text(
                text = r.detail,
                style = MaterialTheme.typography.bodySmall,
                fontFamily = FontFamily.Monospace,
                color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.8f),
            )
        }
    }
}
