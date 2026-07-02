package dev.okhsunrog.vpnhide

import android.util.Log

/**
 * App-process logcat facade. Info/debug/warn are gated by the runtime "debug
 * logging" flag ([enabled]); error always prints for crash analysis. The
 * gate-and-level policy lives in [GatedLogger]; this object only carries the app
 * sink and the flag's source (the canonical config snapshot).
 */
internal object VpnHideLog : GatedLogger() {
    /**
     * Load the effective debug flag from the current root snapshot so the first
     * log call after app start reflects the user's choice without waiting for
     * the settings UI to be opened.
     */
    fun init() {
        enabled = debugFromCanonicalSnapshot(RootSnapshotCache.snapshot.value)
    }

    fun setFromRootSnapshot(rootSnapshot: RootSnapshot?) {
        enabled = debugFromCanonicalSnapshot(rootSnapshot)
    }

    override fun emit(
        priority: Int,
        tag: String,
        msg: String,
        tr: Throwable?,
    ) {
        Log.println(priority, tag, if (tr == null) msg else "$msg\n${Log.getStackTraceString(tr)}")
    }

    fun i(
        tag: String,
        msg: String,
    ) = log(Log.INFO, tag, msg)

    fun d(
        tag: String,
        msg: String,
    ) = log(Log.DEBUG, tag, msg)

    fun w(
        tag: String,
        msg: String,
    ) = log(Log.WARN, tag, msg)

    fun w(
        tag: String,
        msg: String,
        tr: Throwable,
    ) = log(Log.WARN, tag, msg, tr)

    fun e(
        tag: String,
        msg: String,
    ) = log(Log.ERROR, tag, msg)

    fun e(
        tag: String,
        msg: String,
        tr: Throwable,
    ) = log(Log.ERROR, tag, msg, tr)
}
