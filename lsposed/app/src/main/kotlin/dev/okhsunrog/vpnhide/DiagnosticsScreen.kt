package dev.okhsunrog.vpnhide

import android.content.Intent
import android.net.ConnectivityManager
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
import androidx.compose.material.icons.filled.Save
import androidx.compose.material.icons.filled.Share
import androidx.compose.material.icons.filled.Stop
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.FileProvider
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
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream

private const val TAG = "VPNHideTest"
private val VPN_PREFIXES = listOf("tun", "wg", "ppp", "tap", "ipsec", "xfrm")

data class CheckResult(
    val name: String,
    val passed: Boolean?,
    val detail: String,
)

internal data class CheckResults(
    val native: List<CheckResult>,
    val java: List<CheckResult>,
) {
    val all get() = native + java
}

internal suspend fun isVpnActive(): Boolean =
    withContext(Dispatchers.IO) {
        val (exitCode, output) = suExec("ls /sys/class/net/ 2>/dev/null")
        if (exitCode != 0) return@withContext false
        val vpnIfaces =
            output
                .lines()
                .map { it.trim() }
                .filter { name ->
                    name.isNotEmpty() && VPN_PREFIXES.any { name.startsWith(it) }
                }
        if (vpnIfaces.isEmpty()) return@withContext false
        vpnIfaces.any { iface ->
            val (_, state) =
                suExec("cat /sys/class/net/$iface/operstate 2>/dev/null")
            state.trim() == "unknown" || state.trim() == "up"
        }
    }

@Composable
fun DiagnosticsScreen(
    selfNeedsRestart: Boolean,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val cm = context.getSystemService(ConnectivityManager::class.java)
    val scope = rememberCoroutineScope()

    val diagState by DiagnosticsCache.state.collectAsState()
    var exporting by remember { mutableStateOf(false) }
    var debugZipFile by remember { mutableStateOf<File?>(null) }
    var kmodTrace by remember { mutableStateOf<String?>(null) }
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

    LaunchedEffect(Unit) {
        kmodTrace =
            withContext(Dispatchers.IO) {
                val (exit, raw) = suExec("cat $KMOD_LOAD_STATUS_FILE 2>/dev/null")
                if (exit != 0 || raw.isBlank()) return@withContext null
                val (_, bootId) = suExec("cat /proc/sys/kernel/random/boot_id 2>/dev/null")
                val (_, dmesg) = suExec("cat $KMOD_LOAD_DMESG_FILE 2>/dev/null")
                buildString {
                    append(raw.trimEnd())
                    append("\ncurrent_boot_id=")
                    append(bootId.trim())
                    if (dmesg.isNotBlank()) {
                        append("\n--- load_dmesg ---\n")
                        append(dmesg.trimEnd())
                    }
                }
            }
    }

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

    val results = (diagState as? DiagnosticsCache.State.Ready)?.results
    val networkBlocked = results?.all?.any { it.detail.startsWith("NETWORK_BLOCKED:") } == true
    val summary =
        results?.let { r ->
            val scored = r.all.filter { it.passed != null }
            val passed = scored.count { it.passed == true }
            String.format(summaryFmt, passed, scored.size)
        }

    Column(
        modifier =
            modifier
                .fillMaxSize()
                .padding(horizontal = 16.dp)
                .verticalScroll(rememberScrollState()),
    ) {
        Spacer(Modifier.height(8.dp))

        // Protection check section — its content depends on cache state,
        // but the bottom debug-tools section always renders below so
        // users can collect logs / toggle verbose logging even when
        // VPN is off or a run is in flight.
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
                    containerColor = Color(0xFF1B5E20).copy(alpha = 0.15f),
                    contentColor = MaterialTheme.colorScheme.onSurface,
                )

                if (networkBlocked) {
                    Spacer(Modifier.height(6.dp))
                    StatusBanner(
                        text = stringResource(R.string.banner_network_blocked),
                        containerColor = MaterialTheme.colorScheme.errorContainer,
                        contentColor = MaterialTheme.colorScheme.onErrorContainer,
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
                    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                        for (check in r.native) {
                            CheckCard(check)
                        }
                    }

                    Spacer(Modifier.height(16.dp))

                    SectionHeader(stringResource(R.string.section_java))
                    Spacer(Modifier.height(6.dp))
                    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                        for (check in r.java) {
                            CheckCard(check)
                        }
                    }
                }
            }
        }

        kmodTrace?.let { trace ->
            Spacer(Modifier.height(16.dp))
            KmodLoadTraceCard(trace)
        }

        Spacer(Modifier.height(16.dp))

        DebugLoggingCard()

        Spacer(Modifier.height(16.dp))

        LogcatRecordCard()

        Spacer(Modifier.height(16.dp))

        // Collect button
        if (debugZipFile == null) {
            Button(
                onClick = {
                    exporting = true
                    scope.launch {
                        debugZipFile = exportDebugZip(cm, context, selfNeedsRestart)
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
            // Save / Share buttons
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                OutlinedButton(
                    onClick = { saveLauncher.launch(debugZipFile!!.name) },
                    modifier = Modifier.weight(1f),
                ) {
                    Icon(
                        Icons.Default.Save,
                        contentDescription = null,
                        modifier = Modifier.size(18.dp),
                    )
                    Spacer(Modifier.width(8.dp))
                    Text(stringResource(R.string.btn_save_debug))
                }
                Button(
                    onClick = {
                        val uri =
                            FileProvider.getUriForFile(
                                context,
                                "${context.packageName}.fileprovider",
                                debugZipFile!!,
                            )
                        val intent =
                            Intent(Intent.ACTION_SEND).apply {
                                type = "application/zip"
                                putExtra(Intent.EXTRA_STREAM, uri)
                                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                            }
                        context.startActivity(Intent.createChooser(intent, null))
                    },
                    modifier = Modifier.weight(1f),
                ) {
                    Icon(
                        Icons.Default.Share,
                        contentDescription = null,
                        modifier = Modifier.size(18.dp),
                    )
                    Spacer(Modifier.width(8.dp))
                    Text(stringResource(R.string.btn_share_debug))
                }
            }
        }

        Spacer(Modifier.height(16.dp))
    }
}

@Composable
private fun DebugLoggingCard() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var enabled by remember { mutableStateOf(VpnHideLog.enabled) }

    Card(
        shape = RoundedCornerShape(8.dp),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant),
        modifier = Modifier.fillMaxWidth(),
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
private fun KmodLoadTraceCard(trace: String) {
    Card(
        shape = RoundedCornerShape(8.dp),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(modifier = Modifier.padding(16.dp)) {
            Text(
                text = stringResource(R.string.diag_kmod_load_trace_title),
                style = MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.Bold,
            )
            Spacer(Modifier.height(4.dp))
            Text(
                text = stringResource(R.string.diag_kmod_load_trace_description),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(8.dp))
            Text(
                text = trace,
                style = MaterialTheme.typography.bodySmall,
                fontFamily = FontFamily.Monospace,
                fontSize = 12.sp,
                color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.9f),
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

    Card(
        shape = RoundedCornerShape(8.dp),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant),
        modifier = Modifier.fillMaxWidth(),
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
                    Button(
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
                    val hasLast = last != null && last.exists()
                    if (hasLast) {
                        Text(
                            text =
                                stringResource(
                                    R.string.logcat_last_recording,
                                    formatElapsed(s.lastDurationMs / 1000),
                                    formatSize(last!!.length()),
                                ),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        Spacer(Modifier.height(8.dp))
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            OutlinedButton(
                                onClick = { saveLauncher.launch(last.name) },
                                modifier = Modifier.weight(1f),
                            ) {
                                Icon(
                                    Icons.Default.Save,
                                    contentDescription = null,
                                    modifier = Modifier.size(18.dp),
                                )
                                Spacer(Modifier.width(8.dp))
                                Text(stringResource(R.string.btn_save))
                            }
                            OutlinedButton(
                                onClick = {
                                    val uri =
                                        FileProvider.getUriForFile(
                                            context,
                                            "${context.packageName}.fileprovider",
                                            last,
                                        )
                                    val intent =
                                        Intent(Intent.ACTION_SEND).apply {
                                            type = "text/plain"
                                            putExtra(Intent.EXTRA_STREAM, uri)
                                            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                                        }
                                    context.startActivity(Intent.createChooser(intent, null))
                                },
                                modifier = Modifier.weight(1f),
                            ) {
                                Icon(
                                    Icons.Default.Share,
                                    contentDescription = null,
                                    modifier = Modifier.size(18.dp),
                                )
                                Spacer(Modifier.width(8.dp))
                                Text(stringResource(R.string.btn_share_debug))
                            }
                        }
                        Spacer(Modifier.height(8.dp))
                    }
                    Button(
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
private fun StatusBanner(
    text: String,
    containerColor: Color,
    contentColor: Color,
) {
    Card(
        shape = RoundedCornerShape(8.dp),
        colors = CardDefaults.cardColors(containerColor = containerColor),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text(
            text = text,
            style = MaterialTheme.typography.bodyMedium,
            color = contentColor,
            modifier = Modifier.padding(12.dp),
        )
    }
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
private fun CheckCard(r: CheckResult) {
    val darkTheme = isSystemInDarkTheme()
    val actualColor =
        if (darkTheme) {
            when (r.passed) {
                true -> Color(0xFF1B5E20).copy(alpha = 0.3f)
                false -> Color(0xFFB71C1C).copy(alpha = 0.3f)
                null -> MaterialTheme.colorScheme.surfaceVariant
            }
        } else {
            when (r.passed) {
                true -> Color(0xFFE8F5E9)
                false -> Color(0xFFFFEBEE)
                null -> MaterialTheme.colorScheme.surfaceVariant
            }
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
            true -> Color(0xFF2E7D32)
            false -> Color(0xFFC62828)
            null -> MaterialTheme.colorScheme.onSurfaceVariant
        }

    Card(
        shape = RoundedCornerShape(8.dp),
        colors = CardDefaults.cardColors(containerColor = actualColor),
        modifier = Modifier.fillMaxWidth(),
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

internal fun runAllChecks(
    cm: ConnectivityManager,
    context: android.content.Context,
): CheckResults {
    VpnHideLog.i(TAG, "========================================")
    VpnHideLog.i(TAG, "=== VPNHide — starting all checks ===")
    VpnHideLog.i(TAG, "========================================")

    val res = context.resources

    val native =
        listOf(
            nativeCheck(res.getString(R.string.check_ioctl_flags)) { NativeChecks.checkIoctlSiocgifflags() },
            nativeCheck(res.getString(R.string.check_ioctl_mtu)) { NativeChecks.checkIoctlSiocgifmtu() },
            nativeCheck(res.getString(R.string.check_ioctl_conf)) { NativeChecks.checkIoctlSiocgifconf() },
            nativeCheck(res.getString(R.string.check_getifaddrs)) { NativeChecks.checkGetifaddrs() },
            nativeCheck(res.getString(R.string.check_netlink_getlink)) { NativeChecks.checkNetlinkGetlink() },
            nativeCheck(res.getString(R.string.check_netlink_getlink_recv)) { NativeChecks.checkNetlinkGetlinkRecv() },
            nativeCheck(res.getString(R.string.check_netlink_getroute)) { NativeChecks.checkNetlinkGetroute() },
            nativeCheck(res.getString(R.string.check_proc_route)) { NativeChecks.checkProcNetRoute() },
            nativeCheck(res.getString(R.string.check_proc_ipv6_route)) { NativeChecks.checkProcNetIpv6Route() },
            nativeCheck(res.getString(R.string.check_proc_if_inet6)) { NativeChecks.checkProcNetIfInet6() },
            nativeCheck(res.getString(R.string.check_proc_tcp)) { NativeChecks.checkProcNetTcp() },
            nativeCheck(res.getString(R.string.check_proc_tcp6)) { NativeChecks.checkProcNetTcp6() },
            nativeCheck(res.getString(R.string.check_proc_udp)) { NativeChecks.checkProcNetUdp() },
            nativeCheck(res.getString(R.string.check_proc_udp6)) { NativeChecks.checkProcNetUdp6() },
            nativeCheck(res.getString(R.string.check_proc_dev)) { NativeChecks.checkProcNetDev() },
            nativeCheck(res.getString(R.string.check_proc_fib_trie)) { NativeChecks.checkProcNetFibTrie() },
            nativeCheck(res.getString(R.string.check_sys_class_net)) { NativeChecks.checkSysClassNet() },
            checkNetworkInterfaceEnum(res.getString(R.string.check_net_iface_enum)),
            checkProcNetRouteJava(res.getString(R.string.check_proc_route_java)),
        )

    val java =
        listOf(
            checkHasTransportVpn(cm, res.getString(R.string.check_has_transport_vpn)),
            checkHasCapabilityNotVpn(cm, res.getString(R.string.check_has_capability_not_vpn)),
            checkTransportInfo(cm, res.getString(R.string.check_transport_info)),
            checkAllNetworksVpn(cm, res.getString(R.string.check_all_networks_vpn)),
            checkActiveNetworkVpn(cm, res.getString(R.string.check_active_network_vpn)),
            checkLinkPropertiesIfname(cm, res.getString(R.string.check_link_properties)),
            checkLinkPropertiesRoutes(cm, res.getString(R.string.check_link_properties_routes)),
            checkProxyHost(res.getString(R.string.check_proxy_host)),
        )

    val all = native + java
    val scored = all.filter { it.passed != null }
    val passed = scored.count { it.passed == true }
    VpnHideLog.i(TAG, "=== SUMMARY: $passed/${scored.size} passed ===")

    return CheckResults(native = native, java = java)
}

private fun nativeCheck(
    name: String,
    block: () -> String,
): CheckResult =
    try {
        val raw = block()
        val passed =
            when {
                raw.startsWith("PASS") -> true
                raw.startsWith("NETWORK_BLOCKED:") -> null
                else -> false
            }
        VpnHideLog.i(TAG, "[$name] $raw")
        CheckResult(name, passed, raw)
    } catch (e: Exception) {
        val detail = "FAIL: exception: ${e.message}"
        Log.e(TAG, "[$name] $detail", e)
        CheckResult(name, false, detail)
    }

// ==========================================================================
//  Java API checks
// ==========================================================================

private fun checkHasTransportVpn(
    cm: ConnectivityManager,
    name: String,
): CheckResult {
    val net = cm.activeNetwork ?: return CheckResult(name, true, "PASS: no active network")
    val caps = cm.getNetworkCapabilities(net) ?: return CheckResult(name, true, "PASS: no capabilities")
    val hasVpn = caps.hasTransport(NetworkCapabilities.TRANSPORT_VPN)
    val hasWifi = caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)
    val hasCellular = caps.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)
    val detail =
        if (!hasVpn) {
            "PASS: hasTransport(VPN)=false, WIFI=$hasWifi, CELLULAR=$hasCellular"
        } else {
            "FAIL: hasTransport(VPN)=true, WIFI=$hasWifi, CELLULAR=$hasCellular"
        }
    return CheckResult(name, !hasVpn, detail)
}

private fun checkHasCapabilityNotVpn(
    cm: ConnectivityManager,
    name: String,
): CheckResult {
    val net = cm.activeNetwork ?: return CheckResult(name, true, "PASS: no active network")
    val caps = cm.getNetworkCapabilities(net) ?: return CheckResult(name, true, "PASS: no capabilities")
    val notVpn = caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
    val detail = if (notVpn) "PASS: NOT_VPN capability present" else "FAIL: NOT_VPN capability MISSING"
    return CheckResult(name, notVpn, detail)
}

private fun checkTransportInfo(
    cm: ConnectivityManager,
    name: String,
): CheckResult {
    val net = cm.activeNetwork ?: return CheckResult(name, true, "PASS: no active network")
    val caps = cm.getNetworkCapabilities(net) ?: return CheckResult(name, true, "PASS: no capabilities")
    val info = caps.transportInfo
    val className = info?.javaClass?.name ?: "null"
    val isVpn = className.contains("VpnTransportInfo")
    val detail = if (!isVpn) "PASS: transportInfo=$className" else "FAIL: VpnTransportInfo: $info"
    return CheckResult(name, !isVpn, detail)
}

private fun checkNetworkInterfaceEnum(name: String): CheckResult =
    try {
        val ifaces =
            NetworkInterface.getNetworkInterfaces()
                ?: return CheckResult(name, true, "PASS: returned null")
        val allNames = mutableListOf<String>()
        val vpnNames = mutableListOf<String>()
        for (iface in ifaces) {
            allNames.add(iface.name)
            if (VPN_PREFIXES.any { iface.name.startsWith(it) }) vpnNames.add(iface.name)
        }
        val detail =
            if (vpnNames.isEmpty()) {
                "PASS: ${allNames.size} ifaces [${allNames.joinToString()}], no VPN"
            } else {
                "FAIL: VPN [${vpnNames.joinToString()}] in [${allNames.joinToString()}]"
            }
        CheckResult(name, vpnNames.isEmpty(), detail)
    } catch (e: Exception) {
        CheckResult(name, false, "FAIL: ${e.message}")
    }

@Suppress("DEPRECATION")
private fun checkAllNetworksVpn(
    cm: ConnectivityManager,
    name: String,
): CheckResult {
    val networks = cm.allNetworks
    if (networks.isEmpty()) return CheckResult(name, true, "PASS: no networks")
    val vpnNetworks =
        networks.filter { net ->
            cm.getNetworkCapabilities(net)?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) == true
        }
    val detail =
        if (vpnNetworks.isEmpty()) {
            "PASS: ${networks.size} networks, none have TRANSPORT_VPN"
        } else {
            "FAIL: ${vpnNetworks.size} network(s) with TRANSPORT_VPN"
        }
    return CheckResult(name, vpnNetworks.isEmpty(), detail)
}

private fun checkActiveNetworkVpn(
    cm: ConnectivityManager,
    name: String,
): CheckResult {
    val net = cm.activeNetwork ?: return CheckResult(name, true, "PASS: no active network")
    val caps = cm.getNetworkCapabilities(net) ?: return CheckResult(name, true, "PASS: no capabilities")
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
            "PASS: transports=[${transports.joinToString()}], no VPN"
        } else {
            "FAIL: transports include VPN: [${transports.joinToString()}]"
        }
    return CheckResult(name, !hasVpn, detail)
}

private fun checkLinkPropertiesIfname(
    cm: ConnectivityManager,
    name: String,
): CheckResult {
    val net = cm.activeNetwork ?: return CheckResult(name, true, "PASS: no active network")
    val lp = cm.getLinkProperties(net) ?: return CheckResult(name, true, "PASS: no link properties")
    val ifname = lp.interfaceName ?: "(null)"
    val routes = lp.routes.map { "${it.destination} via ${it.gateway} dev ${it.`interface`}" }
    val dns = lp.dnsServers.map { it.hostAddress ?: "?" }
    val isVpn = VPN_PREFIXES.any { ifname.startsWith(it) }
    val detail =
        if (!isVpn) {
            "PASS: ifname=$ifname, ${routes.size} routes, dns=[${dns.joinToString()}]"
        } else {
            "FAIL: ifname=$ifname is a VPN interface"
        }
    return CheckResult(name, !isVpn, detail)
}

private fun checkLinkPropertiesRoutes(
    cm: ConnectivityManager,
    name: String,
): CheckResult {
    val net = cm.activeNetwork ?: return CheckResult(name, true, "PASS: no active network")
    val lp = cm.getLinkProperties(net) ?: return CheckResult(name, true, "PASS: no link properties")
    val routes = lp.routes
    val vpnRoutes =
        routes.filter { route ->
            val iface = route.`interface` ?: return@filter false
            VPN_PREFIXES.any { iface.startsWith(it) }
        }
    val detail =
        if (vpnRoutes.isEmpty()) {
            "PASS: ${routes.size} routes, none via VPN interfaces"
        } else {
            "FAIL: ${vpnRoutes.size} route(s) via VPN"
        }
    return CheckResult(name, vpnRoutes.isEmpty(), detail)
}

private fun checkProxyHost(name: String): CheckResult {
    val httpHost = System.getProperty("http.proxyHost")
    val socksHost = System.getProperty("socksProxyHost")
    val hasProxy = !httpHost.isNullOrEmpty() || !socksHost.isNullOrEmpty()
    val detail =
        if (!hasProxy) {
            "PASS: no proxy (http=$httpHost, socks=$socksHost)"
        } else {
            val httpPort = System.getProperty("http.proxyPort")
            val socksPort = System.getProperty("socksProxyPort")
            "FAIL: proxy found — http=$httpHost:$httpPort, socks=$socksHost:$socksPort"
        }
    return CheckResult(name, !hasProxy, detail)
}

private fun checkProcNetRouteJava(name: String): CheckResult =
    try {
        val allLines = mutableListOf<String>()
        val vpnLines = mutableListOf<String>()
        BufferedReader(InputStreamReader(java.io.FileInputStream("/proc/net/route"))).use { br ->
            var line: String?
            while (br.readLine().also { line = it } != null) {
                allLines.add(line!!)
                if (VPN_PREFIXES.any { line!!.startsWith(it) }) vpnLines.add(line!!.take(60))
            }
        }
        val detail =
            if (vpnLines.isEmpty()) {
                "PASS: ${allLines.size} lines, no VPN entries"
            } else {
                "FAIL: ${vpnLines.size} VPN lines:\n${vpnLines.joinToString("\n") { "  $it" }}"
            }
        CheckResult(name, vpnLines.isEmpty(), detail)
    } catch (e: Exception) {
        val msg = e.message ?: ""
        if (msg.contains("EACCES") || msg.contains("Permission denied")) {
            CheckResult(name, true, "PASS: access denied by SELinux")
        } else {
            CheckResult(name, false, "FAIL: ${e.message}")
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
        // Force-enable app/lsposed/zygisk debug logging while the capture
        // runs so the dump contains VpnHide-tagged lines even when the
        // user's persistent toggle is OFF (the default). We restore to
        // whatever the SharedPreferences say at the end — if the user
        // happens to flip the UI toggle mid-capture, we honor their
        // final choice instead of blindly rolling back.
        val loggingWasForced = !VpnHideLog.enabled
        if (loggingWasForced) applyDebugLoggingRuntime(true)
        try {
            // 1. Enable kmod debug logging
            suExec("echo 1 > /proc/vpnhide_debug 2>/dev/null")

            // 2. Clear dmesg so we only capture fresh output
            suExec("dmesg -c > /dev/null 2>&1")

            // 3. Run all diagnostic checks (this triggers kmod hooks)
            val checkResults = runAllChecks(cm, context)

            // 4. Capture dmesg right after checks
            val (_, dmesg) = suExec("dmesg 2>/dev/null")

            // 5. Disable kmod debug logging
            suExec("echo 0 > /proc/vpnhide_debug 2>/dev/null")

            // 6. Collect additional info
            val files = mutableMapOf<String, String>()

            // dmesg (vpnhide lines only + full for context)
            val vpnhideLines = dmesg.lines().filter { it.contains("vpnhide") }
            files["dmesg_vpnhide.txt"] = vpnhideLines.joinToString("\n")
            files["dmesg_full.txt"] = dmesg

            // Diagnostics results
            val diagText =
                buildString {
                    val scored = checkResults.all.filter { it.passed != null }
                    val passed = scored.count { it.passed == true }
                    appendLine("=== Diagnostics: $passed/${scored.size} passed ===")
                    appendLine()
                    appendLine("--- Native level ---")
                    for (c in checkResults.native) {
                        val badge =
                            when (c.passed) {
                                true -> "PASS"
                                false -> "FAIL"
                                null -> "INFO"
                            }
                        appendLine("[$badge] ${c.name}")
                        appendLine("  ${c.detail}")
                    }
                    appendLine()
                    appendLine("--- Java API level ---")
                    for (c in checkResults.java) {
                        val badge =
                            when (c.passed) {
                                true -> "PASS"
                                false -> "FAIL"
                                null -> "INFO"
                            }
                        appendLine("[$badge] ${c.name}")
                        appendLine("  ${c.detail}")
                    }
                }
            files["diagnostics.txt"] = diagText

            // Device info
            val (_, kernelVersion) = suExec("uname -r 2>/dev/null")
            val (_, procVersion) = suExec("cat /proc/version 2>/dev/null")
            val (_, selinuxMode) = suExec("getenforce 2>/dev/null")
            val deviceInfo =
                buildString {
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
            files["device_info.txt"] = deviceInfo

            // Module info
            val moduleInfo =
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
                        listOf(
                            "dev_ioctl",
                            "dev_ifconf",
                            "rtnl_fill_ifinfo",
                            "inet6_fill_ifaddr",
                            "inet_fill_ifaddr",
                            "fib_route_seq_show",
                        )
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
            files["modules.txt"] = moduleInfo

            // Target UIDs
            val (_, procTargets) = suExec("cat /proc/vpnhide_targets 2>/dev/null")
            val (_, kmodTargets) = suExec("cat $KMOD_TARGETS 2>/dev/null")
            val (_, zygiskTargets) = suExec("cat $ZYGISK_TARGETS 2>/dev/null")
            val (_, lsposedTargets) = suExec("cat $LSPOSED_TARGETS 2>/dev/null")
            val targetsText =
                buildString {
                    appendLine("=== /proc/vpnhide_targets (live UIDs) ===")
                    appendLine(procTargets.ifEmpty { "(empty)" })
                    appendLine()
                    appendLine("=== kmod targets ===")
                    appendLine(kmodTargets.ifEmpty { "(empty)" })
                    appendLine()
                    appendLine("=== zygisk targets ===")
                    appendLine(zygiskTargets.ifEmpty { "(empty)" })
                    appendLine()
                    appendLine("=== lsposed targets ===")
                    appendLine(lsposedTargets.ifEmpty { "(empty)" })
                }
            files["targets.txt"] = targetsText

            // Network interfaces (via su, unfiltered)
            val ifacesText =
                buildString {
                    appendLine("=== ip -d addr ===")
                    val (_, ipAddr) = suExec("ip -d addr 2>/dev/null")
                    appendLine(ipAddr.ifEmpty { "(not available)" })
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
                    val (_, routes) = suExec("ip route show table all 2>/dev/null")
                    appendLine(routes.ifEmpty { "(not available)" })
                    appendLine()
                    appendLine("=== ip rule ===")
                    val (_, rules) = suExec("ip rule 2>/dev/null")
                    appendLine(rules.ifEmpty { "(not available)" })
                }
            files["interfaces.txt"] = ifacesText

            // /proc/net files
            val procFiles = listOf("route", "ipv6_route", "if_inet6", "tcp", "tcp6", "udp", "udp6", "dev")
            val procNet =
                buildString {
                    for (pf in procFiles) {
                        appendLine("=== /proc/net/$pf ===")
                        val (_, content) = suExec("cat /proc/net/$pf 2>/dev/null")
                        appendLine(content.ifEmpty { "(not available)" })
                        appendLine()
                    }
                }
            files["proc_net.txt"] = procNet

            // Logcat (app-level logs)
            // Read logcat directly (no su) — app can read its own logs
            val logcat =
                try {
                    val proc =
                        Runtime.getRuntime().exec(
                            arrayOf(
                                "logcat",
                                "-d",
                                "-s",
                                "VPNHideTest:*",
                                "VpnHide:*",
                                "VpnHide-Dashboard:*",
                            ),
                        )
                    val output = proc.inputStream.bufferedReader().readText()
                    proc.waitFor()
                    proc.destroy()
                    output
                } catch (e: Exception) {
                    "(logcat failed: ${e.message})"
                }
            files["logcat.txt"] = logcat.ifEmpty { "(no logcat entries)" }

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
