package dev.okhsunrog.vpnhide

import android.content.Context

/**
 * Persisted "debug logging" preference and its propagation. Debug is stored in
 * the canonical JSON and folded into the native control-config wire by the
 * activator, so every backend observes one source of truth. The sinks are:
 *
 *  - App Kotlin code → [VpnHideLog.enabled] (volatile).
 *  - system_server LSPosed hooks → `/data/system/vpnhide_config.json`, watched
 *    by [HookLog].
 *  - native backends → [ConfigChannels.reconcileCommand], which runs the
 *    installed activator after the JSON update.
 */
private const val PREFS_NAME = "vpnhide_prefs"
private const val KEY_DEBUG_LOGGING = "debug_logging"

/** Default is OFF — stealth-first matches the project's anti-detection stance. */
internal fun isEnabledInPrefs(context: Context): Boolean =
    context
        .getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
        .getBoolean(KEY_DEBUG_LOGGING, false)

/**
 * Flip the persisted preference and propagate it to every sink. Runs
 * SU commands, so callers should invoke from a background dispatcher.
 * Use this for the user-facing toggle in Diagnostics.
 */
internal fun setDebugLoggingEnabled(
    context: Context,
    enabled: Boolean,
) {
    storeDebugLoggingPreference(context, enabled)
    writeDebugFlagFiles(enabled)
    RootSnapshotCache.invalidate()
    DashboardCache.invalidate()
}

internal fun storeDebugLoggingPreference(
    context: Context,
    enabled: Boolean,
) {
    context
        .getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
        .edit()
        .putBoolean(KEY_DEBUG_LOGGING, enabled)
        .apply()
    VpnHideLog.enabled = enabled
}

/**
 * Push [enabled] to the runtime sinks only, without touching
 * SharedPreferences. Used by diagnostic capture paths (Collect debug
 * log button + [LogcatRecorder]) that temporarily force-enable logging
 * for the duration of a capture and then restore the user's persisted
 * choice — without this, the user-facing toggle would visually flip
 * under the user while they collected a bug report.
 */
internal fun applyDebugLoggingRuntime(enabled: Boolean) {
    VpnHideLog.enabled = enabled
    writeDebugFlagFiles(enabled)
}

/**
 * Startup safety-net: re-propagate the persisted debug flag to the on-disk
 * config + native sinks only if they've drifted from [enabled]. This runs on
 * every cold start; without the drift check it would rewrite the canonical
 * config (and re-run the native activator) with byte-identical content on every
 * launch. Capture paths (LogcatRecorder / debug export) do not go through here —
 * they already gate their own [applyDebugLoggingRuntime] calls on
 * [VpnHideLog.enabled] and must always write both directions.
 *
 * When the snapshot isn't loaded yet the flag is treated as drifted, so the
 * safety-net still fires once.
 */
internal fun reapplyDebugLoggingIfDrifted(enabled: Boolean) {
    val onDiskDebug =
        TargetsCache.snapshot.value
            ?.canonicalConfig
            ?.debug
    if (onDiskDebug == enabled) {
        VpnHideLog.enabled = enabled
        return
    }
    applyDebugLoggingRuntime(enabled)
}

private fun writeDebugFlagFiles(enabled: Boolean) {
    val parts = mutableListOf<String>()

    // Re-emit the runtime config with the new flag so a running native backend
    // picks it up. Needs the current targets; if the snapshot isn't loaded yet,
    // the next startup reconcile or Save carries the flag into the channels.
    TargetsCache.snapshot.value?.let { snap ->
        val canonical = buildCanonicalConfigFromTargetsSnapshot(snap, debug = enabled)
        parts += buildCanonicalConfigWriteCommand(canonical)
        parts += ConfigChannels.reconcileCommand()
    }
    parts += "true"

    // Batched into one su invocation to keep the toggle UI snappy — each
    // round-trip is ~50-100ms. Channels whose component isn't installed/loaded
    // are skipped by the guards inside the config-write parts.
    suExec(parts.joinToString(" ; "))
}
