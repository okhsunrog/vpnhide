package dev.okhsunrog.vpnhide.diagnostics

import android.app.Application
import android.app.Service
import android.content.Intent
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.PowerManager
import android.os.Process
import dev.okhsunrog.vpnhide.core.CorePaths
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel
import kotlinx.coroutines.joinAll
import kotlinx.coroutines.launch
import kotlinx.coroutines.supervisorScope
import uniffi.vpnhide_core_ffi.CoreDiagnosticException
import uniffi.vpnhide_core_ffi.CoreDiagnosticWorkerTask
import uniffi.vpnhide_core_ffi.CoreException
import uniffi.vpnhide_core_ffi.CoreSession
import uniffi.vpnhide_core_ffi.observeDiagnosticFixture
import uniffi.vpnhide_core_ffi.registerDiagnosticRaw
import java.util.concurrent.Executors

/** A genuine Zygote app process. Core observes its specialization independently;
 * this service cannot manufacture loader or hook-health evidence.
 */
class LoaderProbeService : Service() {
    private val collector =
        Executors
            .newSingleThreadExecutor { task ->
                Thread(task, "vpnhide-diagnostic-collector")
            }.asCoroutineDispatcher()
    private val scope = CoroutineScope(SupervisorJob() + collector)
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
        handler.postDelayed(finish, SERVICE_WATCHDOG_MS)
        scope.launch {
            try {
                workerStage("connect") { CoreSession(CorePaths.socket(this@LoaderProbeService)) }.use { core ->
                    val task = workerStage("join") { core.joinDiagnosticWorker() }
                    if (task != null) {
                        runDiagnostic(core, task)
                    } else {
                        // Loader selection also starts this worker before any diagnostic run.
                        // Keep it alive briefly for independent specialization observation.
                        handler.postDelayed(finish, LOADER_LIFETIME_MS)
                    }
                }
            } catch (_: CoreException) {
                // Invalid local task; Core owns timeout and cleanup.
            } catch (_: CoreDiagnosticException) {
                // Core owns timeout, attribution and cleanup; a lost reply cannot
                // manufacture a successful result or restart the run from this process.
            }
        }
    }

    private suspend fun runDiagnostic(
        core: CoreSession,
        task: CoreDiagnosticWorkerTask,
    ) {
        val power = getSystemService(PowerManager::class.java)
        val wakeLock = power.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "VPNHide:Diagnostics")
        try {
            wakeLock.acquire(MEASUREMENT_WATCHDOG_MS)
            val tid = Process.myTid().toUInt()
            val observations =
                workerStage("native-${task.session}") {
                    measureNativeFixture(
                        step = { phase -> core.nativeWorkerStep(task, tid, phase) },
                        register = ::registerDiagnosticRaw,
                        observe = { observeDiagnosticFixture(task) },
                    )
                }
            try {
                // Retain native evidence BEFORE Java work: process death during
                // framework observations must not erase an already measured leak.
                workerStage("native-report-${task.session}") { core.reportDiagnosticWorker(task, observations) }
                supervisorScope {
                    val ports = async(Dispatchers.IO) { measurePortsFixture(core, task) }
                    val framework = async { measureFrameworkFixture(this@LoaderProbeService, core, task) }
                    // A failure in one collector must still let the other submit its evidence.
                    joinAll(ports, framework)
                    framework.await()
                    ports.await()
                }
            } finally {
                workerStage("finish-${task.session}") { core.finishDiagnosticWorker(task) }
            }
        } finally {
            if (wakeLock.isHeld) wakeLock.release()
            handler.post(finish)
        }
    }

    override fun onStartCommand(
        intent: Intent?,
        flags: Int,
        startId: Int,
    ): Int = START_NOT_STICKY

    override fun onDestroy() {
        handler.removeCallbacks(finish)
        scope.cancel()
        collector.close()
        super.onDestroy()
        // Core stops this service before a fresh diagnostic authorization is consumed.
        // A cached process would keep the previous Zygisk policy generation.
        if (Application.getProcessName() == WORKER_PROCESS) {
            Process.killProcess(Process.myPid())
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private companion object {
        const val WORKER_PROCESS = "dev.okhsunrog.vpnhide:diagnostics"
        const val LOADER_LIFETIME_MS = 6_000L

        // The Rust coordinator owns phase deadlines. These are Android-side
        // bounds on process lifetime and the wake lock if Core stops responding.
        const val SERVICE_WATCHDOG_MS = 65_000L
        const val MEASUREMENT_WATCHDOG_MS = 35_000L
    }
}
