package dev.okhsunrog.vpnhide.diagnostics

/**
 * Which VPN networks the transport watcher has already accounted for, keyed by
 * `Network.networkHandle`. Registering a network callback replays the current
 * state (available plus its first capabilities) for every existing VPN network;
 * that replay is knowledge, not a transition, and must not invalidate a routing
 * observation that already reflects it.
 */
internal data class VpnTransportKnowledge(
    val known: Set<Long> = emptySet(),
    val capabilitiesSeen: Set<Long> = emptySet(),
)

internal enum class VpnTransportEvent { Available, Lost, CapabilitiesChanged }

internal data class VpnTransportDecision(
    val knowledge: VpnTransportKnowledge,
    /** True when the event is a change relative to what was already known. */
    val transition: Boolean,
)

/**
 * A network becoming available is a transition only when it was not known; the
 * first capabilities delivery of a network is part of its arrival, later ones are
 * changes; a loss is always a change.
 */
internal fun reduceVpnTransport(
    knowledge: VpnTransportKnowledge,
    event: VpnTransportEvent,
    handle: Long,
): VpnTransportDecision =
    when (event) {
        VpnTransportEvent.Available -> {
            VpnTransportDecision(knowledge.copy(known = knowledge.known + handle), transition = handle !in knowledge.known)
        }

        VpnTransportEvent.Lost -> {
            VpnTransportDecision(
                knowledge.copy(known = knowledge.known - handle, capabilitiesSeen = knowledge.capabilitiesSeen - handle),
                transition = true,
            )
        }

        VpnTransportEvent.CapabilitiesChanged -> {
            VpnTransportDecision(
                knowledge.copy(known = knowledge.known + handle, capabilitiesSeen = knowledge.capabilitiesSeen + handle),
                transition = handle in knowledge.capabilitiesSeen,
            )
        }
    }
