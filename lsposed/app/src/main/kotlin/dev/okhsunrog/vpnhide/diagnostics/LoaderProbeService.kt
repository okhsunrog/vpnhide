package dev.okhsunrog.vpnhide.diagnostics

import android.app.Application
import android.app.Service
import android.content.Intent
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.Process

/** A genuine Zygote app process. Core observes its specialization independently;
 * this service cannot manufacture loader or hook-health evidence.
 */
class LoaderProbeService : Service() {
    private val handler = Handler(Looper.getMainLooper())
    private val finish =
        Runnable {
            stopSelf()
            // stopSelf leaves a cached process. End only this dedicated worker so
            // the next run consumes the current Zygisk authorization from a fresh fork.
            if (Application.getProcessName() == WORKER_PROCESS) {
                Process.killProcess(Process.myPid())
            }
        }

    override fun onCreate() {
        super.onCreate()
        if (Application.getProcessName() != WORKER_PROCESS) {
            stopSelf()
            return
        }
        handler.postDelayed(finish, LIFETIME_MS)
    }

    override fun onStartCommand(
        intent: Intent?,
        flags: Int,
        startId: Int,
    ): Int = START_NOT_STICKY

    override fun onBind(intent: Intent?): IBinder? = null

    private companion object {
        const val WORKER_PROCESS = "dev.okhsunrog.vpnhide:diagnostics"
        const val LIFETIME_MS = 6_000L
    }
}
