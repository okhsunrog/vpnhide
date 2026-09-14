package dev.okhsunrog.vpnhide

/** Hiding deliberately matches more tunnels than the user-VPN diagnostic gate. */
internal data class VpnPresence(
    val interfaces: Set<String>,
    val frameworkInterfaces: Set<String>,
)

private val networkInterface = Regex("InterfaceName: ([A-Za-z0-9_.:-]+)")
private val networkTransports = Regex("Transports: ([A-Z_|]+)")
private val routeDevice = Regex("(?:^|\\s)dev ([A-Za-z0-9_.:-]+)(?:\\s|$)")

/**
 * Only current NetworkAgentInfo records are evidence: requests and historical
 * log entries also contain VPN capabilities, even when no VPN is connected.
 * Root supplies this dump so our own Binder hooks cannot sanitize its view.
 */
internal fun parseFrameworkVpnInterfaces(raw: String): Pair<Set<String>, Set<String>> {
    require(raw.lineSequence().any { it.trim() == "Current Networks:" }) {
        "Current Android networks unavailable"
    }
    val vpn = mutableSetOf<String>()
    val nonVpn = mutableSetOf<String>()
    raw
        .substringAfter("Current Networks:")
        .lineSequence()
        .takeWhile { it.isBlank() || it.first().isWhitespace() }
        .forEach { line ->
            if (!line.trimStart().startsWith("NetworkAgentInfo")) return@forEach
            if ("DISCONNECTED" in line || "CONNECTING" in line) return@forEach
            val iface = networkInterface.find(line)?.groupValues?.get(1) ?: return@forEach
            val transports =
                networkTransports
                    .find(line)
                    ?.groupValues
                    ?.get(1)
                    ?.split('|')
                    .orEmpty()
            when {
                "VPN" in transports -> vpn.add(iface)
                "NOT_VPN" in line && transports.isNotEmpty() -> nonVpn.add(iface)
            }
        }
    return vpn to nonVpn
}

/** Exclude local/broadcast bookkeeping: an address alone is not a routed tunnel. */
internal fun routedInterfaceNames(raw: String): Set<String> =
    raw
        .lineSequence()
        .mapNotNull { line ->
            val words = line.trim().split(Regex("\\s+"))
            if (words.firstOrNull() in setOf("local", "broadcast", "multicast", "unreachable", "blackhole", "prohibit")) {
                null
            } else {
                routeDevice.find(line)?.groupValues?.get(1)
            }
        }.toSet()

internal fun vpnPresenceFromSnapshot(sections: Map<String, String>): VpnPresence {
    val (frameworkVpn, nonVpn) = parseFrameworkVpnInterfaces(sections["vpn_networks"].orEmpty())
    require(listOf("vpn_routes4", "vpn_routes6").all { key -> sections[key].orEmpty().lineSequence().any { it == "probe_ok" } }) {
        "Kernel routing tables unavailable"
    }
    val routes =
        routedInterfaceNames(sections["vpn_routes4"].orEmpty()) +
            routedInterfaceNames(sections["vpn_routes6"].orEmpty())
    val nativeTunnels =
        parseVpnIfaceStates(sections["vpn_ifaces"].orEmpty())
            .filter { (name, state) -> name !in nonVpn && name in routes && state in setOf("up", "unknown") }
            .map { it.first }
    return VpnPresence(frameworkVpn + nativeTunnels, frameworkVpn)
}

/** Read-only route lookups: no packets are sent to these destinations. */
internal fun tunnelRouteProbeCommand(
    sections: Map<String, String>,
    interfaces: Set<String>,
    uid: Int,
): String {
    require(uid >= 0)
    val commands = mutableSetOf<String>()
    for (family in listOf(4, 6)) {
        sections["vpn_routes$family"].orEmpty().lineSequence().forEach { line ->
            if (routedInterfaceNames(line).none { it in interfaces }) return@forEach
            val prefix = line.trim().removePrefix("unicast ").substringBefore(' ')
            val address =
                if (prefix == "default") {
                    if (family == 4) "1.1.1.1" else "2606:4700:4700::1111"
                } else {
                    prefix.substringBefore('/')
                }
            if (address.matches(Regex("[0-9a-fA-F:.]+"))) {
                commands += "ip -$family route get '$address' uid $uid 2>/dev/null || echo probe_error"
            }
        }
    }
    return commands.joinToString("\n")
}

internal fun tunnelRouteProbeResult(
    raw: String,
    interfaces: Set<String>,
): Boolean? =
    when {
        routedInterfaceNames(raw).any { it in interfaces } -> true
        raw.isBlank() || raw.lineSequence().any { it.trim() == "probe_error" } -> null
        else -> false
    }
