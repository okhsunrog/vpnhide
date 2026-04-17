package dev.okhsunrog.vpnhide

import android.content.Context

/**
 * Persisted "debug logging" preference and its propagation to the
 * out-of-process logging sinks:
 *
 *  - App Kotlin code → [VpnHideLog.enabled] (volatile)
 *  - system_server LSPosed hooks → [SS_DEBUG_LOGGING_FILE] (inotify-
 *    watched; flip takes effect immediately for already-running apps)
 *  - Zygisk module → [ZYGISK_DEBUG_LOGGING_FILE] (read when the module
 *    is injected into a forked app, so target apps need to be restarted
 *    before a flip takes effect for them — identical to targets.txt)
 *
 * kmod is intentionally left out: in steady state it emits no logs at
 * all, and its verbose logging already has a separate /proc toggle
 * flipped on demand by Diagnostics.
 */
private const val PREFS_NAME = "vpnhide_prefs"
private const val KEY_DEBUG_LOGGING = "debug_logging"

internal const val SS_DEBUG_LOGGING_FILE = "/data/system/vpnhide_debug_logging"
internal const val ZYGISK_DEBUG_LOGGING_FILE = "/data/adb/modules/vpnhide_zygisk/debug_logging"

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
    context
        .getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
        .edit()
        .putBoolean(KEY_DEBUG_LOGGING, enabled)
        .apply()
    applyDebugLoggingRuntime(enabled)
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

private fun writeDebugFlagFiles(enabled: Boolean) {
    val value = if (enabled) "1" else "0"
    // system_server hook file: written to /data/system, labelled
    // system_data_file so system_server (and nothing else) can read it.
    // `|| true` so the command succeeds even if chcon isn't available
    // on this device — the file still ends up with default labels that
    // system_server can read.
    suExec(
        "echo '$value' > $SS_DEBUG_LOGGING_FILE" +
            " && chmod 644 $SS_DEBUG_LOGGING_FILE" +
            " && chcon u:object_r:system_data_file:s0 $SS_DEBUG_LOGGING_FILE 2>/dev/null; true",
    )
    // Zygisk module file: only written if the module is actually installed.
    // The persistent dir /data/adb/vpnhide_zygisk/ isn't needed here — the
    // flag is a preference, not configuration that should survive a fresh
    // module install.
    suExec(
        "[ -d $ZYGISK_MODULE_DIR ] &&" +
            " echo '$value' > $ZYGISK_DEBUG_LOGGING_FILE" +
            " && chmod 644 $ZYGISK_DEBUG_LOGGING_FILE 2>/dev/null; true",
    )
}
