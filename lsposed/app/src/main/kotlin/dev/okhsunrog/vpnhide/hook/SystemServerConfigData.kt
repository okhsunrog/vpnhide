package dev.okhsunrog.vpnhide.hook

import dev.okhsunrog.vpnhide.CanonicalConfig
import dev.okhsunrog.vpnhide.LsposedJavaHookEntries
import dev.okhsunrog.vpnhide.hookSelectionMask

/**
 * Binder identifies the caller by UID, not package. Each enabled package adds
 * hooks for the whole shared appId, across profiles; a disabled role adds none
 * and cannot veto another package's selection. Preserve the per-package intent
 * in canonical JSON and combine it only at this UID execution boundary.
 */
internal fun javaHookMasksByAppId(
    canonical: CanonicalConfig,
    packageAppIds: Map<String, Int>,
): Map<Int, Long> {
    val masks = mutableMapOf<Int, Long>()
    for ((pkg, app) in canonical.apps) {
        val appId = packageAppIds[pkg] ?: continue
        val mask = hookSelectionMask(enabled = app.java, hooks = app.javaHooks, entries = LsposedJavaHookEntries)
        if (mask != 0L) masks[appId] = (masks[appId] ?: 0L) or mask
    }
    return masks
}
