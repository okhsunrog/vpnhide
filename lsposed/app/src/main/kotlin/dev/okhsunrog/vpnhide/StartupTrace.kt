package dev.okhsunrog.vpnhide

import android.os.SystemClock
import android.util.Log
import java.util.concurrent.atomic.AtomicBoolean

private const val STARTUP_TAG = LogTags.STARTUP

internal object StartupTrace {
    private val originMs = SystemClock.elapsedRealtime()
    private val readyLogged = AtomicBoolean(false)

    fun mark(event: String) {
        Log.i(STARTUP_TAG, "event=$event elapsedMs=${elapsedMs()}")
    }

    fun metric(
        name: String,
        valueMs: Long,
    ) {
        Log.i(STARTUP_TAG, "metric=$name valueMs=$valueMs elapsedMs=${elapsedMs()}")
    }

    fun dashboardReady() {
        if (readyLogged.compareAndSet(false, true)) {
            Log.i(STARTUP_TAG, "event=dashboard_ready elapsedMs=${elapsedMs()}")
        }
    }

    fun rootDeniedReady() {
        if (readyLogged.compareAndSet(false, true)) {
            Log.i(STARTUP_TAG, "event=root_denied_ready elapsedMs=${elapsedMs()}")
        }
    }

    private fun elapsedMs(): Long = SystemClock.elapsedRealtime() - originMs
}
