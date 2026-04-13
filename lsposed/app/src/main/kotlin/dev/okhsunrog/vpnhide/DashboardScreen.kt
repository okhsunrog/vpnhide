package dev.okhsunrog.vpnhide

import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.util.Log
import dev.okhsunrog.vpnhide.BuildConfig
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

// ── Domain types — invalid states are unrepresentable ────────────────────

sealed interface ModuleState {
    data object NotInstalled : ModuleState
    data class Installed(
        val version: String?,
        val active: Boolean,
        val targetCount: Int,
    ) : ModuleState
}

sealed interface LsposedState {
    data object NotDetected : LsposedState
    data class NeedsReboot(val version: String?) : LsposedState
    data class Active(val version: String?, val targetCount: Int) : LsposedState
}

sealed interface ProtectionCheck {
    data object NoVpn : ProtectionCheck
    data object NeedsRestart : ProtectionCheck
    data class Checked(
        val native: NativeResult,
        val java: JavaResult,
    ) : ProtectionCheck
}

sealed interface NativeResult {
    data object Ok : NativeResult
    data class Fail(val passed: Int, val failed: Int) : NativeResult
    data object NoModule : NativeResult
}

sealed interface JavaResult {
    data object Ok : JavaResult
    data class Fail(val failedChecks: Int) : JavaResult
    data object HooksInactive : JavaResult
}

private data class DashboardState(
    val kmod: ModuleState,
    val zygisk: ModuleState,
    val lsposed: LsposedState,
    val protection: ProtectionCheck,
    val issues: List<String>,
)

// ── Screen ───────────────────────────────────────────────────────────────

@Composable
fun DashboardScreen(modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val cm = context.getSystemService(ConnectivityManager::class.java)

    var state by remember { mutableStateOf<DashboardState?>(null) }

    LaunchedEffect(Unit) {
        state = withContext(Dispatchers.IO) { loadDashboardState(cm, context) }
    }

    Column(
        modifier = modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(horizontal = 16.dp),
    ) {
        Spacer(Modifier.height(12.dp))

        val s = state
        if (s == null) {
            Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
                CircularProgressIndicator()
            }
            return@Column
        }

        // Module status cards
        Text(
            text = stringResource(R.string.dashboard_modules),
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.Bold,
        )
        Spacer(Modifier.height(8.dp))

        LsposedCard(s.lsposed)
        Spacer(Modifier.height(8.dp))
        ModuleCard(stringResource(R.string.dashboard_kmod), s.kmod)
        Spacer(Modifier.height(8.dp))
        ModuleCard(stringResource(R.string.dashboard_zygisk), s.zygisk)

        // Protection status
        Spacer(Modifier.height(20.dp))
        Text(
            text = stringResource(R.string.dashboard_protection),
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.Bold,
        )
        Spacer(Modifier.height(8.dp))

        when (val p = s.protection) {
            is ProtectionCheck.NoVpn -> {
                StatusBanner(
                    text = stringResource(R.string.dashboard_no_vpn),
                    containerColor = MaterialTheme.colorScheme.surfaceVariant,
                    contentColor = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            is ProtectionCheck.NeedsRestart -> {
                StatusBanner(
                    text = stringResource(R.string.dashboard_needs_restart),
                    containerColor = MaterialTheme.colorScheme.tertiaryContainer,
                    contentColor = MaterialTheme.colorScheme.onTertiaryContainer,
                )
            }
            is ProtectionCheck.Checked -> {
                NativeProtectionCard(p.native)
                Spacer(Modifier.height(8.dp))
                JavaProtectionCard(p.java)
            }
        }

        // Issues
        if (s.issues.isNotEmpty()) {
            Spacer(Modifier.height(20.dp))
            Text(
                text = stringResource(R.string.dashboard_issues, s.issues.size),
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Bold,
                color = MaterialTheme.colorScheme.error,
            )
            Spacer(Modifier.height(8.dp))
            for (issue in s.issues) {
                StatusBanner(
                    text = issue,
                    containerColor = MaterialTheme.colorScheme.errorContainer,
                    contentColor = MaterialTheme.colorScheme.onErrorContainer,
                )
                Spacer(Modifier.height(6.dp))
            }
        }

        Spacer(Modifier.height(16.dp))
    }
}

// ── UI Components ────────────────────────────────────────────────────────

@Composable
private fun ModuleCard(name: String, state: ModuleState) {
    val darkTheme = isSystemInDarkTheme()
    when (state) {
        is ModuleState.NotInstalled -> {
            ModuleCardShell(
                name = name,
                version = null,
                subtitle = stringResource(R.string.dashboard_not_installed),
                dotColor = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.4f),
                containerColor = MaterialTheme.colorScheme.surfaceVariant,
            )
        }
        is ModuleState.Installed -> {
            val active = state.active
            ModuleCardShell(
                name = name,
                version = state.version,
                subtitle = if (active) {
                    stringResource(R.string.dashboard_active_targets, state.targetCount)
                } else {
                    stringResource(R.string.dashboard_installed_inactive)
                },
                dotColor = if (active) Color(0xFF4CAF50) else Color(0xFFFF9800),
                containerColor = if (active) {
                    if (darkTheme) Color(0xFF1B5E20).copy(alpha = 0.3f) else Color(0xFFE8F5E9)
                } else {
                    if (darkTheme) Color(0xFFE65100).copy(alpha = 0.2f) else Color(0xFFFFF3E0)
                },
            )
        }
    }
}

@Composable
private fun LsposedCard(state: LsposedState) {
    val darkTheme = isSystemInDarkTheme()
    when (state) {
        is LsposedState.NotDetected -> {
            ModuleCardShell(
                name = "LSPosed",
                version = null,
                subtitle = stringResource(R.string.dashboard_not_installed),
                dotColor = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.4f),
                containerColor = MaterialTheme.colorScheme.surfaceVariant,
            )
        }
        is LsposedState.NeedsReboot -> {
            ModuleCardShell(
                name = "LSPosed",
                version = state.version,
                subtitle = stringResource(R.string.dashboard_reboot_needed),
                dotColor = Color(0xFFFF9800),
                containerColor = if (darkTheme) Color(0xFFE65100).copy(alpha = 0.2f) else Color(0xFFFFF3E0),
            )
        }
        is LsposedState.Active -> {
            val subtitle = stringResource(R.string.dashboard_active_targets, state.targetCount) +
                if (state.version != null) {
                    "\n" + stringResource(R.string.dashboard_running_version, state.version)
                } else ""
            ModuleCardShell(
                name = "LSPosed",
                version = state.version,
                subtitle = subtitle,
                dotColor = Color(0xFF4CAF50),
                containerColor = if (darkTheme) Color(0xFF1B5E20).copy(alpha = 0.3f) else Color(0xFFE8F5E9),
            )
        }
    }
}

@Composable
private fun ModuleCardShell(
    name: String,
    version: String?,
    subtitle: String,
    dotColor: Color,
    containerColor: Color,
) {
    Card(
        shape = RoundedCornerShape(12.dp),
        colors = CardDefaults.cardColors(containerColor = containerColor),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Row(
            modifier = Modifier.padding(16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Surface(
                shape = RoundedCornerShape(50),
                color = dotColor,
                modifier = Modifier.size(12.dp),
            ) {}
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        text = name,
                        style = MaterialTheme.typography.titleSmall,
                        fontWeight = FontWeight.Bold,
                    )
                    if (version != null) {
                        Spacer(Modifier.width(8.dp))
                        Text(
                            text = version,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
                Text(
                    text = subtitle,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.7f),
                )
            }
        }
    }
}

@Composable
private fun NativeProtectionCard(result: NativeResult) {
    val darkTheme = isSystemInDarkTheme()
    val (containerColor, statusText, statusColor) = when (result) {
        is NativeResult.Ok -> Triple(
            if (darkTheme) Color(0xFF1B5E20).copy(alpha = 0.3f) else Color(0xFFE8F5E9),
            stringResource(R.string.dashboard_protection_ok),
            Color(0xFF4CAF50),
        )
        is NativeResult.Fail -> {
            val text = if (result.passed > 0) {
                stringResource(R.string.dashboard_protection_partial)
            } else {
                stringResource(R.string.dashboard_protection_fail)
            }
            val color = if (result.passed > 0) Color(0xFFFF9800) else Color(0xFFC62828)
            val bg = if (result.passed > 0) {
                if (darkTheme) Color(0xFFE65100).copy(alpha = 0.2f) else Color(0xFFFFF3E0)
            } else {
                if (darkTheme) Color(0xFFB71C1C).copy(alpha = 0.3f) else Color(0xFFFFEBEE)
            }
            Triple(bg, text, color)
        }
        is NativeResult.NoModule -> Triple(
            MaterialTheme.colorScheme.surfaceVariant,
            stringResource(R.string.dashboard_protection_no_module),
            MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
    ProtectionCardShell(
        label = stringResource(R.string.dashboard_native_protection),
        statusText = statusText,
        statusColor = statusColor,
        containerColor = containerColor,
    )
}

@Composable
private fun JavaProtectionCard(result: JavaResult) {
    val darkTheme = isSystemInDarkTheme()
    val (containerColor, statusText, statusColor) = when (result) {
        is JavaResult.Ok -> Triple(
            if (darkTheme) Color(0xFF1B5E20).copy(alpha = 0.3f) else Color(0xFFE8F5E9),
            stringResource(R.string.dashboard_protection_ok),
            Color(0xFF4CAF50),
        )
        is JavaResult.Fail -> Triple(
            if (darkTheme) Color(0xFFB71C1C).copy(alpha = 0.3f) else Color(0xFFFFEBEE),
            stringResource(R.string.dashboard_protection_fail),
            Color(0xFFC62828),
        )
        is JavaResult.HooksInactive -> Triple(
            MaterialTheme.colorScheme.surfaceVariant,
            stringResource(R.string.dashboard_protection_hooks_inactive),
            MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
    ProtectionCardShell(
        label = stringResource(R.string.dashboard_java_protection),
        statusText = statusText,
        statusColor = statusColor,
        containerColor = containerColor,
    )
}

@Composable
private fun ProtectionCardShell(
    label: String,
    statusText: String,
    statusColor: Color,
    containerColor: Color,
) {
    Card(
        shape = RoundedCornerShape(12.dp),
        colors = CardDefaults.cardColors(containerColor = containerColor),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Row(
            modifier = Modifier.padding(16.dp).fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = label,
                style = MaterialTheme.typography.bodyLarge,
                fontWeight = FontWeight.Medium,
            )
            Text(
                text = statusText,
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.Bold,
                color = statusColor,
            )
        }
    }
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

// ── Data loading ─────────────────────────────────────────────────────────

private const val TAG = "VpnHide-Dashboard"

/**
 * Ensure the VPN Hide app itself is in all 3 target lists.
 * Returns true if self had to be added (= hooks not yet applied to this process).
 */
private fun ensureSelfInTargets(selfPkg: String): Boolean {
    var added = false

    fun addToTargetsIfMissing(path: String, dirCheck: String?) {
        if (dirCheck != null) {
            val (_, exists) = suExec("[ -d $dirCheck ] && echo 1 || echo 0")
            if (exists.trim() != "1") {
                Log.d(TAG, "ensureSelfInTargets: $dirCheck not found, skipping $path")
                return
            }
        }
        val (_, raw) = suExec("cat $path 2>/dev/null || true")
        val existing = raw.lines().map { it.trim() }.filter { it.isNotEmpty() && !it.startsWith("#") }
        if (selfPkg in existing) {
            Log.d(TAG, "ensureSelfInTargets: $selfPkg already in $path")
            return
        }
        val newBody = "# Managed by VPN Hide app\n" +
            (existing + selfPkg).sorted().joinToString("\n") + "\n"
        val b64 = android.util.Base64.encodeToString(newBody.toByteArray(), android.util.Base64.NO_WRAP)
        suExec("echo '$b64' | base64 -d > $path && chmod 644 $path")
        Log.i(TAG, "ensureSelfInTargets: added $selfPkg to $path")
        added = true
    }

    addToTargetsIfMissing(KMOD_TARGETS, "/data/adb/vpnhide_kmod")
    addToTargetsIfMissing(ZYGISK_TARGETS, "/data/adb/vpnhide_zygisk")
    suExec("mkdir -p /data/adb/vpnhide_lsposed")
    addToTargetsIfMissing(LSPOSED_TARGETS, null)

    // Resolve UIDs for kmod + lsposed so hooks pick us up immediately
    val uidCmd = buildString {
        append("ALL_PKGS=\"\$(pm list packages -U 2>/dev/null)\"")
        append("; SELF_UID=\$(echo \"\$ALL_PKGS\" | grep '^package:$selfPkg ' | sed 's/.*uid://')")
        append("; if [ -f $PROC_TARGETS ] && [ -n \"\$SELF_UID\" ]; then")
        append("   EXISTING=\$(cat $PROC_TARGETS 2>/dev/null)")
        append(";  echo \"\$EXISTING\" | grep -q \"^\$SELF_UID\$\" || echo \"\$SELF_UID\" >> $PROC_TARGETS")
        append("; fi")
        append("; if [ -n \"\$SELF_UID\" ]; then")
        append("   EXISTING2=\$(cat $SS_UIDS_FILE 2>/dev/null)")
        append(";  echo \"\$EXISTING2\" | grep -q \"^\$SELF_UID\$\" || { echo \"\$SELF_UID\" >> $SS_UIDS_FILE; chmod 644 $SS_UIDS_FILE; chcon u:object_r:system_data_file:s0 $SS_UIDS_FILE 2>/dev/null; }")
        append("; fi")
    }
    suExec(uidCmd)
    Log.d(TAG, "ensureSelfInTargets: done, added=$added")
    return added
}

private fun loadDashboardState(
    cm: ConnectivityManager,
    context: android.content.Context,
): DashboardState {
    val issues = mutableListOf<String>()
    val res = context.resources
    val selfPkg = context.packageName

    Log.i(TAG, "=== Loading dashboard state ===")

    // ── Ensure self in targets ──
    val selfJustAdded = ensureSelfInTargets(selfPkg)

    // ── Module detection ──
    fun parseModuleProp(dir: String): Pair<Boolean, String?> {
        val (exitCode, out) = suExec("cat $dir/module.prop 2>/dev/null")
        if (exitCode != 0 || out.isBlank()) return false to null
        val version = out.lines().firstOrNull { it.startsWith("version=") }?.removePrefix("version=")
        return true to version
    }

    fun countTargets(path: String): Int {
        val (_, out) = suExec("cat $path 2>/dev/null || true")
        return out.lines().count { line ->
            val trimmed = line.trim()
            trimmed.isNotEmpty() && !trimmed.startsWith("#") && trimmed != selfPkg
        }
    }

    // kmod
    val (kmodInstalled, kmodVersion) = parseModuleProp(KMOD_MODULE_DIR)
    val (_, procExists) = suExec("[ -f $PROC_TARGETS ] && echo 1 || echo 0")
    val kmodActive = kmodInstalled && procExists.trim() == "1"
    val kmodTargetCount = if (kmodInstalled) countTargets(KMOD_TARGETS) else 0
    val kmod: ModuleState = if (kmodInstalled) {
        ModuleState.Installed(kmodVersion, kmodActive, kmodTargetCount)
    } else {
        ModuleState.NotInstalled
    }
    Log.i(TAG, "kmod: $kmod")

    // zygisk
    val (zygiskInstalled, zygiskVersion) = parseModuleProp(ZYGISK_MODULE_DIR)
    val zygiskTargetCount = if (zygiskInstalled) countTargets(ZYGISK_TARGETS) else 0
    val zygisk: ModuleState = if (zygiskInstalled) {
        ModuleState.Installed(zygiskVersion, true, zygiskTargetCount)
    } else {
        ModuleState.NotInstalled
    }
    Log.i(TAG, "zygisk: $zygisk")

    // lsposed hook status
    val (_, hookStatusRaw) = suExec("cat ${HookEntry.HOOK_STATUS_FILE} 2>/dev/null || true")
    val hookProps = hookStatusRaw.lines().mapNotNull {
        val parts = it.split("=", limit = 2)
        if (parts.size == 2) parts[0] to parts[1] else null
    }.toMap()
    val hookVersion = hookProps["version"]
    val hookBootId = hookProps["boot_id"]
    val (_, currentBootId) = suExec("cat /proc/sys/kernel/random/boot_id 2>/dev/null")
    val hooksActiveThisBoot = hookBootId != null && hookBootId == currentBootId.trim()
    val lsposedTargetCount = countTargets(LSPOSED_TARGETS)

    val lsposed: LsposedState = when {
        hookVersion == null -> LsposedState.NotDetected
        !hooksActiveThisBoot -> LsposedState.NeedsReboot(hookVersion)
        else -> LsposedState.Active(hookVersion, lsposedTargetCount)
    }
    Log.i(TAG, "lsposed: $lsposed (hookBootId=$hookBootId currentBootId=${currentBootId.trim()})")

    // ── Issues ──
    val hasNative = kmod is ModuleState.Installed || zygisk is ModuleState.Installed
    if (!hasNative) {
        issues += res.getString(R.string.dashboard_issue_no_native)
    }
    if (lsposed is LsposedState.NeedsReboot) {
        issues += res.getString(R.string.dashboard_issue_reboot)
    }
    if (lsposed is LsposedState.Active) {
        if (lsposedTargetCount == 0) {
            issues += res.getString(R.string.dashboard_issue_no_targets)
        }
        val appVersion = BuildConfig.VERSION_NAME
        val runningVersion = lsposed.version
        if (runningVersion != null && runningVersion != appVersion) {
            Log.w(TAG, "version mismatch: running=$runningVersion app=$appVersion")
            issues += res.getString(R.string.dashboard_issue_version_mismatch, runningVersion, appVersion)
        }
    }

    // ── Protection checks ──
    val vpnActive = isVpnActiveSync()
    Log.i(TAG, "vpnActive=$vpnActive selfJustAdded=$selfJustAdded")

    val protection: ProtectionCheck = when {
        !vpnActive -> ProtectionCheck.NoVpn
        selfJustAdded -> ProtectionCheck.NeedsRestart
        else -> {
            val native = if (hasNative) {
                runNativeProtectionCheck()
            } else {
                NativeResult.NoModule
            }
            Log.i(TAG, "nativeResult=$native")

            val java = if (lsposed is LsposedState.Active) {
                runJavaProtectionCheck(cm)
            } else {
                JavaResult.HooksInactive
            }
            Log.i(TAG, "javaResult=$java")

            ProtectionCheck.Checked(native, java)
        }
    }

    Log.i(TAG, "protection=$protection issues=$issues")
    Log.i(TAG, "=== Dashboard state loaded ===")

    return DashboardState(
        kmod = kmod,
        zygisk = zygisk,
        lsposed = lsposed,
        protection = protection,
        issues = issues,
    )
}

private fun isVpnActiveSync(): Boolean {
    val (exitCode, output) = suExec("ls /sys/class/net/ 2>/dev/null")
    if (exitCode != 0) return false
    val vpnPrefixes = listOf("tun", "wg", "ppp", "tap", "ipsec", "xfrm")
    val vpnIfaces = output.lines().map { it.trim() }.filter { name ->
        name.isNotEmpty() && vpnPrefixes.any { name.startsWith(it) }
    }
    if (vpnIfaces.isEmpty()) {
        Log.d(TAG, "isVpnActive: no VPN interfaces found")
        return false
    }
    return vpnIfaces.any { iface ->
        val (_, state) = suExec("cat /sys/class/net/$iface/operstate 2>/dev/null")
        val up = state.trim() == "unknown" || state.trim() == "up"
        Log.d(TAG, "isVpnActive: $iface operstate=${state.trim()} up=$up")
        up
    }
}

private fun runNativeProtectionCheck(): NativeResult {
    val checks = listOf(
        "ioctl_flags" to { NativeChecks.checkIoctlSiocgifflags() },
        "ioctl_mtu" to { NativeChecks.checkIoctlSiocgifmtu() },
        "ioctl_conf" to { NativeChecks.checkIoctlSiocgifconf() },
        "getifaddrs" to { NativeChecks.checkGetifaddrs() },
        "netlink_getlink" to { NativeChecks.checkNetlinkGetlink() },
        "netlink_getroute" to { NativeChecks.checkNetlinkGetroute() },
        "proc_route" to { NativeChecks.checkProcNetRoute() },
        "proc_ipv6_route" to { NativeChecks.checkProcNetIpv6Route() },
        "proc_if_inet6" to { NativeChecks.checkProcNetIfInet6() },
        "proc_tcp" to { NativeChecks.checkProcNetTcp() },
        "proc_tcp6" to { NativeChecks.checkProcNetTcp6() },
        "proc_udp" to { NativeChecks.checkProcNetUdp() },
        "proc_udp6" to { NativeChecks.checkProcNetUdp6() },
        "proc_dev" to { NativeChecks.checkProcNetDev() },
        "proc_fib_trie" to { NativeChecks.checkProcNetFibTrie() },
        "sys_class_net" to { NativeChecks.checkSysClassNet() },
    )

    var passed = 0
    var failed = 0
    var skipped = 0
    for ((name, check) in checks) {
        try {
            val result = check()
            when {
                result.startsWith("NETWORK_BLOCKED:") -> {
                    skipped++; Log.d(TAG, "native[$name]: NETWORK_BLOCKED")
                }
                result.contains("SELinux") || result.contains("EACCES") ||
                    result.contains("Permission denied") -> {
                    skipped++; Log.d(TAG, "native[$name]: SELinux blocked, skipping")
                }
                result.startsWith("PASS") -> {
                    passed++; Log.d(TAG, "native[$name]: PASS")
                }
                else -> {
                    failed++; Log.w(TAG, "native[$name]: FAIL — $result")
                }
            }
        } catch (e: Exception) {
            failed++
            Log.e(TAG, "native[$name]: exception — ${e.message}")
        }
    }

    Log.i(TAG, "native protection: passed=$passed failed=$failed skipped=$skipped")
    return when {
        passed == 0 && failed == 0 -> NativeResult.Ok // all SELinux-blocked = nothing leaked
        failed == 0 -> NativeResult.Ok
        passed > 0 -> NativeResult.Fail(passed, failed)
        else -> NativeResult.Fail(0, failed)
    }
}

@Suppress("DEPRECATION")
private fun runJavaProtectionCheck(cm: ConnectivityManager): JavaResult {
    val net = cm.activeNetwork
    if (net == null) { Log.d(TAG, "java: no active network"); return JavaResult.Ok }
    val caps = cm.getNetworkCapabilities(net)
    if (caps == null) { Log.d(TAG, "java: no capabilities"); return JavaResult.Ok }

    var failed = 0

    val hasVpn = caps.hasTransport(NetworkCapabilities.TRANSPORT_VPN)
    if (hasVpn) failed++
    Log.d(TAG, "java: hasTransport(VPN)=$hasVpn")

    val notVpn = caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
    if (!notVpn) failed++
    Log.d(TAG, "java: hasCapability(NOT_VPN)=$notVpn")

    val info = caps.transportInfo
    val isVpnTi = info?.javaClass?.name?.contains("VpnTransportInfo") == true
    if (isVpnTi) failed++
    Log.d(TAG, "java: transportInfo=${info?.javaClass?.name} isVpn=$isVpnTi")

    val vpnNets = cm.allNetworks.count {
        cm.getNetworkCapabilities(it)?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) == true
    }
    if (vpnNets > 0) failed++
    Log.d(TAG, "java: allNetworks vpnCount=$vpnNets")

    val lp = cm.getLinkProperties(net)
    val ifname = lp?.interfaceName
    val vpnPrefixes = listOf("tun", "wg", "ppp", "tap", "ipsec", "xfrm")
    val vpnIfname = ifname != null && vpnPrefixes.any { ifname.startsWith(it) }
    if (vpnIfname) failed++
    Log.d(TAG, "java: linkProperties ifname=$ifname isVpn=$vpnIfname")

    Log.i(TAG, "java protection: failed=$failed")
    return if (failed == 0) JavaResult.Ok else JavaResult.Fail(failed)
}
