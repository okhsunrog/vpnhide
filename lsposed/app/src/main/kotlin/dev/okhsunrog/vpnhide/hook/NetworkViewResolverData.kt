package dev.okhsunrog.vpnhide.hook

/**
 * One candidate for the cover network shown to a target uid in place of the VPN,
 * reduced to the facts the choice turns on. Built by reflection in
 * [VisibleNetworkResolver]; the choice itself is this pure function so the policy
 * is unit-tested without a live ConnectivityService.
 */
internal data class CoverCandidate(
    // "underlying" (the VPN's declared underlying network for this uid), "default"
    // (the uid's default network) or "heuristic" (best non-VPN by transport score).
    val source: String,
    val netId: Int,
    val isVpn: Boolean,
    val hasInternet: Boolean,
    val notRestricted: Boolean,
    // Best-effort: false when the platform's per-uid blocked check is unavailable.
    val blocked: Boolean,
)

private fun CoverCandidate.usableCover(): Boolean = !isVpn && hasInternet && notRestricted && !blocked

/**
 * Pick the cover network the way AOSP resolves the network *behind* a VPN for a
 * uid: the VPN's declared underlying network first, then the uid's default
 * network, then the transport-score heuristic as a last resort. [candidates] is
 * already in that priority order, so the first usable one wins.
 *
 * [underlyingDeclaredEmpty] means the VPN explicitly published an empty underlying
 * array — "no default network" — so there is genuinely nothing to show and the
 * caller must leave the app with no active network rather than inventing one from
 * the heuristic. Distinct from the VPN declaring nothing (null), where the uid's
 * real default is the right cover.
 */
internal fun selectCoverIndex(
    candidates: List<CoverCandidate>,
    underlyingDeclaredEmpty: Boolean,
): Int? {
    if (underlyingDeclaredEmpty) return null
    val index = candidates.indexOfFirst { it.usableCover() }
    return index.takeIf { it >= 0 }
}
