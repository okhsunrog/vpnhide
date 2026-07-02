package dev.okhsunrog.vpnhide

import android.os.FileObserver
import android.util.Log
import de.robv.android.xposed.XposedBridge

/**
 * system_server logcat facade for LSPosed hooks, which can't reach the app's
 * canonical-snapshot cache. Same gate-and-level policy as [VpnHideLog] (see
 * [GatedLogger]) — info is per-request/hot-path and gated, error always prints
 * so "hooks didn't attach" reports stay diagnosable — but the flag is read
 * straight from the canonical JSON on [install] and refreshed by an inotify
 * watcher, so a toggle flip lands without restarting system_server.
 *
 * Every line also goes to `XposedBridge.log`, keeping Settings → Debugging →
 * Debug logging visible through framework UIs that surface the Xposed log
 * separately from ordinary bug-report captures.
 */
internal object HookLog : GatedLogger() {
    @Volatile private var watcher: FileObserver? = null

    fun install() {
        reload()
        if (watcher != null) return
        // MODIFY covers manual in-place edits; MOVED_TO/CLOSE_WRITE from
        // watchSystemDataDir cover the app's atomic JSON replacement.
        watcher =
            watchSystemDataDir(extraEvents = FileObserver.MODIFY) { path ->
                if (path == "vpnhide_config.json") {
                    SystemServerConfigCache.invalidate()
                    reload()
                }
            }
    }

    private fun reload() {
        enabled = SystemServerConfigCache.load().debug
    }

    override fun emit(
        priority: Int,
        tag: String,
        msg: String,
        tr: Throwable?,
    ) {
        Log.println(priority, tag, if (tr == null) msg else "$msg\n${Log.getStackTraceString(tr)}")
        XposedBridge.log(msg)
    }

    fun i(msg: String) = log(Log.INFO, LogTags.LSPOSED, msg)

    /** Always prints — used for install failures and other diagnostics we can't afford to lose. */
    fun e(msg: String) = log(Log.ERROR, LogTags.LSPOSED, msg)
}
