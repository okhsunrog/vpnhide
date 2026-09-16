package dev.okhsunrog.vpnhide.hook

/**
 * One candidate for the cover network shown to a target uid in place of the VPN,
 * reduced to the facts the choice turns on. Built by reflection in
 * [VisibleNetworkResolver]; whether it may be used is this pure function so the
 * policy is unit-tested without a live ConnectivityService.
 */
internal data class CoverCandidate(
    // "underlying" (the VPN's declared underlying network for this uid), "default"
    // (the uid's default network) or "heuristic" (best non-VPN by transport score).
    val source: String,
    val netId: Int,
    val isVpn: Boolean,
    val hasInternet: Boolean,
    val notRestricted: Boolean,
    // The platform's own per-uid blocked verdict (data saver, background chain,
    // lockdown). Undeterminable (the method is absent) resolves to false — a
    // network AOSP itself handed back as the underlying/default is trusted rather
    // than withheld, which is what keeps a foreground online app from being told
    // it is offline.
    val blocked: Boolean,
)

/**
 * A cover must be a real, usable non-VPN network the uid could actually be on: not
 * the VPN, reachable (INTERNET), not restricted away from an ordinary app, and not
 * blocked for this uid. A transient underlying with no capabilities yet, or a
 * blocked one, is rejected so the caller can fall through — and, when nothing is
 * usable, report no active network exactly as a uid with no VPN would see.
 */
internal fun isUsableCover(candidate: CoverCandidate): Boolean =
    !candidate.isVpn && candidate.hasInternet && candidate.notRestricted && !candidate.blocked
