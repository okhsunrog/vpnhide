package dev.okhsunrog.vpnhide.debug

import android.content.Context
import dev.okhsunrog.vpnhide.LogTags
import dev.okhsunrog.vpnhide.RootSnapshot
import dev.okhsunrog.vpnhide.RootSnapshotCache
import dev.okhsunrog.vpnhide.VpnHideLog
import dev.okhsunrog.vpnhide.collectDebugShellSnapshot
import dev.okhsunrog.vpnhide.core.CoreProbe
import dev.okhsunrog.vpnhide.core.CoreRoot
import dev.okhsunrog.vpnhide.diagnostics.buildHookDiagnosticsText
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
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * Records a full `logcat -b all` session and saves it as a diagnostic ZIP.
 *
 * The app cannot read other apps' logs and runs no root process, so core reads them:
 * while recording, the app polls core's fixed `LogcatSince` probe every few seconds
 * and appends the lines newer than the last one it kept ([LogcatTail]), so a ring
 * buffer that rotates mid-session loses nothing older than one poll. Polling rather
 * than streaming is deliberate: core's tarpc IPC has no server streaming and its
 * probes are bounded, killable children (see docs/debug-bundle.md, "How
 * `logcat_full.txt` is captured").
 *
 * The raw logcat stream is captured between Start/Stop while debug logging is
 * temporarily enabled through the same canonical-config path used by the
 * one-shot debug ZIP. Stop then adds the shared device/backend/kernel snapshot
 * so reports about a third-party app still carry enough VPN Hide context.
 */
internal object LogcatRecorder {
    private const val TAG = LogTags.LOGCAT
    private const val LOGCAT_STOP_GRACE_MS = 200L
    private const val LOGCAT_POLL_MS = 10_000L

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
        val tail: LogcatTail,
        val scope: CoroutineScope,
        val pollJob: Job,
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
        var scope: CoroutineScope? = null
        try {
            loggingSession = beginDebugCaptureLogging()
            val startMs = System.currentTimeMillis()
            // Step back one second so the start marker and any immediate debug
            // propagation lines are not lost to logcat timestamp rounding.
            val logcatSince = formatLogcatSince(Date(startMs - 1_000L))
            writeLogcatMarker("start", id)

            val tail = LogcatTail(logcatSince)
            scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
            val pollJob = pollLogcatToFile(scope, tail, rawLogFile)
            val sizeJob = publishSize(scope, rawLogFile)

            active =
                ActiveRecording(
                    id = id,
                    rawLogFile = rawLogFile,
                    logcatSince = logcatSince,
                    startMs = startMs,
                    selfNeedsRestart = selfNeedsRestart,
                    tail = tail,
                    scope = scope,
                    pollJob = pollJob,
                    sizeJob = sizeJob,
                    loggingSession = loggingSession,
                )
            _state.value = State.Recording(rawLogFile, startMs, 0L)
            return rawLogFile
        } catch (t: Throwable) {
            scope?.cancel()
            loggingSession?.let { restoreDebugCaptureLogging(it) }
            runCatching { rawLogFile.delete() }
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

            val stateJson = captureLogcatState(context, recording, stopMs, duration)
            restoreAttempted = true

            val timestamp = SimpleDateFormat("yyyyMMdd_HHmmss", Locale.US).format(Date(stopMs))
            zipFile = File(context.cacheDir, "vpnhide_logcat_$timestamp.zip")
            writeDiagnosticZip(
                zipFile = zipFile,
                textEntries = mapOf("state.json" to stateJson),
                fileEntries = listOf(DiagnosticFileEntry("logcat_full.txt", recording.rawLogFile)),
            )
            _state.value = State.Stopped(zipFile, duration)
            return zipFile
        } finally {
            withContext(NonCancellable) {
                if (!processStopped) {
                    runCatching { stopLogcatProcess(recording) }
                        .onFailure { VpnHideLog.w(TAG, "failed to stop logcat polling: ${it.message}") }
                }
                if (!restoreAttempted) {
                    runCatching { restoreDebugCaptureLogging(recording.loggingSession) }
                        .onFailure { VpnHideLog.w(TAG, "failed to restore debug logging: ${it.message}") }
                }
                if (zipFile == null) {
                    _state.value = State.Stopped(null, duration)
                }
                // The raw stream has been packed into the ZIP (or the run
                // failed); either way it is no longer needed. Without this each
                // Start/Stop leaves a multi-MB raw .log behind in cacheDir.
                runCatching { recording.rawLogFile.delete() }
            }
        }
    }

    private fun pollLogcatToFile(
        scope: CoroutineScope,
        tail: LogcatTail,
        file: File,
    ): Job =
        scope.launch {
            while (isActive) {
                delay(LOGCAT_POLL_MS)
                pollOnce(tail, file)
            }
        }

    /** One core read of everything since the last kept line, appended to [file]. */
    private suspend fun pollOnce(
        tail: LogcatTail,
        file: File,
    ) {
        val result = CoreRoot.collectOne(CoreProbe.LogcatSince(tail.since))
        if (!result.ok) {
            VpnHideLog.w(TAG, "logcat poll failed: core exit=${result.exit}")
            return
        }
        val fresh = tail.accept(result.text)
        if (fresh.isNotEmpty()) file.appendText(fresh.joinToString("\n", postfix = "\n"))
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
        recording.pollJob.cancelAndJoin()
        recording.sizeJob.cancelAndJoin()
        recording.scope.cancel()
        // The final read: everything up to and including the stop marker.
        pollOnce(recording.tail, recording.rawLogFile)
    }

    /** Markers go to this app's own log, which core's logcat read includes. */
    private fun writeLogcatMarker(
        phase: String,
        id: String,
    ) {
        android.util.Log.i(TAG, "full-system-logcat $phase id=$id")
    }

    // Build the canonical state.json for this recording: the same authoritative
    // root snapshot the dashboard uses, plus forensic sections, minus a diagnostics
    // run (the payload is the raw log). Restores debug logging as part of the flow.
    private suspend fun captureLogcatState(
        context: Context,
        recording: ActiveRecording,
        stopMs: Long,
        duration: Long,
    ): String {
        val dmesg = captureDmesg()
        val rootSnapshot =
            runCatching { RootSnapshotCache.refresh() }.getOrElse { RootSnapshot(emptyMap()) }
        val shellSnapshot = collectDebugShellSnapshot()
        val session = recording.loggingSession.withRestore(restoreDebugCaptureLogging(recording.loggingSession))
        return buildVpnHideState(
            context = context,
            captureKind = "full_system_logcat",
            generatedAt = isoNow(),
            selfNeedsRestart = recording.selfNeedsRestart,
            rootSnapshot =
                RootSnapshot(
                    rootSnapshot.sections +
                        ("logcat_recording" to buildRecordingText(recording, stopMs, duration)),
                ),
            shellSnapshot = shellSnapshot,
            gate = null,
            checkResults = null,
            dmesg = dmesg,
            logcat = filterVpnHideLogcat(recording.rawLogFile),
            bootLsposedLogcat = captureBootLsposedLogcat(),
            lsposedConfigDb = buildLsposedConfigText(context),
            hookReport = buildHookDiagnosticsText(context, shellSnapshot),
            debugCapture = session.toDebugCaptureInfo(),
            errors = emptyList(),
            // A full-logcat bug report wants the forensic context; keep the
            // installed-app list private (redacted) as before.
            options = StateContentOptions(forensics = true, appList = false),
        ).toJson()
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
            appendLine("logcatSource=core logcat_since (logcat -d -b all -v threadtime -T), polled every ${LOGCAT_POLL_MS}ms")
        }.trimEnd()

    private fun filterVpnHideLogcat(file: File): String =
        buildString {
            file.bufferedReader().useLines { lines ->
                lines.filter(::isVpnHideLogcatLine).forEach { appendLine(it) }
            }
        }.trimEnd()

    private fun isVpnHideLogcatLine(line: String): Boolean = (LogTags.APP_TAGS + LogTags.NATIVE_TAGS).any { line.contains(it) }

    private fun formatLogcatSince(date: Date): String = SimpleDateFormat("MM-dd HH:mm:ss.SSS", Locale.US).format(date)

    private fun formatWallTime(ms: Long): String = SimpleDateFormat("yyyy-MM-dd HH:mm:ss Z", Locale.US).format(Date(ms))
}

/**
 * Stitches repeated `logcat -d -T <since>` reads into one stream. `-T` includes lines at
 * exactly `since`, so the lines already kept at the newest stamp are remembered and
 * skipped; older lines and logcat's buffer banners are dropped.
 */
internal class LogcatTail(
    start: String,
) {
    var since: String = start
        private set
    private var keptAtSince: MutableSet<String> = mutableSetOf()

    fun accept(chunk: String): List<String> {
        val fresh = mutableListOf<String>()
        chunk.lineSequence().forEach { line ->
            val stamp = logcatStamp(line) ?: return@forEach
            when {
                stamp < since -> {}

                stamp == since -> {
                    if (keptAtSince.add(line)) fresh += line
                }

                else -> {
                    since = stamp
                    keptAtSince = mutableSetOf(line)
                    fresh += line
                }
            }
        }
        return fresh
    }

    private fun logcatStamp(line: String): String? = line.take(STAMP_LENGTH).takeIf { LOGCAT_STAMP.matches(it) }

    private companion object {
        const val STAMP_LENGTH = 18
        val LOGCAT_STAMP = Regex("""\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}""")
    }
}
