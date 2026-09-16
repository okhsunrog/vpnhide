package dev.okhsunrog.vpnhide.debug

import kotlinx.serialization.Serializable

/**
 * The framework network model exactly as ONE uid sees it after Binder: every
 * handle, the facts each handle answers, the legacy type answers, what the push
 * callbacks delivered, and the consistency invariants evaluated over all of it.
 *
 * This is the measurement the LSPosed consistency work is judged by. A target
 * uid's snapshot must agree with a non-target's on every invariant except the
 * absence of the VPN, and every object must describe the network the handle
 * names — a callback carrying a Wi-Fi handle with a transport-less capability
 * set is the exact defect this shape makes visible.
 *
 * Captured in two places from the same code: the app's own debug bundle
 * ([captureNetworkView]) and the root `app_process` probe
 * ([NetworkViewProbeMain]) that runs it under any uid. Pure logic here, Android
 * calls in `NetworkViewCapture.kt`.
 */
@Serializable
internal data class NetworkViewSnapshot(
    val uid: Int,
    val packageName: String,
    val sdk: Int,
    val capturedAt: String,
    // How long the callback registrations stayed open collecting events.
    val captureMs: Long,
    // Whether the capture expects the VPN to be hidden from this uid (a target
    // with the VPN up). Decides whether the VPN-absence invariants apply.
    val expectHidden: Boolean,
    val activeNetwork: Int?,
    val boundNetwork: Int?,
    val allNetworks: List<Int>,
    // One entry per netId seen anywhere: `allNetworks`, the active handle, the
    // callbacks, or the netId scan.
    val networks: List<NetworkFacts>,
    val legacy: LegacyTypeView,
    // netIds outside `allNetworks` that still answered a non-null capability set
    // to a blind `new Network(netId)` query.
    val phantomNetworks: List<Int>,
    val callbacks: List<CallbackEvent>,
    val pendingIntent: PendingIntentResult?,
    val invariants: List<InvariantResult>,
    val errors: List<String>,
)

@Serializable
internal data class NetworkFacts(
    val netId: Int,
    // "all" | "active" | "callback" | "scan"
    val sources: List<String>,
    val capabilities: CapabilityFacts?,
    val linkProperties: LinkFacts?,
    val networkInfo: NetworkInfoFacts?,
)

@Serializable
internal data class CapabilityFacts(
    val transports: List<String>,
    val capabilities: List<String>,
    // Class name only: the object itself carries SSID/BSSID.
    val transportInfo: String?,
    val ownerUid: Int,
)

@Serializable
internal data class LinkFacts(
    val interfaceName: String?,
    val addresses: List<String>,
    val dnsServers: List<String>,
    val routes: List<String>,
    val stackedInterfaces: List<String>,
    val mtu: Int,
)

@Serializable
internal data class NetworkInfoFacts(
    val type: Int,
    val typeName: String,
    val subtype: Int,
    val state: String,
    val detailedState: String,
    val available: Boolean,
    val extraInfo: String?,
)

@Serializable
internal data class LegacyTypeView(
    val activeNetworkInfo: NetworkInfoFacts?,
    // Keyed by legacy type name: VPN, WIFI, MOBILE.
    val byType: Map<String, NetworkInfoFacts?>,
    val allNetworkInfo: List<NetworkInfoFacts>,
    val networkForType: Map<String, Int?>,
)

@Serializable
internal data class CallbackEvent(
    // "default" | "listen_any" | "listen_vpn"
    val registration: String,
    // "available" | "capabilities" | "link_properties" | "lost" | "blocked" | "unavailable"
    val event: String,
    val netId: Int?,
    // Milliseconds since the registrations were opened.
    val tMs: Long,
    val capabilities: CapabilityFacts? = null,
    val linkProperties: LinkFacts? = null,
    val blocked: Int? = null,
)

@Serializable
internal data class PendingIntentResult(
    val registered: Boolean,
    // EXTRA_NETWORK of the one delivery a PendingIntent *listen* without NOT_VPN
    // gets (ConnectivityService sends a PendingIntent once, for the first match).
    val netIds: List<Int>,
    // Same for a PendingIntent *request* for INTERNET without NOT_VPN: satisfied
    // by this uid's best network, which inside a VPN is the VPN itself — the
    // case where the handle in the intent must have been replaced.
    val requestNetIds: List<Int> = emptyList(),
    val error: String? = null,
)

@Serializable
internal data class InvariantResult(
    val id: String,
    // "ok" | "violated" | "not_applicable"
    val status: String,
    val detail: String,
)

internal const val NET_VIEW_OK = "ok"
internal const val NET_VIEW_VIOLATED = "violated"
internal const val NET_VIEW_NA = "not_applicable"

// NetworkCapabilities.TRANSPORT_* by value. hasTransport() answers false for an
// id this build does not know, so probing the whole table is safe on every API.
internal val TRANSPORT_NAMES: Map<Int, String> =
    mapOf(
        0 to "CELLULAR",
        1 to "WIFI",
        2 to "BLUETOOTH",
        3 to "ETHERNET",
        4 to "VPN",
        5 to "WIFI_AWARE",
        6 to "LOWPAN",
        7 to "TEST",
        8 to "USB",
        9 to "THREAD",
        10 to "SATELLITE",
    )

// NetworkCapabilities.NET_CAPABILITY_* by value, the ones a consistency reader
// cares about. Anything else is reported as cap<N>.
internal val CAPABILITY_NAMES: Map<Int, String> =
    mapOf(
        11 to "NOT_METERED",
        12 to "INTERNET",
        13 to "NOT_RESTRICTED",
        14 to "TRUSTED",
        15 to "NOT_VPN",
        16 to "VALIDATED",
        17 to "CAPTIVE_PORTAL",
        18 to "NOT_ROAMING",
        19 to "FOREGROUND",
        20 to "NOT_CONGESTED",
        21 to "NOT_SUSPENDED",
        25 to "TEMPORARILY_NOT_METERED",
        28 to "NOT_VCN_MANAGED",
        36 to "LOCAL_NETWORK",
    )

internal const val CAPABILITY_ID_MAX = 40

// Legacy ConnectivityManager.TYPE_* → the transport that type describes.
internal val LEGACY_TYPE_TRANSPORT: Map<Int, String> =
    mapOf(0 to "CELLULAR", 1 to "WIFI", 7 to "BLUETOOTH", 9 to "ETHERNET", 17 to "VPN")

internal fun transportName(id: Int): String = TRANSPORT_NAMES[id] ?: "transport$id"

internal fun capabilityName(id: Int): String = CAPABILITY_NAMES[id] ?: "cap$id"

private val INACTIVE_DETAILED_STATES = setOf("DISCONNECTED", "BLOCKED")

/** Everything a capture observed, before the invariants are evaluated over it. */
internal data class NetworkViewObservations(
    val activeNetwork: Int?,
    val allNetworks: List<Int>?,
    val networks: List<NetworkFacts>,
    val legacy: LegacyTypeView,
    val phantomNetworks: List<Int>,
    val callbacks: List<CallbackEvent>,
    val pendingIntent: PendingIntentResult?,
    val scanComplete: Boolean = true,
    val topologyStable: Boolean = true,
)

/**
 * Evaluate the consistency invariants over one captured view. Pure so the exact
 * rules the bundle reports are unit-tested; the capture only fills the inputs.
 */
internal fun evaluateNetworkView(
    observed: NetworkViewObservations,
    expectHidden: Boolean,
): List<InvariantResult> {
    val all = observed.allNetworks.orEmpty()
    val byId = observed.networks.associateBy { it.netId }
    val listed = observed.networks.filter { it.netId in all }
    return listOf(
        activeInAll(observed.activeNetwork, all),
        transportsNonEmpty(listed),
        connectedHasInterface(listed),
        infoTypeMatchesTransport(listed),
        noPhantoms(observed.phantomNetworks),
        activeInfoMatchesActive(observed.activeNetwork, byId, observed.legacy),
        callbacksMatchSync(observed.callbacks, byId, all),
        callbacksOnlyListed(observed.callbacks, all),
        pendingIntentOnlyListed(observed.pendingIntent, all),
    ).map { result ->
        when {
            !observed.topologyStable -> {
                notApplicable(result.id, "network topology changed during capture")
            }

            observed.allNetworks == null && result.id in ENUMERATION_INVARIANTS -> {
                notApplicable(result.id, "network enumeration unavailable")
            }

            !observed.scanComplete && result.id == "no_phantom_networks" && result.status != NET_VIEW_VIOLATED -> {
                notApplicable(result.id, "network scan incomplete")
            }

            else -> {
                result
            }
        }
    } + if (expectHidden) vpnAbsent(observed.networks, observed.legacy, observed.callbacks) else emptyList()
}

// These comparisons require a successful enumeration, not an empty error fallback.
private val ENUMERATION_INVARIANTS =
    setOf(
        "active_in_all_networks",
        "listed_networks_have_transport",
        "connected_network_has_interface",
        "info_type_matches_transport",
        "no_phantom_networks",
        "callbacks_only_for_listed_networks",
        "pending_intent_only_listed_networks",
    )

private fun ok(
    id: String,
    detail: String,
) = InvariantResult(id, NET_VIEW_OK, detail)

private fun violated(
    id: String,
    detail: String,
) = InvariantResult(id, NET_VIEW_VIOLATED, detail)

private fun notApplicable(
    id: String,
    detail: String,
) = InvariantResult(id, NET_VIEW_NA, detail)

private fun activeInAll(
    active: Int?,
    all: List<Int>,
): InvariantResult {
    val id = "active_in_all_networks"
    if (active == null) return notApplicable(id, "no active network")
    return if (active in all) ok(id, "active=$active listed") else violated(id, "active=$active not in $all")
}

private fun transportsNonEmpty(listed: List<NetworkFacts>): InvariantResult {
    val id = "listed_networks_have_transport"
    val withCaps = listed.filter { it.capabilities != null }
    if (withCaps.isEmpty()) return notApplicable(id, "no capabilities answered")
    val empty = withCaps.filter { it.capabilities?.transports.isNullOrEmpty() }.map { it.netId }
    return if (empty.isEmpty()) ok(id, "${withCaps.size} networks") else violated(id, "no transport on $empty")
}

private fun connectedHasInterface(listed: List<NetworkFacts>): InvariantResult {
    val id = "connected_network_has_interface"
    val connected = listed.filter { it.networkInfo?.state == "CONNECTED" && it.linkProperties != null }
    if (connected.isEmpty()) return notApplicable(id, "no connected network with link properties")
    val bare = connected.filter { it.linkProperties?.interfaceName == null }.map { it.netId }
    return if (bare.isEmpty()) ok(id, "${connected.size} networks") else violated(id, "no interface on $bare")
}

private fun infoTypeMatchesTransport(listed: List<NetworkFacts>): InvariantResult {
    val id = "info_type_matches_transport"
    val comparable = listed.filter { it.networkInfo != null && it.capabilities != null }
    if (comparable.isEmpty()) return notApplicable(id, "no network with both info and capabilities")
    val mismatched =
        comparable.filter { facts ->
            val expected = LEGACY_TYPE_TRANSPORT[facts.networkInfo?.type ?: -1]
            expected != null && expected !in facts.capabilities!!.transports
        }
    return if (mismatched.isEmpty()) {
        ok(id, "${comparable.size} networks")
    } else {
        violated(
            id,
            mismatched.joinToString { "${it.netId}: info=${it.networkInfo?.typeName} transports=${it.capabilities?.transports}" },
        )
    }
}

private fun noPhantoms(phantoms: List<Int>): InvariantResult {
    val id = "no_phantom_networks"
    return if (phantoms.isEmpty()) ok(id, "scan found nothing outside allNetworks") else violated(id, "answering netIds $phantoms")
}

private fun activeInfoMatchesActive(
    active: Int?,
    byId: Map<Int, NetworkFacts>,
    legacy: LegacyTypeView,
): InvariantResult {
    val id = "active_info_matches_active_network"
    val info = legacy.activeNetworkInfo
    val transports = active?.let { byId[it]?.capabilities?.transports }
    if (info == null || transports == null) return notApplicable(id, "activeInfo=${info?.typeName} active=$active")
    val expected = LEGACY_TYPE_TRANSPORT[info.type] ?: return notApplicable(id, "type ${info.typeName} has no transport")
    return if (expected in transports) {
        ok(id, "activeInfo=${info.typeName} transports=$transports")
    } else {
        violated(id, "activeInfo=${info.typeName} but active $active has $transports")
    }
}

private fun callbacksMatchSync(
    callbacks: List<CallbackEvent>,
    byId: Map<Int, NetworkFacts>,
    all: List<Int>,
): InvariantResult {
    val id = "callback_matches_sync_view"
    val comparable = callbacks.filter { it.netId in all && (it.capabilities != null || it.linkProperties != null) }
    if (comparable.isEmpty()) return notApplicable(id, "no callback payload for a listed network")
    val mismatches = comparable.mapNotNull { describeMismatch(it, byId[it.netId]) }
    return if (mismatches.isEmpty()) ok(id, "${comparable.size} payloads agree") else violated(id, mismatches.joinToString("; "))
}

private fun describeMismatch(
    event: CallbackEvent,
    facts: NetworkFacts?,
): String? {
    val sync = facts ?: return "${event.netId}: no sync facts"
    val caps = event.capabilities
    if (caps != null && sync.capabilities != null && caps.transports.toSet() != sync.capabilities.transports.toSet()) {
        return "${event.netId} ${event.registration}/${event.event}: transports ${caps.transports} vs sync ${sync.capabilities.transports}"
    }
    val lp = event.linkProperties
    if (lp != null && sync.linkProperties != null && lp.interfaceName != sync.linkProperties.interfaceName) {
        return "${event.netId} ${event.registration}/${event.event}: iface ${lp.interfaceName} vs sync ${sync.linkProperties.interfaceName}"
    }
    return null
}

private fun callbacksOnlyListed(
    callbacks: List<CallbackEvent>,
    all: List<Int>,
): InvariantResult {
    val id = "callbacks_only_for_listed_networks"
    val withNet = callbacks.filter { it.netId != null && it.event != "lost" }
    if (withNet.isEmpty()) return notApplicable(id, "no callback named a network")
    val unlisted = withNet.filter { it.netId !in all }.map { "${it.registration}/${it.event}:${it.netId}" }.distinct()
    return if (unlisted.isEmpty()) ok(id, "${withNet.size} events") else violated(id, unlisted.joinToString())
}

private fun pendingIntentOnlyListed(
    result: PendingIntentResult?,
    all: List<Int>,
): InvariantResult {
    val id = "pending_intent_only_listed_networks"
    if (result == null || !result.registered) return notApplicable(id, result?.error ?: "not captured")
    val delivered = result.netIds + result.requestNetIds
    val unlisted = delivered.filter { it !in all }.distinct()
    return if (unlisted.isEmpty()) {
        ok(id, "listen ${result.netIds} request ${result.requestNetIds}")
    } else {
        violated(id, "delivered unlisted $unlisted (listen ${result.netIds} request ${result.requestNetIds})")
    }
}

private fun vpnAbsent(
    listed: List<NetworkFacts>,
    legacy: LegacyTypeView,
    callbacks: List<CallbackEvent>,
): List<InvariantResult> {
    val transportLeaks =
        listed.filter { "VPN" in it.capabilities?.transports.orEmpty() }.map { it.netId } +
            callbacks.filter { "VPN" in it.capabilities?.transports.orEmpty() }.mapNotNull { it.netId }
    val vpnInfo = legacy.byType["VPN"]
    val legacyLeak =
        (vpnInfo != null && vpnInfo.detailedState !in INACTIVE_DETAILED_STATES) ||
            legacy.allNetworkInfo.any { it.type == 17 && it.detailedState !in INACTIVE_DETAILED_STATES } ||
            legacy.networkForType["VPN"] != null
    val vpnCallbacks = callbacks.filter { it.registration == "listen_vpn" && it.event != "unavailable" }
    return listOf(
        if (transportLeaks.isEmpty()) {
            ok("no_vpn_transport", "no TRANSPORT_VPN anywhere")
        } else {
            violated("no_vpn_transport", "TRANSPORT_VPN on ${transportLeaks.distinct()}")
        },
        if (legacyLeak) {
            violated("legacy_vpn_inactive", "vpn=${vpnInfo?.detailedState} handle=${legacy.networkForType["VPN"]}")
        } else {
            ok("legacy_vpn_inactive", "vpn=${vpnInfo?.detailedState ?: "null"}")
        },
        if (vpnCallbacks.isEmpty()) {
            ok("vpn_listen_silent", "no events on a TRANSPORT_VPN listen")
        } else {
            violated("vpn_listen_silent", "${vpnCallbacks.size} events on a TRANSPORT_VPN listen")
        },
    )
}
