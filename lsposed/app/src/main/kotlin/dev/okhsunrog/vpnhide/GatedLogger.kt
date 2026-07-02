package dev.okhsunrog.vpnhide

import android.util.Log

/**
 * Shared gating core for the project's two logcat facades — [VpnHideLog] (app
 * process) and [HookLog] (LSPosed hooks in `system_server`). Both apply the same
 * policy — gate info/debug/warn behind a runtime "debug logging" flag, always
 * emit errors — but they read that flag from different sources (the canonical
 * snapshot vs. an inotify watcher) because they run in different processes. This
 * base owns the single copy of the flag and the gate-and-level policy; each
 * subclass only supplies the concrete sink and its own public method shape.
 */
internal abstract class GatedLogger {
    /**
     * Effective "debug logging" flag. A `@Volatile` plain boolean rather than a
     * synchronized getter: info/debug/warn sit on hot paths (per-request hooks,
     * Dashboard reload, suExec wrappers) and can't afford a monitor per call.
     * Worst case a toggle flip costs one extra log line to take effect.
     */
    @Volatile
    var enabled: Boolean = false

    /** Write one line to the concrete sink(s). [tr], when present, is appended. */
    protected abstract fun emit(
        priority: Int,
        tag: String,
        msg: String,
        tr: Throwable?,
    )

    /** Gate policy: info/debug/warn require [enabled]; error always prints. */
    protected fun log(
        priority: Int,
        tag: String,
        msg: String,
        tr: Throwable? = null,
    ) {
        if (priority < Log.ERROR && !enabled) return
        emit(priority, tag, msg, tr)
    }
}
