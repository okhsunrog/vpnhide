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

private const val TAG = "VPNHideTest"

data class CheckResult(
    val name: String,
    val passed: Boolean?,
    val detail: String,
)

internal data class CheckResults(
    // UniFFI native probes from the fast phase.
    val native: List<CheckResult>,
    // Java-implemented native-level probes (NetworkInterface enum, /proc/net/route)
    // — shown under "Native level" and included in Dashboard once the full
    // diagnostics result is ready.
    val nativeExtra: List<CheckResult> = emptyList(),
    // VPN-presence probes from the fast phase.
    val coreJava: List<CheckResult> = emptyList(),
    // Remaining Java probes (active-network, push callback, routes, proxy) —
    // the slow push-callback check lives here, so it runs in a second phase.
    val extraJava: List<CheckResult> = emptyList(),
) {
    val nativeAll get() = native + nativeExtra
    val java get() = coreJava + extraJava
    val all get() = nativeAll + java
}

/** Number of passed checks out of those that actually ran (NETWORK_BLOCKED
 * probes report `passed == null` and are excluded from the denominator). */
internal data class CheckScore(
    val passed: Int,
    val total: Int,
)

internal fun List<CheckResult>.score(): CheckScore {
    val scored = filter { it.passed != null }
    return CheckScore(passed = scored.count { it.passed == true }, total = scored.size)
}

internal suspend fun isVpnActive(): Boolean {
    val snapshot = RootSnapshotCache.getOrLoad()
    return isVpnActiveFromSnapshot(snapshot.sections["vpn_ifaces"].orEmpty())
}

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
                context.contentResolver.openOutputStream(uri)?.use { out ->
                    zip.inputStream().use { it.copyTo(out) }
                }
            }
        }

    Column(
        modifier = modifier.fillMaxWidth(),
    ) {
        DebugLoggingCard()

        Spacer(Modifier.height(16.dp))

        LogcatRecordCard()

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
private fun DebugLoggingCard() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var enabled by remember { mutableStateOf(VpnHideLog.enabled) }

    EnhancedCard(
        modifier = Modifier.fillMaxWidth(),
        color = MaterialTheme.colorScheme.surfaceVariant,
        shape = MaterialTheme.shapes.medium,
    ) {
        Row(
            modifier = Modifier.padding(16.dp).fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(Modifier.weight(1f)) {
                Text(
                    text = stringResource(R.string.diag_debug_logging_title),
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.Bold,
                )
                Spacer(Modifier.height(4.dp))
                Text(
                    text = stringResource(R.string.diag_debug_logging_description),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Spacer(Modifier.width(12.dp))
            Switch(
                checked = enabled,
                onCheckedChange = { newValue ->
                    enabled = newValue
                    scope.launch(Dispatchers.IO) {
                        setDebugLoggingEnabled(context, newValue)
                    }
                },
            )
        }
    }
}

@Composable
private fun LogcatRecordCard() {
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
            ActivityResultContracts.CreateDocument("text/plain"),
        ) { uri: Uri? ->
            val src = (state as? LogcatRecorder.State.Stopped)?.lastFile ?: return@rememberLauncherForActivityResult
            if (uri != null) {
                context.contentResolver.openOutputStream(uri)?.use { out ->
                    src.inputStream().use { it.copyTo(out) }
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
                            onShare = { shareFileViaProvider(context, last, "text/plain") },
                        )
                        Spacer(Modifier.height(8.dp))
                    }
                    EnhancedButton(
                        onClick = {
                            scope.launch { LogcatRecorder.start(context) }
                        },
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
private fun SectionHeader(title: String) {
    Text(
        text = title,
        style = MaterialTheme.typography.titleSmall,
        fontWeight = FontWeight.Bold,
        color = MaterialTheme.colorScheme.primary,
    )
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

// ==========================================================================
//  Check runner — runs directly in the main process
// ==========================================================================

/**
 * The checks split into two phases so Settings → Detailed diagnostics can show progress
 * before the slow probes finish. [runCoreChecks] runs the fast native and
 * VPN-presence Java probes; [runExtraJavaChecks] runs the remaining Java probes,
 * including the push-callback probe that blocks for up to 3s. Dashboard waits
 * for the complete DiagnosticsCache result before summarizing protection.
 */
internal fun runCoreChecks(
    cm: ConnectivityManager,
    context: android.content.Context,
): CheckResults {
    VpnHideLog.i(TAG, "========================================")
    VpnHideLog.i(TAG, "=== VPNHide — starting checks (core phase) ===")
    VpnHideLog.i(TAG, "========================================")

    val res = context.resources

    val native = NATIVE_CHECKS.map { spec -> nativeCheck(res.getString(spec.labelRes), spec.run) }

    val nativeExtra =
        listOf(
            checkNetworkInterfaceEnum(res.getString(R.string.check_net_iface_enum)),
            checkProcNetRouteJava(res.getString(R.string.check_proc_route_java)),
        ).logged()

    val coreJava =
        listOf(
            checkHasTransportVpn(cm, res.getString(R.string.check_has_transport_vpn)),
            checkHasCapabilityNotVpn(cm, res.getString(R.string.check_has_capability_not_vpn)),
            checkTransportInfo(cm, res.getString(R.string.check_transport_info)),
            checkAllNetworksVpn(cm, res.getString(R.string.check_all_networks_vpn)),
            checkLinkPropertiesIfname(cm, res.getString(R.string.check_link_properties)),
        ).logged()

    return CheckResults(native = native, nativeExtra = nativeExtra, coreJava = coreJava)
}

internal fun runExtraJavaChecks(
    cm: ConnectivityManager,
    context: android.content.Context,
): List<CheckResult> {
    val res = context.resources
    return listOf(
        checkNetworkForTypeVpn(cm, res.getString(R.string.check_network_for_type_vpn)),
        checkActiveNetworkHandle(cm, res.getString(R.string.check_active_network_handle)),
        checkAllNetworksHandles(cm, res.getString(R.string.check_all_networks_handles)),
        checkActiveNetworkVpn(cm, res.getString(R.string.check_active_network_vpn)),
        checkNetworkCallbackVpn(cm, res.getString(R.string.check_network_callback)),
        checkLinkPropertiesRoutes(cm, res.getString(R.string.check_link_properties_routes)),
        checkProxyHost(res.getString(R.string.check_proxy_host)),
    ).logged()
}

/** Log each Java check result; native probes already log via [nativeCheck]. */
private fun List<CheckResult>.logged(): List<CheckResult> =
    onEach { c ->
        val status =
            when (c.passed) {
                true -> "PASS"
                false -> "FAIL"
                null -> "SKIP"
            }
        VpnHideLog.i(TAG, "[${c.name}] $status: ${c.detail}")
    }

/** Run both phases and return the complete results. Used where blocking on the
 * slow probes is fine (debug export); the live cache runs the phased builders
 * directly so Settings → Detailed diagnostics can show the fast phase first. */
internal fun runAllChecks(
    cm: ConnectivityManager,
    context: android.content.Context,
): CheckResults = runCoreChecks(cm, context).copy(extraJava = runExtraJavaChecks(cm, context))

private fun nativeCheck(
    name: String,
    block: () -> CheckOutput,
): CheckResult =
    try {
        val out = block()
        VpnHideLog.i(TAG, "[$name] ${out.status}: ${out.detail}")
        CheckResult(name, out.status.toPassed(), out.detail)
    } catch (e: Exception) {
        val detail = e.message ?: e.javaClass.simpleName
        Log.e(TAG, "[$name] $detail", e)
        CheckResult(name, false, detail)
    }

// ==========================================================================
//  Java API checks
// ==========================================================================

/** Shared preamble for the capability-based checks: resolve the active
 * network's [NetworkCapabilities], reporting "no active network" / "no
 * capabilities" (both PASS — nothing for an app to leak) when absent. */
private inline fun withActiveCaps(
    cm: ConnectivityManager,
    name: String,
    body: (NetworkCapabilities) -> CheckResult,
): CheckResult {
    val net = cm.activeNetwork ?: return CheckResult(name, true, "no active network")
    val caps = cm.getNetworkCapabilities(net) ?: return CheckResult(name, true, "no capabilities")
    return body(caps)
}

/** Shared preamble for the LinkProperties-based checks. */
private inline fun withActiveLinkProperties(
    cm: ConnectivityManager,
    name: String,
    body: (LinkProperties) -> CheckResult,
): CheckResult {
    val net = cm.activeNetwork ?: return CheckResult(name, true, "no active network")
    val lp = cm.getLinkProperties(net) ?: return CheckResult(name, true, "no link properties")
    return body(lp)
}

internal fun checkHasTransportVpn(
    cm: ConnectivityManager,
    name: String,
): CheckResult =
    withActiveCaps(cm, name) { caps ->
        val hasVpn = caps.hasTransport(NetworkCapabilities.TRANSPORT_VPN)
        val hasWifi = caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)
        val hasCellular = caps.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)
        val detail =
            if (!hasVpn) {
                "hasTransport(VPN)=false, WIFI=$hasWifi, CELLULAR=$hasCellular"
            } else {
                "hasTransport(VPN)=true, WIFI=$hasWifi, CELLULAR=$hasCellular"
            }
        CheckResult(name, !hasVpn, detail)
    }

internal fun checkHasCapabilityNotVpn(
    cm: ConnectivityManager,
    name: String,
): CheckResult =
    withActiveCaps(cm, name) { caps ->
        val notVpn = caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
        val detail = if (notVpn) "NOT_VPN capability present" else "NOT_VPN capability MISSING"
        CheckResult(name, notVpn, detail)
    }

internal fun checkTransportInfo(
    cm: ConnectivityManager,
    name: String,
): CheckResult =
    withActiveCaps(cm, name) { caps ->
        val info = caps.transportInfo
        val className = info?.javaClass?.name ?: "null"
        val isVpn = className.contains("VpnTransportInfo")
        val detail = if (!isVpn) "transportInfo=$className" else "VpnTransportInfo: $info"
        CheckResult(name, !isVpn, detail)
    }

private fun checkNetworkInterfaceEnum(name: String): CheckResult =
    try {
        val ifaces =
            NetworkInterface.getNetworkInterfaces()
                ?: return CheckResult(name, true, "returned null")
        val allNames = mutableListOf<String>()
        val vpnNames = mutableListOf<String>()
        for (iface in ifaces) {
            allNames.add(iface.name)
            if (IfaceLists.isVpnIface(iface.name)) vpnNames.add(iface.name)
        }
        val detail =
            if (vpnNames.isEmpty()) {
                "${allNames.size} ifaces [${allNames.joinToString()}], no VPN"
            } else {
                "VPN [${vpnNames.joinToString()}] in [${allNames.joinToString()}]"
            }
        CheckResult(name, vpnNames.isEmpty(), detail)
    } catch (e: Exception) {
        CheckResult(name, false, "${e.message}")
    }

@Suppress("DEPRECATION")
internal fun checkAllNetworksVpn(
    cm: ConnectivityManager,
    name: String,
): CheckResult {
    val networks = cm.allNetworks
    if (networks.isEmpty()) return CheckResult(name, true, "no networks")
    val vpnNetworks =
        networks.filter { net ->
            cm.getNetworkCapabilities(net)?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) == true
        }
    val detail =
        if (vpnNetworks.isEmpty()) {
            "${networks.size} networks, none have TRANSPORT_VPN"
        } else {
            "${vpnNetworks.size} network(s) with TRANSPORT_VPN"
        }
    return CheckResult(name, vpnNetworks.isEmpty(), detail)
}

private fun checkActiveNetworkVpn(
    cm: ConnectivityManager,
    name: String,
): CheckResult =
    withActiveCaps(cm, name) { caps ->
        val transports = mutableListOf<String>()
        mapOf(
            NetworkCapabilities.TRANSPORT_CELLULAR to "CELLULAR",
            NetworkCapabilities.TRANSPORT_WIFI to "WIFI",
            NetworkCapabilities.TRANSPORT_BLUETOOTH to "BLUETOOTH",
            NetworkCapabilities.TRANSPORT_ETHERNET to "ETHERNET",
            NetworkCapabilities.TRANSPORT_VPN to "VPN",
            NetworkCapabilities.TRANSPORT_WIFI_AWARE to "WIFI_AWARE",
        ).forEach { (id, label) -> if (caps.hasTransport(id)) transports.add(label) }
        val hasVpn = caps.hasTransport(NetworkCapabilities.TRANSPORT_VPN)
        val detail =
            if (!hasVpn) {
                "transports=[${transports.joinToString()}], no VPN"
            } else {
                "transports include VPN: [${transports.joinToString()}]"
            }
        CheckResult(name, !hasVpn, detail)
    }

private data class NetworkForTypeResult(
    val network: Network? = null,
    val unavailable: Boolean = false,
    val error: String? = null,
)

@Suppress("DEPRECATION")
private fun queryNetworkForType(
    cm: ConnectivityManager,
    type: Int,
): NetworkForTypeResult =
    try {
        val method = ConnectivityManager::class.java.getMethod("getNetworkForType", Integer.TYPE)
        method.isAccessible = true
        NetworkForTypeResult(network = method.invoke(cm, type) as? Network)
    } catch (_: NoSuchMethodException) {
        NetworkForTypeResult(unavailable = true)
    } catch (t: Throwable) {
        NetworkForTypeResult(error = t.cause?.message ?: t.message ?: t.javaClass.simpleName)
    }

@Suppress("DEPRECATION")
private fun checkNetworkForTypeVpn(
    cm: ConnectivityManager,
    name: String,
): CheckResult {
    val result = queryNetworkForType(cm, ConnectivityManager.TYPE_VPN)
    result.error?.let { return CheckResult(name, false, it) }
    if (result.unavailable) return CheckResult(name, true, "getNetworkForType unavailable")
    val vpnNetwork = result.network ?: return CheckResult(name, true, "TYPE_VPN returned null")

    val caps = cm.getNetworkCapabilities(vpnNetwork)
    val hasVpn = caps?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) == true
    val detail =
        if (hasVpn) {
            "TYPE_VPN returned $vpnNetwork with VPN capabilities"
        } else {
            "TYPE_VPN returned $vpnNetwork"
        }
    return CheckResult(name, false, detail)
}

@Suppress("DEPRECATION")
private fun checkActiveNetworkHandle(
    cm: ConnectivityManager,
    name: String,
): CheckResult {
    val active = cm.activeNetwork ?: return CheckResult(name, true, "no active network")
    val vpnResult = queryNetworkForType(cm, ConnectivityManager.TYPE_VPN)
    vpnResult.error?.let { return CheckResult(name, false, it) }
    if (vpnResult.unavailable) return CheckResult(name, true, "active=$active, getNetworkForType unavailable")
    val vpnNetwork = vpnResult.network ?: return CheckResult(name, true, "active=$active, TYPE_VPN not exposed")

    val leaksVpnHandle = active == vpnNetwork
    val detail =
        if (leaksVpnHandle) {
            "activeNetwork equals TYPE_VPN network $active"
        } else {
            "active=$active, TYPE_VPN=$vpnNetwork"
        }
    return CheckResult(name, !leaksVpnHandle, detail)
}

@Suppress("DEPRECATION")
private fun checkAllNetworksHandles(
    cm: ConnectivityManager,
    name: String,
): CheckResult {
    val networks = cm.allNetworks
    val vpnResult = queryNetworkForType(cm, ConnectivityManager.TYPE_VPN)
    vpnResult.error?.let { return CheckResult(name, false, it) }
    if (vpnResult.unavailable) return CheckResult(name, true, "${networks.size} networks, getNetworkForType unavailable")
    val vpnNetwork = vpnResult.network ?: return CheckResult(name, true, "${networks.size} networks, TYPE_VPN not exposed")

    val containsVpnHandle = networks.any { it == vpnNetwork }
    val detail =
        if (containsVpnHandle) {
            "allNetworks includes TYPE_VPN network $vpnNetwork"
        } else {
            "${networks.size} networks, TYPE_VPN=$vpnNetwork not listed"
        }
    return CheckResult(name, !containsVpnHandle, detail)
}

// Push-callback leak (issue #70, e.g. VTB): apps using
// registerDefaultNetworkCallback receive NetworkCapabilities *pushed* from
// system_server. The writeToParcel hook keys off Binder.getCallingUid(), which
// on the callback path is system_server (1000), not the app — so it doesn't
// sanitize, and the app sees the real VPN through the callback even though the
// synchronous getNetworkCapabilities() is clean. We read caps via the callback
// and fail if VPN is still visible.
internal fun checkNetworkCallbackVpn(
    cm: ConnectivityManager,
    name: String,
): CheckResult {
    val latch = CountDownLatch(1)
    val seen = AtomicReference<NetworkCapabilities?>(null)
    val callback =
        object : ConnectivityManager.NetworkCallback() {
            override fun onCapabilitiesChanged(
                network: Network,
                caps: NetworkCapabilities,
            ) {
                seen.set(caps)
                latch.countDown()
            }
        }
    return try {
        cm.registerDefaultNetworkCallback(callback)
        val fired = latch.await(3, TimeUnit.SECONDS)
        val caps = seen.get()
        if (!fired || caps == null) {
            CheckResult(name, true, "no callback delivered")
        } else {
            val hasVpn = caps.hasTransport(NetworkCapabilities.TRANSPORT_VPN)
            val notVpn = caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
            val leaked = hasVpn || !notVpn
            val detail =
                if (!leaked) {
                    "callback caps clean (no VPN transport, NOT_VPN present)"
                } else {
                    "callback leaks VPN: hasTransport(VPN)=$hasVpn, NOT_VPN=$notVpn"
                }
            CheckResult(name, !leaked, detail)
        }
    } catch (e: Exception) {
        CheckResult(name, false, e.message ?: e.javaClass.simpleName)
    } finally {
        runCatching { cm.unregisterNetworkCallback(callback) }
    }
}

internal fun checkLinkPropertiesIfname(
    cm: ConnectivityManager,
    name: String,
): CheckResult =
    withActiveLinkProperties(cm, name) { lp ->
        val ifname = lp.interfaceName ?: "(null)"
        val routes = lp.routes.map { "${it.destination} via ${it.gateway} dev ${it.`interface`}" }
        val dns = lp.dnsServers.map { it.hostAddress ?: "?" }
        val isVpn = IfaceLists.isVpnIface(ifname)
        val detail =
            if (!isVpn) {
                "ifname=$ifname, ${routes.size} routes, dns=[${dns.joinToString()}]"
            } else {
                "ifname=$ifname is a VPN interface"
            }
        CheckResult(name, !isVpn, detail)
    }

private fun checkLinkPropertiesRoutes(
    cm: ConnectivityManager,
    name: String,
): CheckResult =
    withActiveLinkProperties(cm, name) { lp ->
        val routes = lp.routes
        val vpnRoutes =
            routes.filter { route ->
                val iface = route.`interface` ?: return@filter false
                IfaceLists.isVpnIface(iface)
            }
        val detail =
            if (vpnRoutes.isEmpty()) {
                "${routes.size} routes, none via VPN interfaces"
            } else {
                "${vpnRoutes.size} route(s) via VPN"
            }
        CheckResult(name, vpnRoutes.isEmpty(), detail)
    }

private fun checkProxyHost(name: String): CheckResult {
    val httpHost = System.getProperty("http.proxyHost")
    val socksHost = System.getProperty("socksProxyHost")
    val hasProxy = !httpHost.isNullOrEmpty() || !socksHost.isNullOrEmpty()
    val detail =
        if (!hasProxy) {
            "no proxy (http=$httpHost, socks=$socksHost)"
        } else {
            val httpPort = System.getProperty("http.proxyPort")
            val socksPort = System.getProperty("socksProxyPort")
            "proxy found — http=$httpHost:$httpPort, socks=$socksHost:$socksPort"
        }
    return CheckResult(name, !hasProxy, detail)
}

private fun checkProcNetRouteJava(name: String): CheckResult =
    try {
        val allLines = mutableListOf<String>()
        val vpnLines = mutableListOf<String>()
        BufferedReader(InputStreamReader(java.io.FileInputStream("/proc/net/route"))).use { br ->
            while (true) {
                val line = br.readLine() ?: break
                allLines.add(line)
                // /proc/net/route is whitespace-separated; check
                // each token instead of just startsWith on the raw
                // line so we don't match e.g. an IP-as-hex by chance.
                if (line.split(Regex("\\s+")).any(IfaceLists::isVpnIface)) {
                    vpnLines.add(line.take(60))
                }
            }
        }
        val detail =
            if (vpnLines.isEmpty()) {
                "${allLines.size} lines, no VPN entries"
            } else {
                "${vpnLines.size} VPN lines:\n${vpnLines.joinToString("\n") { "  $it" }}"
            }
        CheckResult(name, vpnLines.isEmpty(), detail)
    } catch (e: Exception) {
        val msg = e.message ?: ""
        if (msg.contains("EACCES") || msg.contains("Permission denied")) {
            CheckResult(name, true, "access denied by SELinux")
        } else {
            CheckResult(name, false, "${e.message}")
        }
    }

// ==========================================================================
//  Debug log export
// ==========================================================================

private suspend fun exportDebugZip(
    cm: ConnectivityManager,
    context: android.content.Context,
    selfNeedsRestart: Boolean,
): File? =
    withContext(Dispatchers.IO) {
        // Force-enable debug logging across all four sinks (app, system_server,
        // zygisk, kmod) while the capture runs so the dump contains VpnHide-
        // tagged lines + verbose dmesg even when the user's persistent toggle
        // is OFF (the default). We restore to whatever the SharedPreferences
        // say at the end — if the user happens to flip the UI toggle mid-
        // capture, we honor their final choice instead of blindly rolling
        // back. applyDebugLoggingRuntime drives all four sinks uniformly, so
        // there's no ad-hoc /proc/vpnhide_debug flip here anymore.
        val loggingWasForced = !VpnHideLog.enabled
        if (loggingWasForced) applyDebugLoggingRuntime(true)
        try {
            // 1. Clear dmesg so we only capture fresh output from the
            //    kmod hooks fired by runAllChecks below.
            suExec("dmesg -c > /dev/null 2>&1")

            // 2. Run all diagnostic checks (this triggers kmod hooks)
            val checkResults = runAllChecks(cm, context)

            // 3. Capture dmesg right after checks
            val (_, dmesg) = suExec("dmesg 2>/dev/null")

            // 4. Collect everything into named files — each section is its own
            //    builder below.
            val files =
                mapOf(
                    "dmesg_vpnhide.txt" to dmesg.lines().filter { it.contains("vpnhide") }.joinToString("\n"),
                    "dmesg_full.txt" to dmesg,
                    "diagnostics.txt" to buildDiagnosticsText(checkResults),
                    "device_info.txt" to buildDeviceInfoText(context, selfNeedsRestart),
                    "modules.txt" to buildModuleInfoText(),
                    "config.txt" to buildTargetsText(),
                    "interfaces.txt" to buildInterfacesText(),
                    "proc_net.txt" to buildProcNetText(),
                    "logcat.txt" to captureDebugLogcat().ifEmpty { "(no logcat entries)" },
                )

            // Create zip
            val timestamp = SimpleDateFormat("yyyyMMdd_HHmmss", Locale.US).format(Date())
            val zipFile = File(context.cacheDir, "vpnhide_debug_$timestamp.zip")
            ZipOutputStream(zipFile.outputStream()).use { zos ->
                for ((name, content) in files) {
                    zos.putNextEntry(ZipEntry(name))
                    zos.write(content.toByteArray())
                    zos.closeEntry()
                }
            }
            zipFile
        } catch (e: Exception) {
            Log.e(TAG, "Debug export failed", e)
            null
        } finally {
            if (loggingWasForced) {
                val target = isEnabledInPrefs(context)
                if (VpnHideLog.enabled != target) applyDebugLoggingRuntime(target)
            }
        }
    }

// ── Debug-zip section builders (each produces one file in the export) ─────

private fun badge(passed: Boolean?): String =
    when (passed) {
        true -> "PASS"
        false -> "FAIL"
        null -> "INFO"
    }

private fun buildDiagnosticsText(results: CheckResults): String =
    buildString {
        val score = results.all.score()
        appendLine("=== Diagnostics: ${score.passed}/${score.total} passed ===")
        appendLine()
        appendLine("--- Native level ---")
        for (c in results.nativeAll) {
            appendLine("[${badge(c.passed)}] ${c.name}")
            appendLine("  ${c.detail}")
        }
        appendLine()
        appendLine("--- Java API level ---")
        for (c in results.java) {
            appendLine("[${badge(c.passed)}] ${c.name}")
            appendLine("  ${c.detail}")
        }
    }

private fun buildDeviceInfoText(
    context: android.content.Context,
    selfNeedsRestart: Boolean,
): String {
    val (_, kernelVersion) = suExec("uname -r 2>/dev/null")
    val (_, procVersion) = suExec("cat /proc/version 2>/dev/null")
    val (_, selinuxMode) = suExec("getenforce 2>/dev/null")
    return buildString {
        appendLine("Device: ${Build.MANUFACTURER} ${Build.MODEL}")
        appendLine("Android: ${Build.VERSION.RELEASE} (SDK ${Build.VERSION.SDK_INT})")
        appendLine("Kernel: ${kernelVersion.trim()}")
        appendLine("Kernel full: ${procVersion.trim()}")
        appendLine("ABI: ${Build.SUPPORTED_ABIS.joinToString()}")
        appendLine("SELinux: ${selinuxMode.trim()}")
        appendLine("App package: ${context.packageName}")
        try {
            val pInfo = context.packageManager.getPackageInfo(context.packageName, 0)
            appendLine("App version: ${pInfo.versionName}")
        } catch (_: Exception) {
        }
        appendLine("selfNeedsRestart: $selfNeedsRestart")
        appendLine()
        appendLine("=== Root manager ===")
        val (_, magiskVer) = suExec("magisk -V 2>/dev/null")
        val (_, magiskVerName) = suExec("magisk -v 2>/dev/null")
        if (magiskVer.isNotBlank()) {
            appendLine("Magisk: ${magiskVerName.trim()} (${magiskVer.trim()})")
        }
        val (_, ksuVer) = suExec("cat /data/adb/ksu/version 2>/dev/null")
        if (ksuVer.isNotBlank()) {
            appendLine("KernelSU: ${ksuVer.trim()}")
        }
        val (exitKsuNext, ksuNextVer) = suExec("ksud --version 2>/dev/null")
        if (exitKsuNext == 0 && ksuNextVer.isNotBlank()) {
            appendLine("KernelSU-Next: ${ksuNextVer.trim()}")
        }
        if (magiskVer.isBlank() && ksuVer.isBlank() && (exitKsuNext != 0 || ksuNextVer.isBlank())) {
            appendLine("(unknown root manager)")
        }
    }
}

private fun buildModuleInfoText(): String =
    buildString {
        appendLine("=== Kernel module (kmod) ===")
        val (_, kmodProp) = suExec("cat /data/adb/modules/vpnhide_kmod/module.prop 2>/dev/null")
        appendLine(kmodProp.ifEmpty { "Not installed" })
        appendLine()
        appendLine("=== kmod load_status (boot-time diagnostics) ===")
        val (_, loadStatus) = suExec("cat $KMOD_LOAD_STATUS_FILE 2>/dev/null")
        appendLine(loadStatus.ifEmpty { "(not available — module never ran post-fs-data.sh this boot)" })
        appendLine()
        appendLine("=== Current boot_id ===")
        val (_, curBootId) = suExec("cat /proc/sys/kernel/random/boot_id 2>/dev/null")
        appendLine(curBootId.trim().ifEmpty { "(not available)" })
        appendLine()
        appendLine("=== kmod load_dmesg ===")
        val (_, loadDmesg) = suExec("cat $KMOD_LOAD_DMESG_FILE 2>/dev/null")
        appendLine(loadDmesg.ifEmpty { "(not captured)" })
        appendLine()
        appendLine("=== Zygisk module ===")
        val (_, zygiskProp) = suExec("cat /data/adb/modules/vpnhide_zygisk/module.prop 2>/dev/null")
        appendLine(zygiskProp.ifEmpty { "Not installed" })
        appendLine()
        appendLine("=== Registered kretprobes ===")
        val (_, kprobes) = suExec("cat /sys/kernel/debug/kprobes/list 2>/dev/null | grep vpnhide")
        appendLine(kprobes.ifEmpty { "(not available or no vpnhide probes)" })
        appendLine()
        appendLine("=== Kernel symbols (hooked functions) ===")
        val symbols =
            listOf("dev_ioctl", "dev_ifconf", "rtnl_fill_ifinfo", "inet6_fill_ifaddr", "inet_fill_ifaddr", "fib_route_seq_show")
        for (sym in symbols) {
            val (_, line) = suExec("cat /proc/kallsyms 2>/dev/null | grep -w $sym | head -3")
            appendLine("$sym: ${line.trim().ifEmpty { "(not found)" }}")
        }
        appendLine()
        appendLine("=== LSPosed configuration ===")
        val (_, lsposedDb) =
            suExec(
                "sqlite3 /data/adb/lspd/config/modules_config.db " +
                    "\"SELECT mid, module_pkg_name, enabled FROM modules WHERE module_pkg_name LIKE '%vpnhide%';\" 2>/dev/null",
            )
        appendLine(lsposedDb.ifEmpty { "(not available or module not in LSPosed)" })
        val (_, lsposedScope) =
            suExec(
                "sqlite3 /data/adb/lspd/config/modules_config.db " +
                    "\"SELECT s.app_pkg_name FROM scope s JOIN modules m ON s.mid=m.mid WHERE m.module_pkg_name LIKE '%vpnhide%';\" 2>/dev/null",
            )
        if (lsposedScope.isNotBlank()) {
            appendLine("Scope: ${lsposedScope.trim()}")
        }
    }

private fun buildTargetsText(): String =
    buildString {
        appendLine("=== /proc/vpnhide_ctl (live status + stats) ===")
        appendLine(suExec("cat $PROC_CTL 2>/dev/null").second.ifEmpty { "(empty)" })
        appendLine()
        appendLine("=== LSPosed state (live status + stats) ===")
        appendLine(suExec("cat $LSPOSED_STATE_FILE 2>/dev/null").second.ifEmpty { "(empty)" })
        appendLine()
        appendLine("=== canonical config ===")
        appendLine(suExec("cat $CANONICAL_CONFIG_FILE 2>/dev/null").second.ifEmpty { "(empty)" })
    }

private fun buildInterfacesText(): String =
    buildString {
        appendLine("=== ip -d addr ===")
        appendLine(suExec("ip -d addr 2>/dev/null").second.ifEmpty { "(not available)" })
        appendLine()
        appendLine("=== Interface operstate ===")
        val (_, operstate) =
            suExec(
                "for iface in /sys/class/net/*; do " +
                    "echo \"\$(basename \$iface): \$(cat \$iface/operstate 2>/dev/null)\"; " +
                    "done",
            )
        appendLine(operstate.ifEmpty { "(not available)" })
        appendLine()
        appendLine("=== ip route show table all ===")
        appendLine(suExec("ip route show table all 2>/dev/null").second.ifEmpty { "(not available)" })
        appendLine()
        appendLine("=== ip rule ===")
        appendLine(suExec("ip rule 2>/dev/null").second.ifEmpty { "(not available)" })
    }

private fun buildProcNetText(): String =
    buildString {
        for (pf in listOf("route", "ipv6_route", "if_inet6", "tcp", "tcp6", "udp", "udp6", "dev")) {
            appendLine("=== /proc/net/$pf ===")
            appendLine(suExec("cat /proc/net/$pf 2>/dev/null").second.ifEmpty { "(not available)" })
            appendLine()
        }
    }

private fun captureDebugLogcat(): String {
    val tags =
        listOf(
            "VPNHideTest:*",
            "VpnHide:*",
            "VpnHide-Dashboard:*",
            "VpnHide-LSPosed:*",
            // zygisk's android_logger uses this tag (see zygisk/src/lib.rs:LOG_TAG);
            // without it the export is missing all native-side hook logs.
            "vpnhide-zygisk:*",
        ).joinToString(" ")
    val (exit, output) = suExec("logcat -d -b all -v threadtime -s $tags 2>/dev/null")
    return if (exit == 0) output else "(logcat failed: exit=$exit)\n$output"
}
