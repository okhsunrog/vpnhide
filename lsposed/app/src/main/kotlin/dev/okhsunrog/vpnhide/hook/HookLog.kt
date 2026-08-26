package dev.okhsunrog.vpnhide.hook

import android.os.FileObserver
import android.util.Log
import de.robv.android.xposed.XposedBridge
import dev.okhsunrog.vpnhide.GatedLogger
import dev.okhsunrog.vpnhide.LogTags
import dev.okhsunrog.vpnhide.VpnHideLog

/**
 * system_server logcat facade for LSPosed hooks, which can't reach the app's
 * canonical-snapshot cache. Same gate-and-level policy as [VpnHideLog] (see
 * [GatedLogger]) — info is per-request/hot-path and gated, error always prints
 * so "hooks didn't attach" reports stay diagnosable — but the flag is owned by
 * [SystemServerConfigCache], which re-reads the canonical JSON on the same 1s
 * stat poll that keeps the hooks' targets current. A toggle flip therefore lands
 * without restarting system_server, and without trusting inotify to deliver it.
 *
 * Every line also goes to `XposedBridge.log`, keeping Settings → Debugging →
 * Debug logging visible through framework UIs that surface the Xposed log
 * separately from ordinary bug-report captures.
 */
internal object HookLog : GatedLogger() {
    @Volatile private var watcher: FileObserver? = null

    fun install() {
        // Priming read: SystemServerConfigCache sets [enabled] whenever it
        // installs a fresh config, so this both loads the config and seeds the
        // flag. It is also the only step required for the flag to stay correct
        // — the watcher below is latency, not correctness.
        SystemServerConfigCache.load()
        if (watcher != null) return
        // Makes a toggle flip land immediately instead of within the cache's 1s
        // stat interval. MODIFY covers manual in-place edits; MOVED_TO/CLOSE_WRITE
        // from watchSystemDataDir cover the app's atomic JSON replacement.
        //
        // Do NOT make anything depend on this firing: on at least one device it
        // stopped delivering events entirely (see the note in
        // SystemServerConfigCache.load), which is exactly how the debug flag once
        // got stuck on for days.
        watcher =
            watchSystemDataDir(extraEvents = FileObserver.MODIFY) { path ->
                if (path == "vpnhide_config.json") {
                    SystemServerConfigCache.invalidate()
                    SystemServerConfigCache.load()
                }
            }
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
