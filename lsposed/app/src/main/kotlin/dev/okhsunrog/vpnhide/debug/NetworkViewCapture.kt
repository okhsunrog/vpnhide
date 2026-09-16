// The deprecated legacy ConnectivityManager surface is the point of this capture:
// NetworkInfo, getNetworkInfo(type), allNetworkInfo and getNetworkForType are
// exactly what a VPN-probing app compares against the modern handles, so the
// consistency snapshot must read the same calls.
@file:Suppress("DEPRECATION")

package dev.okhsunrog.vpnhide.debug

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkInfo
import android.net.NetworkRequest
import android.os.Build
import android.os.SystemClock
import java.util.concurrent.CopyOnWriteArrayList

/** How a [NetworkViewSnapshot] is taken. */
internal data class NetworkViewOptions(
    val captureMs: Long = DEFAULT_CAPTURE_MS,
    val expectHidden: Boolean,
    // A PendingIntent registration needs a real app process (a BroadcastReceiver
    // bound to this package); the root app_process probe cannot host one.
    val includePendingIntent: Boolean,
    // How far past the highest listed netId the blind `Network(netId)` scan looks.
    val scanBeyond: Int = DEFAULT_SCAN_BEYOND,
)

private const val DEFAULT_CAPTURE_MS = 2_000L
private const val DEFAULT_SCAN_BEYOND = 30
private const val FIRST_NET_ID = 100
private const val NETWORK_HANDLE_MAGIC = 0xcafed00dL
private const val LISTEN_INTENT_ACTION = "dev.okhsunrog.vpnhide.debug.NETWORK_VIEW_LISTEN"
private const val REQUEST_INTENT_ACTION = "dev.okhsunrog.vpnhide.debug.NETWORK_VIEW_REQUEST"

private val LEGACY_TYPES = mapOf("MOBILE" to 0, "WIFI" to 1, "VPN" to 17)

/**
 * Capture the framework network model as this process's uid sees it: every
 * synchronous answer, the push callbacks over [NetworkViewOptions.captureMs], and
 * the invariants over both. Blocking for the capture window; call off the main
 * thread. Every probe is individually guarded so one throwing API degrades to an
 * entry in `errors` instead of losing the snapshot.
 */
internal fun captureNetworkView(
    context: Context,
    cm: ConnectivityManager,
    uid: Int,
    packageName: String,
    options: NetworkViewOptions,
): NetworkViewSnapshot {
    val errors = mutableListOf<String>()
    val active = guard(errors, "activeNetwork") { cm.activeNetwork?.netId() }
    val bound = guard(errors, "boundNetworkForProcess") { cm.boundNetworkForProcess?.netId() }
    val all = guard(errors, "allNetworks") { cm.allNetworks.mapNotNull { it.netId() } }.orEmpty()
    val sources = linkedMapOf<Int, MutableSet<String>>()
    all.forEach { sources.getOrPut(it) { linkedSetOf() } += "all" }
    active?.let { sources.getOrPut(it) { linkedSetOf() } += "active" }

    val callbacks = captureCallbacks(cm, options.captureMs, errors)
    callbacks.mapNotNull { it.netId }.forEach { sources.getOrPut(it) { linkedSetOf() } += "callback" }
    val pendingIntent = if (options.includePendingIntent) capturePendingIntent(context, cm, options.captureMs) else null
    pendingIntent?.let { it.netIds + it.requestNetIds }?.forEach { sources.getOrPut(it) { linkedSetOf() } += "callback" }

    val phantoms = scanPhantoms(cm, all, options.scanBeyond)
    phantoms.forEach { sources.getOrPut(it) { linkedSetOf() } += "scan" }

    val networks = sources.map { (netId, from) -> factsFor(cm, netId, from.toList()) }
    val legacy = captureLegacy(cm, errors)
    return NetworkViewSnapshot(
        uid = uid,
        packageName = packageName,
        sdk = Build.VERSION.SDK_INT,
        capturedAt = isoNow(),
        captureMs = options.captureMs,
        expectHidden = options.expectHidden,
        activeNetwork = active,
        boundNetwork = bound,
        allNetworks = all,
        networks = networks,
        legacy = legacy,
        phantomNetworks = phantoms,
        callbacks = callbacks,
        pendingIntent = pendingIntent,
        invariants =
            evaluateNetworkView(
                NetworkViewObservations(active, all, networks, legacy, phantoms, callbacks, pendingIntent),
                options.expectHidden,
            ),
        errors = errors,
    )
}

/**
 * The synchronous half of the network view — every handle and the facts each
 * answers, the legacy type answers, the netId scan, and the invariants over
 * them — with no callback window, no PendingIntent and no Context. The app's
 * diagnostics run this as a self-test after the routing gate has confirmed the
 * VPN is up and this app is routed, so [expectHidden] is true and the
 * VPN-absence invariants apply. The push-path invariants come out
 * not-applicable (no callbacks captured); [checkNetworkCallbackVpn] guards
 * those separately without a second callback window.
 */
internal fun captureSyncNetworkView(
    cm: ConnectivityManager,
    uid: Int,
    expectHidden: Boolean,
): NetworkViewSnapshot {
    val errors = mutableListOf<String>()
    val active = guard(errors, "activeNetwork") { cm.activeNetwork?.netId() }
    val all = guard(errors, "allNetworks") { cm.allNetworks.mapNotNull { it.netId() } }.orEmpty()
    val sources = linkedMapOf<Int, MutableSet<String>>()
    all.forEach { sources.getOrPut(it) { linkedSetOf() } += "all" }
    active?.let { sources.getOrPut(it) { linkedSetOf() } += "active" }
    val phantoms = scanPhantoms(cm, all, DEFAULT_SCAN_BEYOND)
    phantoms.forEach { sources.getOrPut(it) { linkedSetOf() } += "scan" }
    val networks = sources.map { (netId, from) -> factsFor(cm, netId, from.toList()) }
    val legacy = captureLegacy(cm, errors)
    val observed = NetworkViewObservations(active, all, networks, legacy, phantoms, emptyList(), null)
    return NetworkViewSnapshot(
        uid = uid,
        packageName = "self",
        sdk = Build.VERSION.SDK_INT,
        capturedAt = isoNow(),
        captureMs = 0,
        expectHidden = expectHidden,
        activeNetwork = active,
        boundNetwork = null,
        allNetworks = all,
        networks = networks,
        legacy = legacy,
        phantomNetworks = phantoms,
        callbacks = emptyList(),
        pendingIntent = null,
        invariants = evaluateNetworkView(observed, expectHidden),
        errors = errors,
    )
}

private inline fun <T> guard(
    errors: MutableList<String>,
    what: String,
    block: () -> T,
): T? =
    try {
        block()
    } catch (t: Throwable) {
        errors += "$what: ${t.javaClass.simpleName}: ${t.message}"
        null
    }

/** `Network.toString()` is the netId on every Android version; `getNetId()` is hidden. */
internal fun Network.netId(): Int? = toString().toIntOrNull()

/** Build a handle for a netId the way the framework encodes it, through the public factory. */
internal fun networkForNetId(netId: Int): Network = Network.fromNetworkHandle((netId.toLong() shl 32) or NETWORK_HANDLE_MAGIC)

private fun factsFor(
    cm: ConnectivityManager,
    netId: Int,
    sources: List<String>,
): NetworkFacts {
    val network = networkForNetId(netId)
    return NetworkFacts(
        netId = netId,
        sources = sources,
        capabilities = runCatching { cm.getNetworkCapabilities(network)?.let(::capabilityFacts) }.getOrNull(),
        linkProperties = runCatching { cm.getLinkProperties(network)?.let(::linkFacts) }.getOrNull(),
        networkInfo = runCatching { cm.getNetworkInfo(network)?.let(::infoFacts) }.getOrNull(),
    )
}

internal fun capabilityFacts(nc: NetworkCapabilities): CapabilityFacts =
    CapabilityFacts(
        transports = TRANSPORT_NAMES.keys.filter { nc.hasTransport(it) }.map(::transportName),
        capabilities = (0..CAPABILITY_ID_MAX).filter { nc.hasCapability(it) }.map(::capabilityName),
        transportInfo =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) nc.transportInfo?.javaClass?.simpleName else null,
        ownerUid = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) nc.ownerUid else -1,
    )

internal fun linkFacts(lp: LinkProperties): LinkFacts =
    LinkFacts(
        interfaceName = lp.interfaceName,
        addresses = lp.linkAddresses.map { it.toString() },
        dnsServers = lp.dnsServers.map { it.hostAddress ?: "?" },
        routes = lp.routes.map { "${it.destination} via ${it.gateway?.hostAddress} dev ${it.`interface`}" },
        stackedInterfaces = stackedInterfaces(lp),
        mtu = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) lp.mtu else -1,
    )

// getStackedLinks() is a system API; a hidden-API refusal just reports no stacked links.
@Suppress("UNCHECKED_CAST")
private fun stackedInterfaces(lp: LinkProperties): List<String> =
    runCatching {
        (LinkProperties::class.java.getMethod("getStackedLinks").invoke(lp) as? List<LinkProperties>)
            .orEmpty()
            .map { it.interfaceName ?: "?" }
    }.getOrDefault(emptyList())

internal fun infoFacts(ni: NetworkInfo): NetworkInfoFacts =
    NetworkInfoFacts(
        type = ni.type,
        typeName = ni.typeName,
        subtype = ni.subtype,
        state = ni.state.name,
        detailedState = ni.detailedState.name,
        available = ni.isAvailable,
        extraInfo = ni.extraInfo,
    )

private fun captureLegacy(
    cm: ConnectivityManager,
    errors: MutableList<String>,
): LegacyTypeView =
    LegacyTypeView(
        activeNetworkInfo = guard(errors, "activeNetworkInfo") { cm.activeNetworkInfo?.let(::infoFacts) },
        byType =
            LEGACY_TYPES.mapValues { (name, type) ->
                guard(errors, "getNetworkInfo($name)") { cm.getNetworkInfo(type)?.let(::infoFacts) }
            },
        allNetworkInfo = guard(errors, "allNetworkInfo") { cm.allNetworkInfo.map(::infoFacts) }.orEmpty(),
        networkForType = LEGACY_TYPES.mapValues { (name, type) -> guard(errors, "getNetworkForType($name)") { networkForType(cm, type) } },
    )

// ConnectivityManager.getNetworkForType is hidden but public; a refusal surfaces
// in `errors` through the guard rather than as a fake null answer.
private fun networkForType(
    cm: ConnectivityManager,
    type: Int,
): Int? = (ConnectivityManager::class.java.getMethod("getNetworkForType", Integer.TYPE).invoke(cm, type) as? Network)?.netId()

private fun scanPhantoms(
    cm: ConnectivityManager,
    all: List<Int>,
    beyond: Int,
): List<Int> {
    val upper = (all.maxOrNull() ?: FIRST_NET_ID) + beyond
    return (FIRST_NET_ID..upper).filter { netId ->
        netId !in all && runCatching { cm.getNetworkCapabilities(networkForNetId(netId)) }.getOrNull() != null
    }
}

private class Recorder(
    private val registration: String,
    private val start: Long,
    private val sink: MutableList<CallbackEvent>,
) : ConnectivityManager.NetworkCallback() {
    private fun add(
        event: String,
        network: Network?,
        caps: CapabilityFacts? = null,
        lp: LinkFacts? = null,
        blocked: Int? = null,
    ) {
        sink += CallbackEvent(registration, event, network?.netId(), SystemClock.elapsedRealtime() - start, caps, lp, blocked)
    }

    override fun onAvailable(network: Network) = add("available", network)

    override fun onCapabilitiesChanged(
        network: Network,
        networkCapabilities: NetworkCapabilities,
    ) = add("capabilities", network, caps = capabilityFacts(networkCapabilities))

    override fun onLinkPropertiesChanged(
        network: Network,
        linkProperties: LinkProperties,
    ) = add("link_properties", network, lp = linkFacts(linkProperties))

    override fun onBlockedStatusChanged(
        network: Network,
        blocked: Boolean,
    ) = add("blocked", network, blocked = if (blocked) 1 else 0)

    override fun onLost(network: Network) = add("lost", network)

    override fun onUnavailable() = add("unavailable", null)
}

/** The listen every VPN-aware app can file: the builder's default NOT_VPN removed. */
internal fun listenAnyRequest(): NetworkRequest =
    NetworkRequest.Builder().removeCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN).build()

private fun listenVpnRequest(): NetworkRequest =
    NetworkRequest
        .Builder()
        .removeCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
        .addTransportType(NetworkCapabilities.TRANSPORT_VPN)
        .build()

private fun captureCallbacks(
    cm: ConnectivityManager,
    captureMs: Long,
    errors: MutableList<String>,
): List<CallbackEvent> {
    val sink = CopyOnWriteArrayList<CallbackEvent>()
    val start = SystemClock.elapsedRealtime()
    val registered = mutableListOf<ConnectivityManager.NetworkCallback>()

    fun register(
        name: String,
        block: (ConnectivityManager.NetworkCallback) -> Unit,
    ) {
        val cb = Recorder(name, start, sink)
        guard(errors, "register $name") {
            block(cb)
            registered += cb
        }
    }
    register("default") { cm.registerDefaultNetworkCallback(it) }
    register("listen_any") { cm.registerNetworkCallback(listenAnyRequest(), it) }
    register("listen_vpn") { cm.registerNetworkCallback(listenVpnRequest(), it) }
    try {
        Thread.sleep(captureMs)
    } finally {
        registered.forEach { runCatching { cm.unregisterNetworkCallback(it) } }
    }
    return sink.sortedBy { it.tMs }
}

private fun capturePendingIntent(
    context: Context,
    cm: ConnectivityManager,
    captureMs: Long,
): PendingIntentResult {
    val listenNetIds = CopyOnWriteArrayList<Int>()
    val requestNetIds = CopyOnWriteArrayList<Int>()
    val receiver = networkExtraReceiver(mapOf(LISTEN_INTENT_ACTION to listenNetIds, REQUEST_INTENT_ACTION to requestNetIds))
    val intents = mutableListOf<PendingIntent>()
    return try {
        registerNotExported(
            context,
            receiver,
            IntentFilter().apply {
                addAction(LISTEN_INTENT_ACTION)
                addAction(REQUEST_INTENT_ACTION)
            },
        )
        val listen = broadcastIntent(context, LISTEN_INTENT_ACTION, 0).also(intents::add)
        cm.registerNetworkCallback(listenAnyRequest(), listen)
        val request = broadcastIntent(context, REQUEST_INTENT_ACTION, 1).also(intents::add)
        cm.requestNetwork(internetAnyRequest(), request)
        Thread.sleep(captureMs)
        PendingIntentResult(registered = true, netIds = listenNetIds.distinct(), requestNetIds = requestNetIds.distinct())
    } catch (t: Throwable) {
        PendingIntentResult(
            registered = false,
            netIds = listenNetIds.distinct(),
            requestNetIds = requestNetIds.distinct(),
            error = "${t.javaClass.simpleName}: ${t.message}",
        )
    } finally {
        intents.forEach { pi ->
            runCatching { cm.unregisterNetworkCallback(pi) }
            runCatching { pi.cancel() }
        }
        runCatching { context.unregisterReceiver(receiver) }
    }
}

/** A request the VPN satisfies for a uid inside it: INTERNET, the builder's NOT_VPN removed. */
private fun internetAnyRequest(): NetworkRequest =
    NetworkRequest
        .Builder()
        .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
        .removeCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
        .build()

private fun networkExtraReceiver(sinks: Map<String, MutableList<Int>>): BroadcastReceiver =
    object : BroadcastReceiver() {
        override fun onReceive(
            context: Context,
            intent: Intent,
        ) {
            val netId = intent.getParcelableExtra<Network>(ConnectivityManager.EXTRA_NETWORK)?.netId() ?: return
            sinks[intent.action]?.add(netId)
        }
    }

private fun broadcastIntent(
    context: Context,
    action: String,
    requestCode: Int,
): PendingIntent = PendingIntent.getBroadcast(context, requestCode, Intent(action).setPackage(context.packageName), pendingIntentFlags())

private fun registerNotExported(
    context: Context,
    receiver: BroadcastReceiver,
    filter: IntentFilter,
) {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        context.registerReceiver(receiver, filter, Context.RECEIVER_NOT_EXPORTED)
    } else {
        context.registerReceiver(receiver, filter)
    }
}

// ConnectivityService fills EXTRA_NETWORK into the intent, so it must be mutable.
private fun pendingIntentFlags(): Int =
    PendingIntent.FLAG_UPDATE_CURRENT or
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) PendingIntent.FLAG_MUTABLE else 0
