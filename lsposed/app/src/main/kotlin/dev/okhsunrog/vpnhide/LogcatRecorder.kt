package dev.okhsunrog.vpnhide

import android.content.Context
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.concurrent.TimeUnit

/**
 * Records a full `logcat -b all` session and saves it as a diagnostic ZIP.
 *
 * The raw logcat stream is captured between Start/Stop while debug logging is
 * temporarily enabled through the same canonical-config path used by the
 * one-shot debug ZIP. Stop then adds the shared device/backend/kernel snapshot
 * so reports about a third-party app still carry enough VPN Hide context.
 */
internal object LogcatRecorder {
    private const val TAG = "VpnHide-Logcat"
    private const val LOGCAT_STOP_GRACE_MS = 200L
    private const val LOGCAT_PIPE_JOIN_MS = 2_000L

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

    private data class ActiveRecording(
        val id: String,
        val rawLogFile: File,
        val logcatSince: String,
        val startMs: Long,
        val selfNeedsRestart: Boolean,
        val process: Process,
        val scope: CoroutineScope,
        val pipeJob: Job,
        val sizeJob: Job,
        val loggingSession: DebugCaptureLoggingSession,
    )

    private val _state = MutableStateFlow<State>(State.Stopped(null))
    val state: StateFlow<State> = _state

    private val activeMutex = Mutex()
    private var active: ActiveRecording? = null

    /**
     * Start recording. No-op if already recording.
     *
     * The returned file is the temporary raw logcat stream. The user-visible
     * artifact is created by [stop] as `vpnhide_logcat_*.zip`.
     */
    suspend fun start(
        context: Context,
        selfNeedsRestart: Boolean,
    ): File? =
        withContext(Dispatchers.IO) {
            activeMutex.withLock {
                active?.let { return@withLock it.rawLogFile }
                startRecordingLocked(
                    context = context.applicationContext,
                    selfNeedsRestart = selfNeedsRestart,
                )
            }
        }

    private suspend fun startRecordingLocked(
        context: Context,
        selfNeedsRestart: Boolean,
    ): File? {
        val timestamp = SimpleDateFormat("yyyyMMdd_HHmmss", Locale.US).format(Date())
        val id = timestamp
        val rawLogFile = File(context.cacheDir, "vpnhide_logcat_raw_$timestamp.log")
        runCatching { rawLogFile.createNewFile() }
            .onFailure {
                VpnHideLog.w(TAG, "failed to create output file: ${it.message}")
                return null
            }

        var loggingSession: DebugCaptureLoggingSession? = null
        var process: Process? = null
        var scope: CoroutineScope? = null
        try {
            loggingSession = beginDebugCaptureLogging(context)
            val startMs = System.currentTimeMillis()
            // Step back one second so the start marker and any immediate debug
            // propagation lines are not lost to logcat timestamp rounding.
            val logcatSince = formatLogcatSince(Date(startMs - 1_000L))
            writeLogcatMarker("start", id)

            process = startLogcatProcess(logcatSince)
            if (process == null) {
                restoreDebugCaptureLogging(context, loggingSession)
                return null
            }

            scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
            val pipeJob = pipeLogcatToFile(scope, process, rawLogFile)
            val sizeJob = publishSize(scope, rawLogFile)

            active =
                ActiveRecording(
                    id = id,
                    rawLogFile = rawLogFile,
                    logcatSince = logcatSince,
                    startMs = startMs,
                    selfNeedsRestart = selfNeedsRestart,
                    process = process,
                    scope = scope,
                    pipeJob = pipeJob,
                    sizeJob = sizeJob,
                    loggingSession = loggingSession,
                )
            _state.value = State.Recording(rawLogFile, startMs, 0L)
            return rawLogFile
        } catch (t: Throwable) {
            process?.destroyForcibly()
            scope?.cancel()
            loggingSession?.let { restoreDebugCaptureLogging(context, it) }
            throw t
        }
    }

    /**
     * Stop recording and return the final diagnostic ZIP, or null if no
     * recording was active. Safe to call multiple times.
     */
    suspend fun stop(context: Context): File? =
        withContext(Dispatchers.IO) {
            activeMutex.withLock {
                val recording = active ?: return@withLock null
                active = null
                stopRecordingLocked(
                    context = context.applicationContext,
                    recording = recording,
                )
            }
        }

    private suspend fun stopRecordingLocked(
        context: Context,
        recording: ActiveRecording,
    ): File? {
        val stopMs = System.currentTimeMillis()
        val duration = (stopMs - recording.startMs).coerceAtLeast(0L)
        var processStopped = false
        var restoreAttempted = false
        var zipFile: File? = null

        try {
            writeLogcatMarker("stop", recording.id)
            delay(LOGCAT_STOP_GRACE_MS)
            stopLogcatProcess(recording)
            processStopped = true

            val dmesg = suExec("dmesg 2>/dev/null").second
            val shellSnapshot = collectDebugShellSnapshot()
            val restore = restoreDebugCaptureLogging(context, recording.loggingSession)
            restoreAttempted = true
            val completedLoggingSession = recording.loggingSession.withRestore(restore)

            val timestamp = SimpleDateFormat("yyyyMMdd_HHmmss", Locale.US).format(Date(stopMs))
            zipFile = File(context.cacheDir, "vpnhide_logcat_$timestamp.zip")

            writeDiagnosticZip(
                zipFile = zipFile,
                textEntries =
                    buildFullSystemLogcatTextEntries(
                        context = context,
                        recording = recording,
                        stopMs = stopMs,
                        duration = duration,
                        shellSnapshot = shellSnapshot,
                        loggingSession = completedLoggingSession,
                        dmesg = dmesg,
                    ),
                fileEntries = listOf(DiagnosticFileEntry("logcat_full.txt", recording.rawLogFile)),
            )
            _state.value = State.Stopped(zipFile, duration)
            return zipFile
        } finally {
            withContext(NonCancellable) {
                if (!processStopped) {
                    runCatching { stopLogcatProcess(recording) }
                        .onFailure { VpnHideLog.w(TAG, "failed to stop logcat process: ${it.message}") }
                }
                if (!restoreAttempted) {
                    runCatching { restoreDebugCaptureLogging(context, recording.loggingSession) }
                        .onFailure { VpnHideLog.w(TAG, "failed to restore debug logging: ${it.message}") }
                }
                if (zipFile == null) {
                    _state.value = State.Stopped(null, duration)
                }
            }
        }
    }

    private fun buildFullSystemLogcatTextEntries(
        context: Context,
        recording: ActiveRecording,
        stopMs: Long,
        duration: Long,
        shellSnapshot: DebugShellSnapshot,
        loggingSession: DebugCaptureLoggingSession,
        dmesg: String,
    ): LinkedHashMap<String, String> {
        val files =
            linkedMapOf(
                "summary.txt" to
                    buildDiagnosticSummaryText(
                        context = context,
                        selfNeedsRestart = recording.selfNeedsRestart,
                        results = null,
                        shellSnapshot = shellSnapshot,
                        loggingSession = loggingSession,
                        captureKind = "full_system_logcat",
                    ),
                "recording.txt" to buildRecordingText(recording, stopMs, duration),
            )
        files.putAll(
            buildCommonDiagnosticTextFiles(
                context = context,
                selfNeedsRestart = recording.selfNeedsRestart,
                shellSnapshot = shellSnapshot,
                loggingSession = loggingSession,
            ),
        )
        files["hook_report.txt"] =
            buildHookDiagnosticsText(
                context = context,
                shellSnapshot = shellSnapshot,
            )
        files["dmesg_vpnhide.txt"] = filterVpnHideDmesg(dmesg)
        files["dmesg_full.txt"] = dmesg.ifBlank { "(no dmesg entries)" }
        files["logcat_vpnhide.txt"] = filterVpnHideLogcat(recording.rawLogFile).ifBlank { "(no VpnHide logcat entries)" }
        return files
    }

    private fun startLogcatProcess(logcatSince: String): Process? =
        try {
            ProcessBuilder("su", "-c", "exec logcat -b all -v threadtime -T \"$logcatSince\"")
                .redirectErrorStream(true)
                .start()
        } catch (t: Throwable) {
            VpnHideLog.w(TAG, "failed to spawn logcat via su: ${t.message}")
            null
        }

    private fun pipeLogcatToFile(
        scope: CoroutineScope,
        process: Process,
        file: File,
    ): Job =
        scope.launch {
            try {
                process.inputStream.use { input ->
                    file.outputStream().use { out ->
                        val buf = ByteArray(DEFAULT_BUFFER_SIZE)
                        while (true) {
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

    private fun publishSize(
        scope: CoroutineScope,
        file: File,
    ): Job =
        scope.launch {
            while (isActive) {
                delay(500)
                val current = _state.value
                if (current is State.Recording) {
                    _state.value = current.copy(sizeBytes = file.length())
                }
            }
        }

    private suspend fun stopLogcatProcess(recording: ActiveRecording) {
        recording.process.destroy()
        if (!recording.process.waitFor(LOGCAT_PIPE_JOIN_MS, TimeUnit.MILLISECONDS)) {
            recording.process.destroyForcibly()
        }
        withTimeoutOrNull(LOGCAT_PIPE_JOIN_MS) { recording.pipeJob.join() }
        recording.sizeJob.cancelAndJoin()
        recording.scope.cancel()
    }

    private fun writeLogcatMarker(
        phase: String,
        id: String,
    ) {
        suExec(
            "log -t $TAG \"full-system-logcat $phase id=$id\" >/dev/null 2>&1",
            timeoutSec = 2,
        )
    }

    private fun buildRecordingText(
        recording: ActiveRecording,
        stopMs: Long,
        durationMs: Long,
    ): String =
        buildString {
            appendLine("recordingId=${recording.id}")
            appendLine("started=${formatWallTime(recording.startMs)}")
            appendLine("stopped=${formatWallTime(stopMs)}")
            appendLine("durationMs=$durationMs")
            appendLine("selfNeedsRestart=${recording.selfNeedsRestart}")
            appendLine("logcatSince=${recording.logcatSince}")
            appendLine("rawLogBytes=${recording.rawLogFile.length()}")
            appendLine("logcatCommand=logcat -b all -v threadtime -T \"${recording.logcatSince}\"")
        }.trimEnd()

    private fun filterVpnHideLogcat(file: File): String =
        buildString {
            file.bufferedReader().useLines { lines ->
                lines.filter(::isVpnHideLogcatLine).forEach { appendLine(it) }
            }
        }.trimEnd()

    private fun isVpnHideLogcatLine(line: String): Boolean =
        listOf(
            "VPNHideTest",
            "VpnHide",
            "VpnHide-Dashboard",
            "VpnHide-Startup",
            "VpnHide-LSPosed",
            "VpnHide-Diag",
            "VpnHide-Logcat",
            "VpnHide-Update",
            "VpnHideAgentBridge",
            "vpnhide",
            "vpnhide_ports",
            "vpnhide-zygisk",
            "shadowhook_tag",
        ).any { line.contains(it) }

    private fun formatLogcatSince(date: Date): String = SimpleDateFormat("MM-dd HH:mm:ss.SSS", Locale.US).format(date)

    private fun formatWallTime(ms: Long): String = SimpleDateFormat("yyyy-MM-dd HH:mm:ss Z", Locale.US).format(Date(ms))
}
