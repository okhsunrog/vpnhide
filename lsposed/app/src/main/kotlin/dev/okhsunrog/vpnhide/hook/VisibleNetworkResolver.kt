package dev.okhsunrog.vpnhide.hook

import android.net.Network
import android.net.NetworkCapabilities
import de.robv.android.xposed.XposedHelpers

/**
 * Resolve the one physical network a target uid should see in place of its VPN,
 * from the live ConnectivityService — the single choice every hook that swaps a
 * VPN handle shares, so a handle and its facts always describe the same network.
 *
 * The choice follows AOSP's own per-uid resolution rather than a free-standing
 * heuristic: the network *behind* the VPN for this uid is the VPN's declared
 * underlying network (`getVpnUnderlyingNetworks(uid)`), else the uid's default
 * network (`getDefaultNetworkForUid(uid)` / `getDefaultNetwork()`), each checked
 * for INTERNET, NOT_RESTRICTED and not-blocked-for-uid. The transport-score
 * heuristic remains only as a last resort for a ROM where those private methods
 * are absent. When the VPN explicitly declares no underlying network, there is no
 * cover and the caller must report no active network — never invent a Wi-Fi.
 *
 * Every reflective call is guarded; a missing method degrades to the next source,
 * never throws into a ConnectivityService thread.
 */
internal class VisibleNetworkResolver(
    private val rawCapabilities: (Any, Network) -> NetworkCapabilities?,
    private val heuristicOrder: (Any) -> List<Network>,
) {
    /** The cover network for [uid], or null when the platform resolves none. */
    fun coverFor(
        cs: Any,
        uid: Int,
    ): Network? {
        val underlying = vpnUnderlyingNetworks(cs, uid)
        // Empty (not null) means the VPN published "no default network": honour it.
        if (underlying != null && underlying.isEmpty()) return null

        val ordered = orderedCandidates(cs, uid, underlying?.firstOrNull())
        val candidates = ordered.map { (source, network) -> candidateFor(cs, uid, source, network) }
        val index = selectCoverIndex(candidates, underlyingDeclaredEmpty = false) ?: return null
        val chosen = candidates[index]
        // Which source resolved the cover, so a device (and a future ROM where the
        // AOSP methods are absent and it falls back to "heuristic") is provable.
        HookLog.i("VpnHide: cover for uid=$uid via ${chosen.source} = net${chosen.netId}")
        return ordered[index].second
    }

    private fun orderedCandidates(
        cs: Any,
        uid: Int,
        underlying: Network?,
    ): List<Pair<String, Network>> {
        val ordered = LinkedHashMap<Int, Pair<String, Network>>()

        fun offer(
            source: String,
            network: Network?,
        ) {
            val netId = network?.netId() ?: return
            ordered.getOrPut(netId) { source to network }
        }
        offer("underlying", underlying)
        offer("default", defaultNetworkForUid(cs, uid))
        heuristicOrder(cs).forEach { offer("heuristic", it) }
        return ordered.values.toList()
    }

    private fun candidateFor(
        cs: Any,
        uid: Int,
        source: String,
        network: Network,
    ): CoverCandidate {
        val caps = rawCapabilities(cs, network)
        return CoverCandidate(
            source = source,
            netId = network.netId() ?: -1,
            isVpn = caps?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) ?: true,
            hasInternet = caps?.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) ?: false,
            notRestricted = caps?.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_RESTRICTED) ?: false,
            blocked = caps != null && isBlockedForUid(cs, caps, uid),
        )
    }

    /**
     * `getVpnUnderlyingNetworks(int)` — null when no VPN applies to the uid or the
     * method is absent (fall through to the default), an empty array when the VPN
     * declared no default network (no cover at all).
     */
    private fun vpnUnderlyingNetworks(
        cs: Any,
        uid: Int,
    ): Array<Network>? =
        runCatching {
            (XposedHelpers.callMethod(cs, "getVpnUnderlyingNetworks", uid) as? Array<*>)
                ?.filterIsInstance<Network>()
                ?.toTypedArray()
        }.getOrNull()

    /** The uid's default network, as a [Network]: `getDefaultNetworkForUid(uid)` (12+) else `getDefaultNetwork()`. */
    private fun defaultNetworkForUid(
        cs: Any,
        uid: Int,
    ): Network? {
        val nai =
            runCatching { XposedHelpers.callMethod(cs, "getDefaultNetworkForUid", uid) }.getOrNull()
                ?: runCatching { XposedHelpers.callMethod(cs, "getDefaultNetwork") }.getOrNull()
        return nai?.let { runCatching { XposedHelpers.getObjectField(it, "network") as? Network }.getOrNull() }
    }

    /**
     * `isNetworkWithCapabilitiesBlocked(nc, uid, false)` (12+); false when the
     * method is absent — a best-effort filter, not a correctness dependency.
     */
    private fun isBlockedForUid(
        cs: Any,
        caps: NetworkCapabilities,
        uid: Int,
    ): Boolean =
        runCatching {
            XposedHelpers.callMethod(
                cs,
                "isNetworkWithCapabilitiesBlocked",
                arrayOf(NetworkCapabilities::class.java, Integer.TYPE, java.lang.Boolean.TYPE),
                caps,
                uid,
                false,
            ) as? Boolean
        }.getOrNull() ?: false

    private fun Network.netId(): Int? = toString().toIntOrNull()
}
