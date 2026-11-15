package dev.okhsunrog.vpnhide.diagnostics

import android.os.SystemClock
import android.util.Log
import dev.okhsunrog.vpnhide.BuildConfig

private const val WORKER_TIMING_TAG = "VpnHideDiagnosticWorker"

internal inline fun <T> workerStage(
    stage: String,
    action: () -> T,
): T {
    if (!BuildConfig.DEBUG) return action()
    val start = SystemClock.elapsedRealtime()
    Log.i(WORKER_TIMING_TAG, "stage=$stage event=start boot_ms=$start")
    try {
        return action().also {
            Log.i(WORKER_TIMING_TAG, "stage=$stage event=done duration_ms=${SystemClock.elapsedRealtime() - start}")
        }
    } catch (failure: Exception) {
        Log.i(
            WORKER_TIMING_TAG,
            "stage=$stage event=failed duration_ms=${SystemClock.elapsedRealtime() - start} error=${failure.javaClass.simpleName}",
        )
        throw failure
    }
}
