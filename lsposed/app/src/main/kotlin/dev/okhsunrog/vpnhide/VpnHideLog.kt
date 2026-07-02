package dev.okhsunrog.vpnhide

import android.util.Log

/**
 * Log wrapper gated by a runtime "debug logging" flag so users can
 * silence logcat output from the app process.
 *
 * Only i/d/w are gated. Error-level logs always pass through for crash
 * analysis.
 *
 * [enabled] is a volatile plain boolean rather than a synchronized
 * getter: i/d/w are on enough hot paths (Dashboard reload, suExec
 * wrappers, update checks) that we can't afford a monitor per call.
 * Worst case, a toggle flip takes one extra log line to take effect.
 */
internal object VpnHideLog {
    @Volatile
    var enabled: Boolean = false

    /**
     * Load the persisted preference into [enabled] so the first log
     * call after app start reflects the user's choice without waiting
     * for the settings UI to be opened.
     */
    fun init() {
        enabled = debugFromCanonicalSnapshot(RootSnapshotCache.snapshot.value)
    }

    fun setFromRootSnapshot(rootSnapshot: RootSnapshot?) {
        enabled = debugFromCanonicalSnapshot(rootSnapshot)
    }

    fun i(
        tag: String,
        msg: String,
    ) {
        if (enabled) Log.i(tag, msg)
    }

    fun d(
        tag: String,
        msg: String,
    ) {
        if (enabled) Log.d(tag, msg)
    }

    fun w(
        tag: String,
        msg: String,
    ) {
        if (enabled) Log.w(tag, msg)
    }

    fun w(
        tag: String,
        msg: String,
        tr: Throwable,
    ) {
        if (enabled) Log.w(tag, msg, tr)
    }

    fun e(
        tag: String,
        msg: String,
    ) {
        Log.e(tag, msg)
    }

    fun e(
        tag: String,
        msg: String,
        tr: Throwable,
    ) {
        Log.e(tag, msg, tr)
    }
}
