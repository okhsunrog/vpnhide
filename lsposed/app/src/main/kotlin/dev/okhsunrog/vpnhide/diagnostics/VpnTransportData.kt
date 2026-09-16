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

/**
 * What the default-network callback has delivered so far. Registering it replays
 * the current default network as one `onAvailable` when a default exists; that
 * replay is knowledge, not a transition. Network handles are deliberately not
 * compared: for this app's own uid the hooks rewrite a VPN network into its
 * underlying network, so the handle can look unchanged across a real switch.
 */
internal data class DefaultNetworkKnowledge(
    /** True once the registration replay has been consumed, or when none was expected. */
    val replayed: Boolean,
)

/**
 * ConnectivityService dispatches `onAvailable` to a default-network callback
 * only when the network satisfying the default request changes, so every
 * delivery after the replay is a switch (VPN up, VPN down, Wi-Fi to mobile);
 * a loss with no replacement is one as well. Capability changes are frequent
 * and, for this app, sanitized, so they never count.
 */
internal fun reduceDefaultNetwork(
    knowledge: DefaultNetworkKnowledge,
    event: VpnTransportEvent,
): Pair<DefaultNetworkKnowledge, Boolean> =
    when (event) {
        VpnTransportEvent.Available -> DefaultNetworkKnowledge(replayed = true) to knowledge.replayed
        VpnTransportEvent.Lost -> DefaultNetworkKnowledge(replayed = true) to true
        VpnTransportEvent.CapabilitiesChanged -> knowledge to false
    }
