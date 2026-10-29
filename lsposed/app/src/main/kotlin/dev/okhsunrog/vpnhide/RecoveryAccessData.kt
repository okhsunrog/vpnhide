package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.core.CoreRootEffect
import uniffi.vpnhide_core_ffi.CoreRecoveryAccess

/** Null refusals mean allowed; absent publication never grants recovery. */
internal fun recoveryAllowed(
    access: CoreRecoveryAccess?,
    mutation: CanonicalMutation,
): Boolean {
    if (access == null) return false
    return when (mutation.coupledEffects.singleOrNull()) {
        is CoreRootEffect.SetSuperkey -> access.setSuperkey == null
        CoreRootEffect.ClearSuperkey -> access.clearSuperkey == null
        CoreRootEffect.FullReset -> access.fullReset == null
        CoreRootEffect.DeleteLegacyConfig -> mutation.edits.isEmpty() && !mutation.protectAllDrafts && access.cleanup == null
        null -> mutation.forceActivation && access.repair == null
    }
}
