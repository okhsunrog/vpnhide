package dev.okhsunrog.vpnhide

private const val TAG = LogTags.DEBUG_CONFIG

private fun canonicalFromSnapshot(snapshot: RootSnapshot?): CanonicalConfig? =
    snapshot
        ?.let { parseTargetsSnapshot(it).canonicalConfig }

/**
 * Canonical config is the only source of truth for debug logging state.
 *
 * Before a root snapshot exists this is `false`.
 */
internal fun debugFromCanonicalSnapshot(rootSnapshot: RootSnapshot?): Boolean =
    canonicalFromSnapshot(rootSnapshot)
        ?.debug
        ?: false

/**
 * Set debug on the canonical JSON and propagate it through the native activator.
 * SU commands may fail on unusual root states, so callers should run this from
 * an IO dispatcher and still tolerate a temporary mismatch if needed.
 */
internal suspend fun setDebugLoggingEnabled(enabled: Boolean) {
    // Re-read canonical from disk rather than the cached StateFlow: a prior
    // toggle in the same Settings visit invalidates RootSnapshotCache, and
    // nothing on the Settings screen repopulates it, so `.snapshot.value` would
    // be null and this write would silently no-op.
    val snapshot =
        runCatching { RootSnapshotCache.getOrLoad() }.getOrNull()
            ?: RootSnapshotCache.snapshot.value
    val canonical =
        canonicalFromSnapshot(snapshot)
            ?.copy(
                debug = enabled,
                debugSwitch = enabled,
            )
            ?: return

    val command =
        listOf(
            buildCanonicalConfigWriteCommand(canonical),
            ConfigChannels.reconcileCommand(),
        ).joinToString(" ; ")
    val (exit, out) = suExec(command)
    if (exit != 0) {
        VpnHideLog.e(TAG, "write canonical debug command failed: exit=$exit: ${out.trim()}")
    }

    VpnHideLog.enabled = enabled
    RootSnapshotCache.invalidate()
    TargetsCache.invalidate()
    DashboardCache.invalidate()
}
