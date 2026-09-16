package dev.okhsunrog.vpnhide.hook

import android.net.Network
import android.net.NetworkCapabilities
import de.robv.android.xposed.XposedHelpers

/**
 * Resolve the one physical network a target uid should see in place of its VPN,
 * from the live ConnectivityService — the single choice every hook that swaps a
 * VPN handle shares, so a handle and its facts always describe the same network.
 *
 * The choice follows AOSP's own per-uid resolution, and only that when it is
 * available:
 *
 *  1. the VPN's declared underlying network for this uid
 *     (`getVpnUnderlyingNetworks(uid)`) — an empty array means the VPN declared
 *     *no* default network, so there is no cover at all;
 *  2. else the uid's default network (`getDefaultNetworkForUid(uid)`, or the
 *     global `getDefaultNetwork()` only as a version fallback on ROMs without the
 *     per-uid method) — a per-uid default that came back null is honoured as
 *     "this uid has no default", not quietly replaced with the global one;
 *  3. the transport-score heuristic **only** when none of those methods were
 *     callable at all (an old or unusual ROM), never merely because a present
 *     method returned null.
 *
 * Each candidate must be a usable non-VPN network for the uid ([isUsableCover]);
 * when nothing is, the resolver returns null and the caller reports no active
 * network — exactly what a uid with no VPN sees in the same state. Every
 * reflective call is guarded: a missing method degrades to the next source and
 * never throws into a ConnectivityService thread.
 */
internal class VisibleNetworkResolver(
    private val rawCapabilities: (Any, Network) -> NetworkCapabilities?,
    private val heuristicOrder: (Any) -> List<Network>,
) {
    // Reflection outcome: a method that threw (absent) is distinct from one that
    // ran and returned a value (possibly null).
    private sealed interface Reflected<out T> {
        data object Absent : Reflected<Nothing>

        data class Present<T>(
            val value: T,
        ) : Reflected<T>
    }

    /** The cover network for [uid], or null when the platform resolves none. */
    fun coverFor(
        cs: Any,
        uid: Int,
    ): Network? {
        var trustedResolution = false

        when (val underlying = vpnUnderlyingNetworks(cs, uid)) {
            is Reflected.Present -> {
                trustedResolution = true
                val networks = underlying.value
                if (networks != null) {
                    // Empty = the VPN declared no default network: no cover exists.
                    if (networks.isEmpty()) return logNone(uid, "underlying-declared-none")
                    usableCover(cs, uid, "underlying", networks.first())?.let { return logCover(uid, "underlying", it) }
                }
            }

            Reflected.Absent -> {
                Unit
            }
        }

        when (val perUid = reflectNetwork(cs, "getDefaultNetworkForUid", uid)) {
            is Reflected.Present -> {
                trustedResolution = true
                // A per-uid default of null means this uid has no default network —
                // do not substitute the global default it cannot use.
                perUid.value?.let { usableCover(cs, uid, "default", it)?.let { c -> return logCover(uid, "default", c) } }
            }

            Reflected.Absent -> {
                when (val global = reflectNetwork(cs, "getDefaultNetwork")) {
                    is Reflected.Present -> {
                        trustedResolution = true
                        global.value?.let { usableCover(cs, uid, "default", it)?.let { c -> return logCover(uid, "default", c) } }
                    }

                    Reflected.Absent -> {
                        Unit
                    }
                }
            }
        }

        // The heuristic is a last resort for a ROM without the AOSP methods, never a
        // rescue when a present method legitimately resolved to null/blocked.
        if (!trustedResolution) {
            heuristicOrder(cs).forEach { usableCover(cs, uid, "heuristic", it)?.let { return logCover(uid, "heuristic", it) } }
        }
        return logNone(uid, if (trustedResolution) "no-usable-cover" else "no-aosp-methods")
    }

    private fun usableCover(
        cs: Any,
        uid: Int,
        source: String,
        network: Network,
    ): Network? {
        val caps = rawCapabilities(cs, network)
        val candidate =
            CoverCandidate(
                source = source,
                netId = network.netId() ?: -1,
                isVpn = caps?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) ?: true,
                hasInternet = caps?.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) ?: false,
                notRestricted = caps?.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_RESTRICTED) ?: false,
                blocked = caps != null && isBlockedForUid(cs, caps, uid),
            )
        return if (isUsableCover(candidate)) network else null
    }

    // getVpnUnderlyingNetworks(int): Present(array) when the method ran (the array
    // may be empty = "no default network", or its elements the underlying nets),
    // Present(null) when no VPN applies to the uid, Absent when the method threw.
    private fun vpnUnderlyingNetworks(
        cs: Any,
        uid: Int,
    ): Reflected<Array<Network>?> =
        runCatching {
            val value =
                (XposedHelpers.callMethod(cs, "getVpnUnderlyingNetworks", uid) as? Array<*>)
                    ?.filterIsInstance<Network>()
                    ?.toTypedArray()
            Reflected.Present(value)
        }.getOrDefault(Reflected.Absent)

    // A ConnectivityService method returning a NetworkAgentInfo, reduced to its
    // Network. Present(network-or-null) when it ran, Absent when it threw.
    private fun reflectNetwork(
        cs: Any,
        method: String,
        vararg args: Any?,
    ): Reflected<Network?> =
        runCatching {
            val nai = XposedHelpers.callMethod(cs, method, *args)
            Reflected.Present(nai?.let { XposedHelpers.getObjectField(it, "network") as? Network })
        }.getOrDefault(Reflected.Absent)

    // isNetworkWithCapabilitiesBlocked(nc, uid, false) (12+); false when the method
    // is absent — see [CoverCandidate.blocked] for why undeterminable means usable.
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

    private fun logCover(
        uid: Int,
        source: String,
        network: Network,
    ): Network {
        HookLog.i("VpnHide: cover for uid=$uid via $source = net${network.netId()}")
        return network
    }

    private fun logNone(
        uid: Int,
        reason: String,
    ): Network? {
        HookLog.i("VpnHide: no cover for uid=$uid ($reason)")
        return null
    }

    private fun Network.netId(): Int? = toString().toIntOrNull()
}
