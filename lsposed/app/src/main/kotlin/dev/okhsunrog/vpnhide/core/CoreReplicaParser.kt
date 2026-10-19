package dev.okhsunrog.vpnhide.core

import org.json.JSONArray
import org.json.JSONObject

/** Protocol-v4 observation freshness. Last-good must never be presented as current. */
enum class CoreFreshness {
    Never,
    Current,
    Unknown,
    ;

    companion object {
        fun parse(raw: String?): CoreFreshness =
            when (raw?.lowercase()) {
                "current" -> Current
                "unknown" -> Unknown
                "never", null, "" -> Never
                else -> Unknown
            }
    }
}

data class CoreRoutingView(
    val freshness: CoreFreshness,
    val generation: Long,
    val vpnInterfaces: List<String>,
    val lastGoodVpnInterfaces: List<String>,
    val netlinkCandidates: List<String>,
) {
    /** Interfaces safe to treat as authoritative VPN (empty when not Current). */
    fun authoritativeVpnInterfaces(): List<String> =
        if (freshness == CoreFreshness.Current) vpnInterfaces else emptyList()

    /** Display-only fallback when freshness is Unknown — never claim current. */
    fun displayVpnInterfaces(): List<String> =
        when (freshness) {
            CoreFreshness.Current -> vpnInterfaces
            CoreFreshness.Unknown -> lastGoodVpnInterfaces
            CoreFreshness.Never -> emptyList()
        }
}

data class CoreReplicaView(
    val configRevision: Long,
    val hasConfig: Boolean,
    val operationsInFlight: List<Long>,
    val routing: CoreRoutingView,
)

/**
 * Parses core `StateSnapshot` JSON (serde_json snake_case) without depending on UniFFI.
 * Host unit tests and the production gateway share this parser.
 */
object CoreReplicaParser {
    fun parseSnapshot(json: String): CoreReplicaView {
        val root = JSONObject(json)
        val routing = root.optJSONObject("routing") ?: JSONObject()
        return CoreReplicaView(
            configRevision = root.optLong("config_revision", 0),
            hasConfig = !root.isNull("config"),
            operationsInFlight = longList(root.optJSONArray("operations_in_flight")),
            routing =
                CoreRoutingView(
                    freshness = CoreFreshness.parse(routing.optString("freshness", "never")),
                    generation = routing.optLong("generation", 0),
                    vpnInterfaces = stringList(routing.optJSONArray("vpn_interfaces")),
                    lastGoodVpnInterfaces = stringList(routing.optJSONArray("last_good_vpn_interfaces")),
                    netlinkCandidates = stringList(routing.optJSONArray("netlink_candidates")),
                ),
        )
    }

    private fun stringList(arr: JSONArray?): List<String> {
        if (arr == null) return emptyList()
        return buildList {
            for (i in 0 until arr.length()) {
                val v = arr.optString(i, null) ?: continue
                if (v.isNotEmpty()) add(v)
            }
        }
    }

    private fun longList(arr: JSONArray?): List<Long> {
        if (arr == null) return emptyList()
        // Wire may be `[{"0":123}]` (newtype) or bare numbers depending on serde; accept both.
        return buildList {
            for (i in 0 until arr.length()) {
                when (val v = arr.opt(i)) {
                    is Number -> add(v.toLong())
                    is JSONArray -> if (v.length() > 0) add(v.optLong(0))
                    is JSONObject -> {
                        val keys = v.keys()
                        if (keys.hasNext()) {
                            add(v.optLong(keys.next(), 0))
                        }
                    }
                }
            }
        }
    }
}
