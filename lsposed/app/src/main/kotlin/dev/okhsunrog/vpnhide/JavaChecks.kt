package dev.okhsunrog.vpnhide

import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkInfo
import android.net.Uri
import android.os.Build
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
import dev.okhsunrog.vpnhide.checks.CheckStatus
import dev.okhsunrog.vpnhide.checks.NativeProbe
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

private const val TAG = LogTags.TEST

data class CheckResult(
    val name: String,
    val passed: Boolean?,
    val detail: String,
    // The who-hid-it attribution for native probes (root differential). Null for
    // Java-level checks, which have no root ground truth — those fall back to the
    // legacy [passed] tri-state in the UI.
    val outcome: CheckOutcome? = null,
)

internal data class CheckResults(
    // Rust native probes (in-process app view) from the fast phase.
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
    // Honest per-check outcome for the Rust native checks, keyed by spec id —
    // the who-hid-it differential (app view vs root ground truth). Empty when the
    // ground-truth probe couldn't run.
    val nativeOutcomes: Map<String, CheckOutcome> = emptyMap(),
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

    // One JNI call runs every native probe in-process (app view); a root-exec of
    // the same probes gives the unfiltered ground truth for the who-hid-it
    // differential. Join both back to the specs by stable id.
    val nativeAppView = NativeProbe.runAll()
    val nativeGroundTruth = GroundTruthProbe.run(context)
    // One pass builds both the per-check UI results and the outcome map (keyed by
    // spec id for the agent bridge / dashboard). The outcome — the root-differential
    // attribution — rides along on each CheckResult so the UI is a pure function of
    // the list.
    val nativeOutcomes = LinkedHashMap<String, CheckOutcome>()
    val native =
        NATIVE_CHECKS.map { spec ->
            val out =
                nativeAppView[spec.id]
                    ?: CheckOutput(CheckStatus.NETWORK_BLOCKED, "no native result for ${spec.id}")
            val outcome = classifyNativeOutcome(out, nativeGroundTruth[spec.id])
            VpnHideLog.i(TAG, "[outcome] ${spec.id}: ${outcome.token()}")
            nativeOutcomes[spec.id] = outcome
            nativeCheckResult(res.getString(spec.labelRes), out, outcome)
        }

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
        ).logged().withJavaOutcomes()

    return CheckResults(
        native = native,
        nativeExtra = nativeExtra,
        coreJava = coreJava,
        nativeOutcomes = nativeOutcomes,
    )
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
        checkActiveNetworkInfo(cm, res.getString(R.string.check_active_network_info)),
        checkNetworkInfoVpn(cm, res.getString(R.string.check_network_info_vpn)),
    ).logged().withJavaOutcomes()
}

/**
 * Attach the who-hid-it [CheckOutcome] to Java-level checks. The gate guarantees
 * a VPN is up and this app is routed through it, so clean ⟹ hidden by LSPosed,
 * dirty ⟹ leak (see [classifyJavaOutcome]). Applied only to the pure Java-API
 * checks — the Java-implemented native-level probes stay on the passed tri-state.
 */
private fun List<CheckResult>.withJavaOutcomes(): List<CheckResult> = map { it.copy(outcome = classifyJavaOutcome(it.passed)) }

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

private fun nativeCheckResult(
    name: String,
    out: CheckOutput,
    outcome: CheckOutcome? = null,
): CheckResult {
    VpnHideLog.i(TAG, "[$name] ${out.status}: ${out.detail}")
    return CheckResult(name, out.status.toPassed(), out.detail, outcome = outcome)
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

// Legacy NetworkInfo leaks (exercise the LSPOSED_NETWORK_INFO parcel hook +
// the ConnectivityService result sanitizer — surfaces with no other check).
// getActiveNetworkInfo() marshals a NetworkInfo across Binder; without the
// hook an on-VPN caller sees type=VPN. The hook disguises it as WIFI.
@Suppress("DEPRECATION")
private fun checkActiveNetworkInfo(
    cm: ConnectivityManager,
    name: String,
): CheckResult {
    val info = cm.activeNetworkInfo ?: return CheckResult(name, true, "no active network info")
    val isVpn = info.type == ConnectivityManager.TYPE_VPN || info.typeName.equals("VPN", ignoreCase = true)
    val detail =
        if (!isVpn) {
            "type=${info.type} (${info.typeName})"
        } else {
            "activeNetworkInfo type=VPN (${info.typeName})"
        }
    return CheckResult(name, !isVpn, detail)
}

// getNetworkInfo(TYPE_VPN) probes the legacy VPN type directly (issue #85). The
// hook nulls it for a target; a non-null result that reports connected/connecting
// still answers "a VPN-type network exists", which is the leak.
@Suppress("DEPRECATION")
private fun checkNetworkInfoVpn(
    cm: ConnectivityManager,
    name: String,
): CheckResult {
    val info =
        cm.getNetworkInfo(ConnectivityManager.TYPE_VPN)
            ?: return CheckResult(name, true, "getNetworkInfo(TYPE_VPN) returned null")
    val leaks = info.isConnectedOrConnecting
    val detail =
        if (!leaks) {
            "TYPE_VPN state=${info.state}"
        } else {
            "getNetworkInfo(TYPE_VPN) connected: ${info.typeName} ${info.state}"
        }
    return CheckResult(name, !leaks, detail)
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
