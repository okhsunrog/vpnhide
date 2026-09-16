// The deprecated ConnectivityManager surface is the point of this file, not an
// oversight: `allNetworks`, `getNetworkInfo(type)`, `getNetworkInfo(network)`,
// `activeNetworkInfo` and the network-handle calls are exactly what a VPN-probing
// app reaches for, so the checks that prove we hid the tunnel have to reach for the
// same ones. Migrating them to the modern equivalents would silently drop detection
// coverage — see docs/detection-vectors.md for the vector each one stands in for.
//
// Hence file-level rather than the six per-function suppressions this replaces: the
// whole file is deliberately-legacy probe code, and there is no non-probe code here
// for the blanket to hide a genuine deprecation warning in.
@file:Suppress("DEPRECATION")

package dev.okhsunrog.vpnhide.diagnostics

import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkInfo
import android.os.Build
import android.os.Process
import dev.okhsunrog.vpnhide.LogTags
import dev.okhsunrog.vpnhide.R
import dev.okhsunrog.vpnhide.VpnHideLog
import dev.okhsunrog.vpnhide.checks.CheckOutput
import dev.okhsunrog.vpnhide.checks.CheckStatus
import dev.okhsunrog.vpnhide.checks.NativeProbe
import dev.okhsunrog.vpnhide.debug.NET_VIEW_NA
import dev.okhsunrog.vpnhide.debug.NET_VIEW_VIOLATED
import dev.okhsunrog.vpnhide.debug.captureSyncNetworkView
import dev.okhsunrog.vpnhide.generated.IfaceLists
import dev.okhsunrog.vpnhide.next
import java.net.NetworkInterface
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

private const val TAG = LogTags.TEST

data class CheckResult(
    val name: String,
    val detail: String,
    // The who-hid-it classification — the single per-check truth every consumer
    // renders. Native probes get it from the root differential ([classifyNativeOutcome]);
    // Java/nativeExtra probes from their raw observation via [classifyJavaOutcome].
    val outcome: CheckOutcome,
    // The root ground-truth probe's own detail for native checks — what root saw on
    // this surface. Shown next to the app-view detail so the verdict is explained
    // (e.g. "root: 42 routes, no VPN" is WHY a SELinux-blocked read reads as nothing
    // to leak). Null for Java checks and any probe without a root differential.
    val groundTruthDetail: String? = null,
    // Stable, non-localized probe identity: the id of the [NativeCheckSpec] /
    // [JavaCheckSpec] that produced this result. It is the key the probe plan and
    // the per-run outcome map use, so a result is matched by id, never by label
    // or list position. The suite runners always set it; only test fixtures rely
    // on the empty default.
    val id: String = "",
)

/**
 * One Java-implemented probe: the stable [id] the probe plan keys on, the
 * localized label and the runner. [NATIVE_EXTRA_CHECKS], [CORE_JAVA_CHECKS] and
 * [EXTRA_JAVA_CHECKS] are the single registry of these probes, mirroring
 * [NATIVE_CHECKS] for the Rust suite.
 */
internal data class JavaCheckSpec(
    val id: String,
    val labelRes: Int,
    val run: (ConnectivityManager, String) -> CheckResult,
)

/** Java-implemented native-level probes: no hook ownership and no root differential. */
internal val NATIVE_EXTRA_CHECKS: List<JavaCheckSpec> =
    listOf(
        // NetworkInterface.getNetworkInterfaces() is the canonical Java iface-enum a
        // detector uses; kept even though the Rust getifaddrs probe covers the same
        // syscall. (The /proc/net/route Java duplicate was dropped — the Rust probe
        // covers that vector with proper attribution, and the Java one could only
        // report a misleading "OK" on the SELinux denial for want of a root diff.)
        JavaCheckSpec("net_iface_enum", R.string.check_net_iface_enum) { _, name -> checkNetworkInterfaceEnum(name) },
    )

/** VPN-presence framework probes of the fast core phase. */
internal val CORE_JAVA_CHECKS: List<JavaCheckSpec> =
    listOf(
        JavaCheckSpec("has_transport_vpn", R.string.check_has_transport_vpn, ::checkHasTransportVpn),
        JavaCheckSpec("has_capability_not_vpn", R.string.check_has_capability_not_vpn, ::checkHasCapabilityNotVpn),
        JavaCheckSpec("transport_info", R.string.check_transport_info, ::checkTransportInfo),
        JavaCheckSpec("all_networks_vpn", R.string.check_all_networks_vpn, ::checkAllNetworksVpn),
        JavaCheckSpec("link_properties", R.string.check_link_properties, ::checkLinkPropertiesIfname),
    )

/** Remaining framework probes of the slow phase, including the blocking push callback. */
internal val EXTRA_JAVA_CHECKS: List<JavaCheckSpec> =
    listOf(
        JavaCheckSpec("network_for_type_vpn", R.string.check_network_for_type_vpn, ::checkNetworkForTypeVpn),
        JavaCheckSpec("active_network_handle", R.string.check_active_network_handle, ::checkActiveNetworkHandle),
        JavaCheckSpec("all_networks_handles", R.string.check_all_networks_handles, ::checkAllNetworksHandles),
        JavaCheckSpec("active_network_vpn", R.string.check_active_network_vpn, ::checkActiveNetworkVpn),
        JavaCheckSpec("network_callback", R.string.check_network_callback, ::checkNetworkCallbackVpn),
        JavaCheckSpec("link_properties_routes", R.string.check_link_properties_routes, ::checkLinkPropertiesRoutes),
        JavaCheckSpec("network_info_vpn", R.string.check_network_info_vpn, ::checkNetworkInfoVpn),
        JavaCheckSpec("network_view", R.string.check_network_view, ::checkNetworkViewConsistency),
    )

/**
 * Build a Java-level [CheckResult] from a raw tri-state observation. Java and
 * Java-implemented native-level probes have no root differential, so the gate's
 * guarantee (VPN up + this app routed) makes clean ⟹ hidden-by-LSPosed, dirty ⟹
 * leak, unrunnable ⟹ not-measured — exactly [classifyJavaOutcome]. The single
 * place those probes turn a boolean into the canonical [CheckOutcome].
 */
internal fun javaCheck(
    name: String,
    clean: Boolean?,
    detail: String,
): CheckResult = CheckResult(name, detail, outcome = classifyJavaOutcome(clean))

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
) {
    val nativeAll get() = native + nativeExtra
    val java get() = coreJava + extraJava
    val all get() = nativeAll + java
}

/** Vectors the app is protected on out of those actually measured — the numerator
 * counts every non-leak measured outcome, the denominator excludes NotMeasured
 * (probes that couldn't run). Read off [CheckResult.outcome]; backs the agent bridge. */
internal data class CheckScore(
    val passed: Int,
    val total: Int,
)

internal fun Iterable<CheckResult>.protectionScore(): CheckScore {
    val measured = filter { it.outcome !is CheckOutcome.NotMeasured }
    return CheckScore(passed = measured.count { it.outcome !is CheckOutcome.Leak }, total = measured.size)
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
    // The root-differential outcome rides along on each CheckResult, so the UI and
    // the report are a pure function of the list — no parallel outcome map to keep
    // in sync (buildDiagnosticReport derives the by-id map when it needs one).
    val native =
        NATIVE_CHECKS.map { spec ->
            val out =
                nativeAppView[spec.id]
                    ?: CheckOutput(CheckStatus.NETWORK_BLOCKED, "no native result for ${spec.id}")
            val groundTruth = nativeGroundTruth[spec.id]
            val outcome = classifyNativeOutcome(out, groundTruth)
            VpnHideLog.i(TAG, "[outcome] ${spec.id}: ${outcome.token()}")
            nativeCheckResult(spec.id, res.getString(spec.labelRes), out, outcome, groundTruth?.detail)
        }

    return CheckResults(
        native = native,
        nativeExtra = NATIVE_EXTRA_CHECKS.run(cm, res),
        coreJava = CORE_JAVA_CHECKS.run(cm, res),
    )
}

internal fun runExtraJavaChecks(
    cm: ConnectivityManager,
    context: android.content.Context,
): List<CheckResult> = EXTRA_JAVA_CHECKS.run(cm, context.resources)

/** Run a spec registry in order, stamping each result with its stable id. */
private fun List<JavaCheckSpec>.run(
    cm: ConnectivityManager,
    res: android.content.res.Resources,
): List<CheckResult> = map { spec -> spec.run(cm, res.getString(spec.labelRes)).copy(id = spec.id) }.logged()

/** Log each Java check result; native probes already log via [nativeCheck]. */
private fun List<CheckResult>.logged(): List<CheckResult> =
    onEach { c ->
        val status =
            when (c.outcome) {
                CheckOutcome.Leak -> "FAIL"
                is CheckOutcome.NotMeasured -> "SKIP"
                else -> "PASS"
            }
        VpnHideLog.i(TAG, "[${c.name}] $status: ${c.detail}")
    }

private fun nativeCheckResult(
    id: String,
    name: String,
    out: CheckOutput,
    outcome: CheckOutcome,
    groundTruthDetail: String? = null,
): CheckResult {
    VpnHideLog.i(TAG, "[$name] ${out.status}: ${out.detail}")
    return CheckResult(name, out.detail, outcome = outcome, groundTruthDetail = groundTruthDetail, id = id)
}

// ==========================================================================
//  Java API checks
// ==========================================================================

/** Shared preamble for the capability-based checks: resolve the active
 * network's [NetworkCapabilities]. A missing active network / capabilities is
 * reported as not-measured (`javaCheck(name, clean = null, …)`) rather than a green
 * pass: the self-in-tunnel gate guarantees an active network is present, so an
 * absent one means the probe couldn't observe — not that a backend hid the VPN.
 * Classifying it clean would paint a false "hidden by backend" ([classifyJavaOutcome]). */
private inline fun withActiveCaps(
    cm: ConnectivityManager,
    name: String,
    body: (NetworkCapabilities) -> CheckResult,
): CheckResult {
    val net = cm.activeNetwork ?: return javaCheck(name, null, "no active network")
    val caps = cm.getNetworkCapabilities(net) ?: return javaCheck(name, null, "no capabilities")
    return body(caps)
}

/** Shared preamble for the LinkProperties-based checks. A missing active network
 * / link properties is not-measured (`clean = null`) for the same reason as
 * [withActiveCaps] — the gate makes it an unobservable edge, not a clean pass. */
private inline fun withActiveLinkProperties(
    cm: ConnectivityManager,
    name: String,
    body: (LinkProperties) -> CheckResult,
): CheckResult {
    val net = cm.activeNetwork ?: return javaCheck(name, null, "no active network")
    val lp = cm.getLinkProperties(net) ?: return javaCheck(name, null, "no link properties")
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
        javaCheck(name, !hasVpn, detail)
    }

internal fun checkHasCapabilityNotVpn(
    cm: ConnectivityManager,
    name: String,
): CheckResult =
    withActiveCaps(cm, name) { caps ->
        val notVpn = caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
        val detail = if (notVpn) "NOT_VPN capability present" else "NOT_VPN capability MISSING"
        javaCheck(name, notVpn, detail)
    }

internal fun checkTransportInfo(
    cm: ConnectivityManager,
    name: String,
): CheckResult =
    withActiveCaps(cm, name) { caps ->
        // NetworkCapabilities.getTransportInfo() is API 29+. On Android 9 the
        // VpnTransportInfo leak path does not exist, so the check trivially passes.
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) {
            javaCheck(name, true, "transportInfo unavailable (API < 29)")
        } else {
            val info = caps.transportInfo
            val className = info?.javaClass?.name ?: "null"
            val isVpn = className.contains("VpnTransportInfo")
            val detail = if (!isVpn) "transportInfo=$className" else "VpnTransportInfo: $info"
            javaCheck(name, !isVpn, detail)
        }
    }

private fun checkNetworkInterfaceEnum(name: String): CheckResult =
    try {
        val ifaces =
            NetworkInterface.getNetworkInterfaces()
                ?: return javaCheck(name, true, "returned null")
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
        javaCheck(name, vpnNames.isEmpty(), detail)
    } catch (e: Exception) {
        // An exception means the enumeration could not be observed — that is
        // not-measured, not a leak. Reporting it as a leak (clean=false) paints
        // a false FAIL on an unmeasurable surface.
        javaCheck(name, null, "${e.message}")
    }

internal fun checkAllNetworksVpn(
    cm: ConnectivityManager,
    name: String,
): CheckResult {
    val networks = cm.allNetworks
    if (networks.isEmpty()) return javaCheck(name, true, "no networks")
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
    return javaCheck(name, vpnNetworks.isEmpty(), detail)
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
        javaCheck(name, !hasVpn, detail)
    }

private data class NetworkForTypeResult(
    val network: Network? = null,
    val unavailable: Boolean = false,
    val error: String? = null,
)

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

private fun checkNetworkForTypeVpn(
    cm: ConnectivityManager,
    name: String,
): CheckResult {
    val result = queryNetworkForType(cm, ConnectivityManager.TYPE_VPN)
    result.error?.let { return javaCheck(name, false, it) }
    // Reflection couldn't reach getNetworkForType — probe didn't run (not measured),
    // not a clean pass. A non-null TYPE_VPN network below is still a legitimate hidden.
    if (result.unavailable) return javaCheck(name, null, "getNetworkForType unavailable")
    val vpnNetwork = result.network ?: return javaCheck(name, true, "TYPE_VPN returned null")

    val caps = cm.getNetworkCapabilities(vpnNetwork)
    val hasVpn = caps?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) == true
    val detail =
        if (hasVpn) {
            "TYPE_VPN returned $vpnNetwork with VPN capabilities"
        } else {
            "TYPE_VPN returned $vpnNetwork"
        }
    return javaCheck(name, false, detail)
}

private fun checkActiveNetworkHandle(
    cm: ConnectivityManager,
    name: String,
): CheckResult {
    val active = cm.activeNetwork ?: return javaCheck(name, null, "no active network")
    val vpnResult = queryNetworkForType(cm, ConnectivityManager.TYPE_VPN)
    vpnResult.error?.let { return javaCheck(name, false, it) }
    if (vpnResult.unavailable) return javaCheck(name, null, "active=$active, getNetworkForType unavailable")
    val vpnNetwork = vpnResult.network ?: return javaCheck(name, true, "active=$active, TYPE_VPN not exposed")

    val leaksVpnHandle = active == vpnNetwork
    val detail =
        if (leaksVpnHandle) {
            "activeNetwork equals TYPE_VPN network $active"
        } else {
            "active=$active, TYPE_VPN=$vpnNetwork"
        }
    return javaCheck(name, !leaksVpnHandle, detail)
}

private fun checkAllNetworksHandles(
    cm: ConnectivityManager,
    name: String,
): CheckResult {
    val networks = cm.allNetworks
    val vpnResult = queryNetworkForType(cm, ConnectivityManager.TYPE_VPN)
    vpnResult.error?.let { return javaCheck(name, false, it) }
    if (vpnResult.unavailable) return javaCheck(name, null, "${networks.size} networks, getNetworkForType unavailable")
    val vpnNetwork = vpnResult.network ?: return javaCheck(name, true, "${networks.size} networks, TYPE_VPN not exposed")

    val containsVpnHandle = networks.any { it == vpnNetwork }
    val detail =
        if (containsVpnHandle) {
            "allNetworks includes TYPE_VPN network $vpnNetwork"
        } else {
            "${networks.size} networks, TYPE_VPN=$vpnNetwork not listed"
        }
    return javaCheck(name, !containsVpnHandle, detail)
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
    // Both the capabilities and the link properties are needed, and Android
    // delivers them in separate events (onAvailable → onCapabilitiesChanged →
    // onLinkPropertiesChanged), so the latch must trip only once BOTH have
    // arrived for the same handle — not on the capabilities alone, which would
    // race the link properties and let a mismatched interface pass unread.
    val latch = CountDownLatch(1)
    val seenNetwork = AtomicReference<Network?>(null)
    val seenCaps = AtomicReference<NetworkCapabilities?>(null)
    val seenLp = AtomicReference<LinkProperties?>(null)

    fun completeIfReady() {
        if (seenNetwork.get() != null && seenCaps.get() != null && seenLp.get() != null) latch.countDown()
    }
    val callback =
        object : ConnectivityManager.NetworkCallback() {
            override fun onLinkPropertiesChanged(
                network: Network,
                lp: LinkProperties,
            ) {
                seenLp.set(lp)
                completeIfReady()
            }

            override fun onCapabilitiesChanged(
                network: Network,
                caps: NetworkCapabilities,
            ) {
                seenNetwork.set(network)
                seenCaps.set(caps)
                completeIfReady()
            }
        }
    return try {
        cm.registerDefaultNetworkCallback(callback)
        val fired = latch.await(3, TimeUnit.SECONDS)
        val caps = seenCaps.get()
        val lp = seenLp.get()
        if (!fired || caps == null || lp == null) {
            // The full event pair did not arrive within the deadline: a
            // non-observation, not evidence of hiding. Reporting it clean would
            // mask a broken/slow push path — surface it as not-measured, and say
            // which half was missing so a genuinely slow device is diagnosable.
            javaCheck(name, null, "no complete callback (caps=${caps != null}, linkProperties=${lp != null})")
        } else {
            val (detail, clean) = callbackCleanAndCoherent(cm, seenNetwork.get(), caps, lp)
            javaCheck(name, clean, detail)
        }
    } catch (e: Exception) {
        javaCheck(name, false, e.message ?: e.javaClass.simpleName)
    } finally {
        runCatching { cm.unregisterNetworkCallback(callback) }
    }
}

/**
 * A pushed default-network callback must be clean (no VPN transport, NOT_VPN
 * present) AND coherent with the synchronous view: the link properties it
 * carried must name the same interface that `getLinkProperties(handle)` returns
 * for the handle it delivered. A cover handle pushed with the VPN's emptied link
 * properties — the exact incoherence the network-view work fixes — trips the
 * second half even when the capabilities read clean.
 */
private fun callbackCleanAndCoherent(
    cm: ConnectivityManager,
    network: Network?,
    caps: NetworkCapabilities,
    lp: LinkProperties?,
): Pair<String, Boolean> {
    val hasVpn = caps.hasTransport(NetworkCapabilities.TRANSPORT_VPN)
    val notVpn = caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
    if (hasVpn || !notVpn) {
        return "callback leaks VPN: hasTransport(VPN)=$hasVpn, NOT_VPN=$notVpn" to false
    }
    val callbackIface = lp?.interfaceName
    val syncIface = network?.let { runCatching { cm.getLinkProperties(it)?.interfaceName }.getOrNull() }
    if (callbackIface != null && syncIface != null && callbackIface != syncIface) {
        return "callback handle $network carries iface=$callbackIface but sync says $syncIface" to false
    }
    return "callback caps clean and link properties coherent (iface=$callbackIface)" to true
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
        javaCheck(name, !isVpn, detail)
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
        javaCheck(name, vpnRoutes.isEmpty(), detail)
    }

// getNetworkInfo(TYPE_VPN) probes the legacy VPN type directly (issue #85). The
// ConnectivityService result hook returns the platform's disconnected VPN-type
// info; the parcel hook must preserve it. Validate type and enumeration as well
// as state: a disconnected WIFI reply or null must not silently pass this check.
// (Its companion getActiveNetworkInfo() was dropped: .type reports the
// underlying transport (WIFI/mobile) for an active VPN, not TYPE_VPN, so it never
// surfaced the leak.)
private fun checkNetworkInfoVpn(
    cm: ConnectivityManager,
    name: String,
): CheckResult {
    val direct = cm.getNetworkInfo(ConnectivityManager.TYPE_VPN)?.legacySnapshot()
    val all = cm.allNetworkInfo.map { it.legacySnapshot() }
    val assessment = assessLegacyVpnInfo(direct, all)
    return javaCheck(name, assessment.clean, assessment.detail)
}

private fun NetworkInfo.legacySnapshot(): LegacyVpnInfoSnapshot = LegacyVpnInfoSnapshot(type, state.name, detailedState.name, isAvailable)

// Cross-vector consistency, not a single leak: the synchronous network model this
// app sees must hold together — the active handle is listed, a listed network has
// a transport, a connected one has an interface, its NetworkInfo type names a
// transport it has, and no VPN handle answers outside the enumeration. Reuses the
// same invariants the debug bundle and the external probe evaluate. The push-path
// invariants are guarded by [checkNetworkCallbackVpn]; this one needs no callback
// window. Under the gate the VPN is up and this app is routed, so a VPN still
// visible anywhere here is a leak.
private fun checkNetworkViewConsistency(
    cm: ConnectivityManager,
    name: String,
): CheckResult {
    val snapshot = captureSyncNetworkView(cm, Process.myUid(), expectHidden = true)
    // A capture that hit an error read a partial model: active/allNetworks or a
    // per-network fact could not be read, so an empty or truncated snapshot could
    // show "no violations" without having observed anything. That is not-measured,
    // not a pass — the absence of a violation must never be manufactured from a
    // failed read. (The invariant evaluation only proves a *populated* model
    // holds together.)
    if (snapshot.errors.isNotEmpty()) {
        return javaCheck(name, null, "capture incomplete: ${snapshot.errors.joinToString("; ")}")
    }
    val violations = snapshot.invariants.filter { it.status == NET_VIEW_VIOLATED }
    val checked = snapshot.invariants.count { it.status != NET_VIEW_NA }
    val detail =
        if (violations.isEmpty()) {
            "$checked invariants hold across ${snapshot.allNetworks.size} network(s)"
        } else {
            violations.joinToString("; ") { "${it.id}: ${it.detail}" }
        }
    return javaCheck(name, violations.isEmpty(), detail)
}
