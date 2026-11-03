package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.generated.HookIds

internal fun parseLsposedStateMetadata(raw: String): Map<String, String> = uniffi.vpnhide_core_ffi.protocolMetadata(raw)

/**
 * Are the LSPosed hooks live *this boot*? True when the state file reports the
 * LSPosed backend and its stamped boot_id matches the current one — the same test
 * [resolveLsposedState] applies for [LsposedState.Active]. Extracted so the
 * dashboard and the debug export derive "Java layer active" from one place.
 */
internal fun lsposedHooksActiveThisBoot(
    stateRaw: String,
    currentBootId: String,
): Boolean {
    val hookBootId = parseLsposedStateMetadata(stateRaw)[LsposedStateMetadata.BOOT_ID]
    return ProtocolReader.parseStatus(stateRaw)?.backendId == HookIds.Backend.LSPOSED &&
        hookBootId != null &&
        hookBootId == currentBootId.trim()
}
