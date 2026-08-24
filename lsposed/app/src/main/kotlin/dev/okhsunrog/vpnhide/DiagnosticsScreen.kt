package dev.okhsunrog.vpnhide

import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.Uri
import android.os.Build
import android.util.Log
import androidx.activity.compose.ManagedActivityResultLauncher
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.isSystemInDarkTheme
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
import dev.okhsunrog.vpnhide.generated.IfaceLists
import dev.okhsunrog.vpnhide.hook.HookLog
import dev.okhsunrog.vpnhide.ui.components.ButtonSpinner
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
    // The dashboard state carries which native backend is active + the optional hooks
    // it installed — the inputs needed to rebuild the canonical DiagnosticReport here,
    // so each check can be shown against the vectors the backend actually OWNS. Null
    // until the dashboard has loaded (then we fall back to the raw, ownership-less list).
    val dashState by DashboardCache.state.collectAsState()
    // The LIVE gate — kept fresh by VpnTransportWatcher on every VPN transport change —
    // decides which blocking banner to show. DiagnosticsCache's own gate (inside
    // State.Blocked) is process-sticky and only re-derived on an explicit retry, so it
    // must not drive the banner here (see the module doc on RoutingGateCache).
    val liveGate by RoutingGateCache.gate.collectAsState()
    val tallyFmt = stringResource(R.string.diag_summary_tally)

    // Kick off the diagnostics run once per process. The cache parks at
    // Blocked(NEEDS_RESTART) itself when selfNeedsRestart (hooks aren't applied to this
    // app yet, so a run would be meaningless); run is idempotent otherwise.
    LaunchedEffect(selfNeedsRestart) {
        DiagnosticsCache.run(scope, context, selfNeedsRestart)
        // Ensure the backend/ownership state is available even when the user opens
        // Diagnostics without visiting the Dashboard first (cheap no-op if cached).
        DashboardCache.ensureLoaded(scope, context, selfNeedsRestart)
        // Same belt-and-suspenders init as DashboardCache above: usually already
        // seeded by StartupCoordinator.ensureInitialCaches, but a cheap no-op here
        // guarantees the shared gate is ready even if Diagnostics is opened first.
        RoutingGateCache.ensureLoaded(scope, context, selfNeedsRestart)
    }
    // The live gate is the trigger to (re)compute the frozen check results: once the
    // VPN comes up (or this app becomes routed), the results must be measured even if
    // DiagnosticsCache is still sitting on a stale Blocked/Failed from before. run() is
    // idempotent — a no-op on an already-complete Ready — so this is cheap on every
    // recomposition where liveGate hasn't changed.
    LaunchedEffect(liveGate) {
        if (liveGate == DiagnosticGate.ROUTED) {
            DiagnosticsCache.run(scope, context, selfNeedsRestart)
        }
    }

    val results = (diagState as? DiagnosticsCache.State.Ready)?.results
    // Native probes that couldn't run (ECONNREFUSED from socket()) classify as
    // NotMeasured(NoNetworkPermission). Java-level checks never produce that state,
    // so this isolates the "app has no network permission" banner from everything else.
    val networkBlocked = results?.native?.anyNetworkBlocked() == true

    Column(
        modifier =
            modifier
                .fillMaxSize()
                .padding(horizontal = 16.dp)
                .verticalScroll(rememberScrollState()),
    ) {
        Spacer(Modifier.height(8.dp))

        // One re-check drives every surface: RoutingGateCache.refresh so the export
        // sheet / logcat card banners update immediately too, plus the two caches
        // that actually re-derive their own state off the (now shared) gate.
        val onRetry = {
            RoutingGateCache.refresh(scope, context, selfNeedsRestart)
            DiagnosticsCache.retry(scope, context, selfNeedsRestart)
            DashboardCache.refresh(scope, context, selfNeedsRestart)
        }
        when {
            // Still probing (first load, or a re-check in flight before the shared gate
            // has a value yet).
            liveGate == null -> {
                Box(
                    modifier = Modifier.fillMaxWidth().padding(vertical = 32.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    CircularProgressIndicator()
                }
            }

            liveGate == DiagnosticGate.NEEDS_RESTART -> {
                StatusBanner(
                    text = stringResource(R.string.banner_added_self),
                    containerColor = MaterialTheme.colorScheme.tertiaryContainer,
                    contentColor = MaterialTheme.colorScheme.onTertiaryContainer,
                )
            }

            liveGate == DiagnosticGate.VPN_OFF -> {
                VpnOffPrompt(onRetry = onRetry)
            }

            liveGate == DiagnosticGate.SELF_NOT_ROUTED -> {
                SelfNotRoutedPrompt(onRetry = onRetry)
            }

            // ROUTED: the live gate says the measurement is meaningful — render from
            // DiagnosticsCache's own (frozen, process-scoped) results exactly as before.
            diagState is DiagnosticsCache.State.Failed -> {
                DiagnosticsFailedPrompt(onRetry = onRetry)
            }

            diagState is DiagnosticsCache.State.Running ||
                diagState is DiagnosticsCache.State.NotRun ||
                diagState is DiagnosticsCache.State.Blocked -> {
                // Transitional: the LaunchedEffect(liveGate) above already kicked off
                // (or will kick off) a run now that the gate is ROUTED.
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

                results?.let { r ->
                    // Build the canonical report when the dashboard has loaded so each
                    // check knows whether the active backend OWNS its vector; otherwise
                    // render the raw list (every leak reads as a leak — the pre-report
                    // behaviour, used only in the brief window before the dashboard loads).
                    val report =
                        dashState?.let { ds ->
                            buildDiagnosticReport(
                                gate = DiagnosticGate.ROUTED,
                                results = r,
                                backend = ds.nativeBackend,
                                lsposedActive = ds.lsposed is LsposedState.Active,
                                complete = (diagState as? DiagnosticsCache.State.Ready)?.complete == true,
                                installedOptionalHooks = ds.installedOptionalHooks,
                            )
                        }
                    DiagnosticsResults(report = report, results = r, tallyFmt = tallyFmt)
                }
            }
        }

        Spacer(Modifier.height(16.dp))
    }
}

/**
 * A "Save As…" launcher for a generated `application/zip` file: on a picked uri it
 * copies [source]`()` off the main thread and swallows IO errors (a large zip would
 * block the UI, and a write failure would crash) — logging under [errorLabel].
 * [source] is read at launch time, so it always sees the latest generated file.
 */
@Composable
private fun rememberZipSaveLauncher(
    errorLabel: String,
    mimeType: String = "application/zip",
    source: () -> File?,
): ManagedActivityResultLauncher<String, Uri?> {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    return rememberLauncherForActivityResult(
        ActivityResultContracts.CreateDocument(mimeType),
    ) { uri: Uri? ->
        val src = source() ?: return@rememberLauncherForActivityResult
        if (uri != null) {
            scope.launch(Dispatchers.IO) {
                runCatching {
                    context.contentResolver.openOutputStream(uri)?.use { out ->
                        src.inputStream().use { it.copyTo(out) }
                    }
                }.onFailure { HookLog.e("VpnHide: $errorLabel save failed: ${it.message}") }
            }
        }
    }
}

@Composable
fun DebugToolsSection(
    selfNeedsRestart: Boolean?,
    modifier: Modifier = Modifier,
) {
    var showModal by remember { mutableStateOf(false) }

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

        // The logcat card measures its own gate (persistent while the card is shown).
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
    val cm = context.getSystemService(ConnectivityManager::class.java)
    val scope = rememberCoroutineScope()
    var exporting by remember { mutableStateOf(false) }
    var resultFile by remember { mutableStateOf<File?>(null) }

    // One export recipe, shared with the agent bridge's getState.
    var optForensics by remember { mutableStateOf(true) }
    var optAppList by remember { mutableStateOf(false) }
    var optKernelImage by remember { mutableStateOf(false) }

    // Every export kind is now a .zip (carrying state.json), so one MIME type fits all.
    val saveLauncher = rememberZipSaveLauncher("debug-export") { resultFile }

    // Changing any toggle invalidates a result produced with the old recipe.
    val clearResult = { resultFile = null }
    // Recreated with this composable, so a reopened sheet never flashes the previous
    // verdict while a fresh probe runs (e.g. user closed the blocked sheet, turned the
    // VPN on, reopened). rememberCaptureGate re-measures on first composition = on open.
    val gate = rememberCaptureGate(selfNeedsRestart)
    val incompleteGate = gate.incompleteGate
    val rechecking = gate.rechecking
    val onRecheck = gate.recheck
    ModalBottomSheet(onDismissRequest = { if (!exporting) onDismiss() }) {
        Column(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 24.dp)
                    .padding(bottom = 24.dp),
        ) {
            Text(
                text = stringResource(R.string.debug_export_modal_title),
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Bold,
            )
            Spacer(Modifier.height(12.dp))
            ExportToggle(
                title = stringResource(R.string.debug_export_opt_forensics),
                description = stringResource(R.string.debug_export_opt_forensics_desc),
                checked = optForensics,
                enabled = !exporting,
                onCheckedChange = {
                    optForensics = it
                    clearResult()
                },
            )
            ExportToggle(
                title = stringResource(R.string.debug_export_opt_applist),
                description = stringResource(R.string.debug_export_opt_applist_desc),
                checked = optAppList && optForensics,
                enabled = optForensics && !exporting,
                onCheckedChange = {
                    optAppList = it
                    clearResult()
                },
            )
            ExportToggle(
                title = stringResource(R.string.debug_export_opt_kernel),
                description = stringResource(R.string.debug_export_opt_kernel_desc),
                checked = optKernelImage,
                enabled = !exporting,
                onCheckedChange = {
                    optKernelImage = it
                    clearResult()
                },
            )
            Spacer(Modifier.height(16.dp))

            val doExport = {
                selfNeedsRestart?.let { restartState ->
                    val options =
                        StateContentOptions(forensics = optForensics, appList = optAppList && optForensics)
                    val kernel = optKernelImage
                    exporting = true
                    scope.launch {
                        resultFile = exportDebug(cm, context, restartState, options, kernel)
                        exporting = false
                    }
                }
                Unit
            }

            val file = resultFile

            // Configure phase only: warn that a VPN-off / not-in-tunnel capture
            // will be incomplete. The encouraged action (Re-check) rides on the
            // banner; collecting anyway becomes a de-emphasized escape hatch below.
            if (incompleteGate != null && !exporting && file == null) {
                CaptureIncompleteWarning(
                    gate = incompleteGate,
                    rechecking = rechecking,
                    onRecheck = onRecheck,
                )
                Spacer(Modifier.height(12.dp))
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

                // Configure phase, capture would be incomplete: no prominent primary
                // (that would invite a blind tap) — only "collect anyway" + Cancel.
                file == null && incompleteGate != null -> {
                    TextButton(onClick = doExport, modifier = Modifier.fillMaxWidth()) {
                        Text(stringResource(R.string.capture_collect_anyway))
                    }
                    TextButton(
                        onClick = onDismiss,
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Text(stringResource(R.string.btn_cancel))
                    }
                }

                // Configure phase: the one primary action. Held disabled while a
                // fresh gate re-check is in flight so the user can't collect on a
                // stale "routed" verdict before the VPN-off warning resolves.
                file == null -> {
                    EnhancedButton(
                        onClick = doExport,
                        enabled = selfNeedsRestart != null && !rechecking,
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        if (rechecking) {
                            ButtonSpinner()
                            Spacer(Modifier.width(8.dp))
                        }
                        Text(
                            stringResource(
                                if (rechecking) R.string.capture_checking_vpn else R.string.debug_export_modal_confirm,
                            ),
                        )
                    }
                }

                // Result phase: Save/Share are primary; re-running is a de-emphasized
                // secondary action, so there is no ambiguous "Export" to double-tap.
                else -> {
                    FileSaveShareRow(
                        saveLabel = stringResource(R.string.btn_save_debug),
                        shareLabel = stringResource(R.string.btn_share_debug),
                        sharePrimary = true,
                        onSave = { saveLauncher.launch(file.name) },
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
    val state by LogcatRecorder.state.collectAsState()

    // Own gate: the card is always visible, so it holds its VPN-off/not-in-tunnel
    // verdict until re-checked (re-measured on entry and on the banner's re-check).
    val gate = rememberCaptureGate(selfNeedsRestart)
    val incompleteGate = gate.incompleteGate
    val rechecking = gate.rechecking
    val onRecheck = gate.recheck

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
        rememberZipSaveLauncher("logcat") { (state as? LogcatRecorder.State.Stopped)?.lastFile }

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
                    if (incompleteGate != null) {
                        // Same VPN-off / not-in-tunnel guard as the debug export: a
                        // recording started now captures nothing worth diagnosing.
                        CaptureIncompleteWarning(
                            gate = incompleteGate,
                            rechecking = rechecking,
                            onRecheck = onRecheck,
                        )
                        Spacer(Modifier.height(8.dp))
                        TextButton(
                            onClick = {
                                val restartState = selfNeedsRestart ?: return@TextButton
                                scope.launch { LogcatRecorder.start(context, restartState) }
                            },
                            enabled = selfNeedsRestart != null,
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Text(stringResource(R.string.logcat_start_anyway))
                        }
                    } else {
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
}

/** Composition-scoped capture-gate state: [incompleteGate] non-null when a capture
 * now would be incomplete, [rechecking] while a probe is in flight, [recheck] to
 * re-measure. Lifetime follows the call site — see [rememberCaptureGate]. */
private data class CaptureGate(
    val incompleteGate: DiagnosticGate?,
    val rechecking: Boolean,
    val recheck: () -> Unit,
)

/**
 * Reads the shared [RoutingGateCache] — the same cheap probe (VPN-iface read +
 * self-routing) the export used to run privately via the now-removed
 * `measureCaptureGate` — so a re-check from any surface (this sheet, the logcat
 * card, Dashboard, Diagnostics' own retry) updates every other one through the one
 * underlying [StateFlow].
 *
 * [awaitingFreshSinceOpen] preserves the old per-open freshness guarantee on top of
 * that shared state: a reopened sheet must never flash a stale verdict left over from
 * before it was closed (e.g. the sheet was dismissed VPN-off, the user turns the VPN
 * on, then reopens it — the shared cache still holds the old VPN_OFF gate until this
 * composition's own recheck lands). It starts true and flips to false only once THIS
 * composition's own triggered reload has completed, so [CaptureGate.incompleteGate]
 * reports null (show "checking", not a banner) until then.
 *
 * Scope follows the call site: called under `if (showModal)` the state is recreated
 * on every open, so a reopened sheet always re-measures; called in the always-visible
 * logcat card it persists until re-checked.
 */
@Composable
private fun rememberCaptureGate(selfNeedsRestart: Boolean?): CaptureGate {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val sharedGate by RoutingGateCache.gate.collectAsState()
    val sharedLoading by RoutingGateCache.loading.collectAsState()
    var awaitingFreshSinceOpen by remember { mutableStateOf(true) }

    val recheck: () -> Unit = {
        selfNeedsRestart?.let { needsRestart ->
            awaitingFreshSinceOpen = true
            scope.launch {
                RoutingGateCache.ensureLoaded(scope, context, needsRestart)
                RoutingGateCache.refreshInPlace(force = true)
                awaitingFreshSinceOpen = false
            }
        }
        Unit
    }
    LaunchedEffect(selfNeedsRestart) { recheck() }
    return CaptureGate(
        incompleteGate =
            sharedGate
                ?.takeUnless { awaitingFreshSinceOpen }
                ?.takeIf { it == DiagnosticGate.VPN_OFF || it == DiagnosticGate.SELF_NOT_ROUTED },
        rechecking = awaitingFreshSinceOpen || sharedLoading,
        recheck = recheck,
    )
}

/**
 * Red banner shown before a debug export or a logcat recording when the freshly
 * measured gate says the capture would be incomplete (VPN off, or this app not routed
 * through the tunnel). The encouraged action is a Re-check (re-measures the gate);
 * collecting anyway stays available as a de-emphasized button next to this banner.
 * Message adapts to the gate so the fix is actionable.
 */
@Composable
private fun CaptureIncompleteWarning(
    gate: DiagnosticGate,
    rechecking: Boolean,
    onRecheck: () -> Unit,
) {
    val message =
        when (gate) {
            DiagnosticGate.SELF_NOT_ROUTED -> stringResource(R.string.capture_incomplete_not_routed)
            else -> stringResource(R.string.capture_incomplete_vpn_off)
        }
    StatusBanner(
        text = message,
        containerColor = StatusColors.errorContainer(),
        contentColor = MaterialTheme.colorScheme.onSurface,
        action = {
            EnhancedButton(
                onClick = onRecheck,
                enabled = !rechecking,
                modifier = Modifier.fillMaxWidth(),
            ) {
                if (rechecking) {
                    ButtonSpinner()
                    Spacer(Modifier.width(8.dp))
                }
                Text(stringResource(R.string.capture_incomplete_recheck))
            }
        },
    )
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

/**
 * One row on the Diagnostics list, unified across sources so [CheckCard] renders one
 * shape. [uncovered] marks a native leak on a vector the active backend does not own
 * — a detection surface no active hook covers on this device. It is shown neutrally
 * (see [diagStatusUncovered]), never as a red leak, and grouped apart from the
 * backend's own vectors.
 */
private data class DiagCard(
    val name: String,
    val detail: String,
    val groundTruthDetail: String?,
    val outcome: CheckOutcome,
    val uncovered: Boolean,
    // Hook names the active kernel backend owns but did not install this boot.
    // Turns an unexplained red leak into "your kernel does not expose this".
    val missingHooks: List<String> = emptyList(),
)

/** From a canonical report check — the only source that knows [DiagnosticCheck.owned],
 * so it is the only one that can flag an uncovered native leak. */
private fun DiagnosticCheck.toDiagCard(): DiagCard =
    DiagCard(
        name = label,
        detail = appDetail,
        groundTruthDetail = groundTruthDetail,
        outcome = outcome,
        uncovered = layer == CheckLayer.NATIVE && outcome is CheckOutcome.Leak && !owned,
        missingHooks = missingHooks.map { it.hookName },
    )

/** Raw-list fallback (dashboard not yet loaded): no ownership known, so nothing is
 * marked uncovered — a leak reads as a leak, the pre-report behaviour. */
private fun CheckResult.toDiagCard(): DiagCard = DiagCard(name, detail, groundTruthDetail, outcome, uncovered = false)

/**
 * The results body: the honest headline (hidden vs still-leaking) plus the check
 * cards, split into the backend's own vectors, the vectors no active backend covers
 * on this device (shown neutrally), and the Java layer. [report] is null only in the
 * brief window before the dashboard loads, when we fall back to the raw list.
 */
@Composable
private fun DiagnosticsResults(
    report: DiagnosticReport?,
    results: CheckResults,
    tallyFmt: String,
) {
    val nativeCards = report?.native?.checks?.map { it.toDiagCard() } ?: results.nativeAll.map { it.toDiagCard() }
    val javaCards = report?.java?.checks?.map { it.toDiagCard() } ?: results.java.map { it.toDiagCard() }
    val covered = nativeCards.filterNot { it.uncovered }
    val uncovered = nativeCards.filter { it.uncovered }

    // Headline counts the backend's job: hidden vectors vs still-leaking OWNED
    // vectors. Uncovered vectors are out of the active backend's scope, so they are
    // reported below rather than folded into "leaking".
    val scored = covered + javaCards
    val hidden = scored.count { it.outcome is CheckOutcome.HiddenByBackend || it.outcome is CheckOutcome.HiddenBySelinux }
    val leaks = scored.count { it.outcome is CheckOutcome.Leak }

    Spacer(Modifier.height(12.dp))
    Text(
        text = String.format(tallyFmt, hidden, leaks),
        style = MaterialTheme.typography.titleMedium,
        fontWeight = FontWeight.Bold,
    )

    Spacer(Modifier.height(16.dp))
    SectionHeader(stringResource(R.string.section_native))
    Spacer(Modifier.height(6.dp))
    Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
        covered.forEachIndexed { i, c -> CheckCard(c, index = i, count = covered.size) }
    }

    if (uncovered.isNotEmpty()) {
        Spacer(Modifier.height(16.dp))
        SectionHeader(stringResource(R.string.section_native_uncovered))
        Spacer(Modifier.height(4.dp))
        Text(
            text = stringResource(R.string.diag_uncovered_caption),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(6.dp))
        Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
            uncovered.forEachIndexed { i, c -> CheckCard(c, index = i, count = uncovered.size) }
        }
    }

    Spacer(Modifier.height(16.dp))
    SectionHeader(stringResource(R.string.section_java))
    Spacer(Modifier.height(6.dp))
    Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
        javaCards.forEachIndexed { i, c -> CheckCard(c, index = i, count = javaCards.size) }
    }
}

/**
 * One check's status: a coloured **dot + short word** (the "3-B" treatment — the
 * app's own status-dot idiom from the module rows). The card colour tracks current
 * reality only — green when hidden (by backend OR SELinux) or nothing-to-leak, red
 * on a real leak, neutral when not measured — so a normal enforcing device is all
 * green. The backend-vs-SELinux attribution rides on the dot colour + word, never
 * on the card colour (no alarm on SELinux-protected items). Detail is collapsed by
 * default and revealed on tap; a leak is the only thing expanded up front.
 */
private data class DiagStatus(
    val label: String,
    val accent: Color,
    val container: Color,
    val expandedByDefault: Boolean,
)

// Thin renderer: the bucket decision lives in [diagStatusKind] (pure, unit-tested);
// here we only attach the localized word + theme colour. A leak is the one thing
// expanded by default. NothingToLeak/SELinux keep the no-alarm green container but
// a distinct dot + word (grey "nothing", blue "SELinux") so attribution shows.
@Composable
private fun diagStatus(outcome: CheckOutcome): DiagStatus =
    when (outcome.diagStatusKind()) {
        DiagStatusKind.Ok -> {
            DiagStatus(stringResource(R.string.diag_status_ok), StatusColors.successDot, StatusColors.successContainer(), false)
        }

        DiagStatusKind.Leak -> {
            DiagStatus(stringResource(R.string.diag_status_leak), StatusColors.errorDot, StatusColors.errorContainer(), true)
        }

        DiagStatusKind.NothingToLeak -> {
            DiagStatus(stringResource(R.string.diag_status_nothing), StatusColors.neutralAccent, StatusColors.successContainer(), false)
        }

        DiagStatusKind.Selinux -> {
            DiagStatus(stringResource(R.string.diag_status_selinux), StatusColors.infoAccent, StatusColors.successContainer(), false)
        }

        DiagStatusKind.NotMeasured -> {
            DiagStatus(stringResource(R.string.diag_status_nomeasure), StatusColors.neutralAccent, StatusColors.neutralContainer(), false)
        }
    }

/** Neutral "out of scope" status for a native leak on a vector the active backend
 * does not own: no active hook covers it on this device, so it is not the backend
 * failing — it is reported calmly (grey dot + word, no alarm, collapsed), never as a
 * red leak, so a working backend never reads as broken over a gap it cannot close. */
@Composable
private fun diagStatusUncovered(): DiagStatus =
    DiagStatus(
        stringResource(R.string.diag_status_uncovered),
        StatusColors.neutralAccent,
        StatusColors.neutralContainer(),
        false,
    )

@Composable
private fun CheckCard(
    r: DiagCard,
    index: Int = -1,
    count: Int = 1,
) {
    val status = if (r.uncovered) diagStatusUncovered() else diagStatus(r.outcome)
    var expanded by remember(r.name) { mutableStateOf(status.expandedByDefault) }
    val caretRotation by animateFloatAsState(if (expanded) 90f else 0f, label = "caret")
    GroupedCard(
        index = index,
        count = count,
        modifier = Modifier.fillMaxWidth().clickable { expanded = !expanded },
        color = status.container,
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
                    modifier = Modifier.weight(1f).padding(end = 8.dp),
                )
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Box(modifier = Modifier.size(9.dp).clip(CircleShape).background(status.accent))
                    Text(
                        text = status.label,
                        color = status.accent,
                        fontWeight = FontWeight.SemiBold,
                        fontSize = 12.sp,
                    )
                    Icon(
                        imageVector = Icons.AutoMirrored.Filled.KeyboardArrowRight,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.size(18.dp).rotate(caretRotation),
                    )
                }
            }
            if (expanded) {
                Spacer(Modifier.height(6.dp))
                // For native checks the root ground-truth detail is shown next to the
                // app-view read — it is what the verdict is derived from (e.g. a
                // SELinux-blocked read reads as "nothing to leak" precisely because
                // "root: N routes, no VPN"). Java checks have no root diff → plain detail.
                val detailText =
                    r.groundTruthDetail?.let { gt -> "app:  ${r.detail}\nroot: $gt" } ?: r.detail
                Text(
                    text = detailText,
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = FontFamily.Monospace,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.8f),
                )
                if (r.missingHooks.isNotEmpty()) {
                    Spacer(Modifier.height(6.dp))
                    Text(
                        text =
                            stringResource(
                                R.string.diag_check_hook_missing,
                                r.missingHooks.joinToString(", "),
                            ),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }
    }
}
