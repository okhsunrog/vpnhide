package dev.okhsunrog.vpnhide.diagnostics

/** App-side values, captured after Binder unmarshalling rather than inside the hook. */
internal data class LegacyVpnInfoSnapshot(
    val type: Int,
    val state: String,
    val detailedState: String,
    val available: Boolean,
)

internal data class LegacyVpnInfoAssessment(
    val clean: Boolean?,
    val detail: String,
)

internal fun assessLegacyVpnInfo(
    direct: LegacyVpnInfoSnapshot?,
    all: List<LegacyVpnInfoSnapshot>,
): LegacyVpnInfoAssessment {
    val vpnEntries = all.filter { it.type == 17 }
    val observed = "direct=$direct; enumerated VPN entries=$vpnEntries"
    if (direct != null && direct.type != 17) {
        return LegacyVpnInfoAssessment(false, "TYPE_VPN returned another network type; $observed")
    }
    val observedVpn = listOfNotNull(direct) + vpnEntries
    if (observedVpn.any { it.state != "DISCONNECTED" || it.detailedState !in setOf("DISCONNECTED", "BLOCKED") }) {
        return LegacyVpnInfoAssessment(false, "VPN state is not inactive; $observed")
    }
    if (vpnEntries.size > 1) {
        return LegacyVpnInfoAssessment(false, "Duplicate legacy VPN entries; $observed")
    }
    if (direct == null) {
        // null is allowed by Android. It cannot establish correct disconnected-type
        // semantics, and the old nulling hook must no longer receive a green pass.
        return LegacyVpnInfoAssessment(null, "TYPE_VPN returned null; legacy semantics not verified; $observed")
    }
    if (vpnEntries.isEmpty()) {
        return LegacyVpnInfoAssessment(false, "TYPE_VPN missing from getAllNetworkInfo(); $observed")
    }
    // Separate IPC calls can straddle a UID-policy change: DISCONNECTED vs BLOCKED
    // is not itself a leak. Both must independently describe an inactive VPN type.
    return LegacyVpnInfoAssessment(true, "Inactive VPN type preserved across legacy queries; $observed")
}
