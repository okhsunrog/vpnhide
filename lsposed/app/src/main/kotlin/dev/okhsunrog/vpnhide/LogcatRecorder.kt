package dev.okhsunrog.vpnhide

import android.content.Context
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * Records an unfiltered `logcat -b all` stream to a file, driven by a
 * single start/stop lifecycle. Used by the Diagnostics screen to capture
 * everything happening during an issue reproduction — our own VpnHide
 * tags, system_server, radio, crashes, anything.
 *
 * Requires root (READ_LOGS is not granted to third-party apps); we spawn
 * `logcat` under `su` so the subprocess inherits CAP_SYSLOG.
 *
 * Only one recording may be active at a time. The process is intentionally
 * spawned via `exec logcat ...` so `Process.destroy()` kills logcat, not
 * just the wrapping shell.
 */
internal object LogcatRecorder {
    private const val TAG = "VpnHide-Logcat"

    sealed interface State {
        data class Stopped(
            val lastFile: File?,
            val lastDurationMs: Long = 0L,
        ) : State

        data class Recording(
            val file: File,
            val startMs: Long,
            val sizeBytes: Long,
        ) : State
    }

    private val _state = MutableStateFlow<State>(State.Stopped(null))
    val state: StateFlow<State> = _state

    private var process: Process? = null
    private var scope: CoroutineScope? = null

    // True while this recording force-enabled debug logging because the
    // user's persisted preference was OFF. [stop] uses it to decide
    // whether to restore the original state — we always reconcile against
    // current SharedPreferences rather than a snapshot, so a user flipping
    // the UI toggle mid-capture wins over our rollback.
    private var loggingForcedByRecorder = false

    /**
     * Start recording. No-op if already recording.
     *
     * Output is a plain-text logcat capture at `-v threadtime` format,
     * all buffers (`main system crash events radio`), written to a file
     * in the app's cache directory. Returns the target file immediately;
     * the process runs in the background until [stop] is called.
     *
     * If the user's debug-logging preference is currently OFF, this
     * temporarily turns logging on for the duration of the recording so
     * the capture actually contains VpnHide-tagged lines. Without this
     * forcing, a default-install user pressing Start → reproduce →
     * Stop → Share would send a logcat with zero VpnHide entries in it,
     * which is useless for diagnosing bugs.
     */
    fun start(context: Context): File? {
        if (_state.value is State.Recording) return (_state.value as State.Recording).file

        val ts = SimpleDateFormat("yyyyMMdd_HHmmss", Locale.US).format(Date())
        val file = File(context.cacheDir, "vpnhide_logcat_$ts.log")
        try {
            file.createNewFile()
        } catch (t: Throwable) {
            VpnHideLog.w(TAG, "failed to create output file: ${t.message}")
            return null
        }

        val proc =
            try {
                // `exec logcat` ensures the PID we hold is the logcat
                // process itself — su destroy() would otherwise kill
                // only the shell wrapper.
                ProcessBuilder("su", "-c", "exec logcat -b all -v threadtime")
                    .redirectErrorStream(true)
                    .start()
            } catch (t: Throwable) {
                VpnHideLog.w(TAG, "failed to spawn logcat via su: ${t.message}")
                return null
            }

        process = proc
        if (!VpnHideLog.enabled) {
            applyDebugLoggingRuntime(true)
            loggingForcedByRecorder = true
        }
        val newScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        scope = newScope
        val startMs = System.currentTimeMillis()
        _state.value = State.Recording(file, startMs, 0L)

        // Pipe stdout → file
        newScope.launch {
            try {
                proc.inputStream.use { input ->
                    file.outputStream().use { out ->
                        val buf = ByteArray(8 * 1024)
                        while (isActive) {
                            val n = input.read(buf)
                            if (n < 0) break
                            if (n > 0) out.write(buf, 0, n)
                        }
                    }
                }
            } catch (t: Throwable) {
                VpnHideLog.w(TAG, "pipe error: ${t.message}")
            }
        }

        // Periodic size refresh so the UI shows growing recording size
        newScope.launch {
            while (isActive) {
                delay(500)
                val current = _state.value
                if (current is State.Recording) {
                    _state.value = current.copy(sizeBytes = file.length())
                }
            }
        }

        return file
    }

    /**
     * Stop recording. Returns the captured file, or null if nothing was
     * recording. Safe to call multiple times. [context] is needed to read
     * the user's persisted debug-logging preference when restoring the
     * logging state we may have force-enabled in [start].
     */
    suspend fun stop(context: Context): File? {
        val current = _state.value as? State.Recording ?: return null

        process?.destroy()
        process = null
        scope?.cancel()
        scope = null

        // Give the pipe a moment to flush, then publish final state.
        withContext(Dispatchers.IO) {
            // small grace period for the piping coroutine to finish writing
            delay(120)
            if (loggingForcedByRecorder) {
                val target = isEnabledInPrefs(context)
                if (VpnHideLog.enabled != target) applyDebugLoggingRuntime(target)
                loggingForcedByRecorder = false
            }
        }
        val duration = (System.currentTimeMillis() - current.startMs).coerceAtLeast(0L)
        _state.value = State.Stopped(current.file, duration)
        return current.file
    }
}
