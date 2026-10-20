package dev.okhsunrog.vpnhide.debug

import android.content.Context
import dev.okhsunrog.vpnhide.LogTags
import dev.okhsunrog.vpnhide.RootSnapshot
import dev.okhsunrog.vpnhide.RootSnapshotCache
import dev.okhsunrog.vpnhide.VpnHideLog
import dev.okhsunrog.vpnhide.collectDebugShellSnapshot
import dev.okhsunrog.vpnhide.core.CoreLogcatReply
import dev.okhsunrog.vpnhide.core.CoreLogcatSession
import dev.okhsunrog.vpnhide.core.CoreRoot
import dev.okhsunrog.vpnhide.core.LogcatAutoStop
import dev.okhsunrog.vpnhide.diagnostics.buildHookDiagnosticsText
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import java.io.File
import java.security.SecureRandom
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * Records a full `logcat -b all` session and saves it as a diagnostic ZIP.
 *
 * The app cannot read other apps' logs and runs no root process, so core records: Start asks
 * core to run its one `logcat` child into a core-owned file, Stop has core end it and copy
 * the file into the app's export dir. Core enforces the limits itself (a byte cap and a
 * maximum duration, see core-daemon `logcat_session`), so a killed app never leaves root
 * logcat running. While recording, the app only asks core for the session's size and
 * state; when core reports it auto-stopped, the app collects the file at once and says why.
 * After an app restart the running session is adopted ([refresh]) and can be stopped.
 *
 * Debug logging is temporarily enabled through the same canonical-config path used by the
 * one-shot debug ZIP. Stop then adds the shared device/backend/kernel snapshot so reports
 * about a third-party app still carry enough VPN Hide context.
 */
internal object LogcatRecorder {
    private const val TAG = LogTags.LOGCAT
    private const val LOGCAT_STOP_GRACE_MS = 200L
    private const val STATUS_POLL_MS = 2_000L

    sealed interface State {
        data class Stopped(
            val lastFile: File?,
            val lastDurationMs: Long = 0L,
            /** Core ended the last session on its own limits. */
            val autoStop: LogcatAutoStop? = null,
            val maxBytes: Long = 0L,
            val maxDurationMs: Long = 0L,
            /** The last Start or Stop failed: core's reply status. */
            val failure: Failure? = null,
        ) : State

        data class Recording(
            val sessionId: Long,
            val startMs: Long,
            val sizeBytes: Long,
            val maxBytes: Long,
            val maxDurationMs: Long,
        ) : State
    }

    data class Failure(
        val stage: Stage,
        val status: String,
    ) {
        enum class Stage { Start, Stop }
    }

    sealed interface StartOutcome {
        data class Started(
            val sessionId: Long,
        ) : StartOutcome

        data class Failed(
            val status: String,
            val detail: String,
        ) : StartOutcome
    }

    private data class ActiveRecording(
        val id: Long,
        val startMs: Long,
        val selfNeedsRestart: Boolean,
        /** Null for a session adopted after an app restart (its lease died with the old process). */
        val loggingSession: DebugCaptureLoggingSession?,
        val watchJob: Job,
    )

    private val _state = MutableStateFlow<State>(State.Stopped(null))
    val state: StateFlow<State> = _state

    private val activeMutex = Mutex()
    private var active: ActiveRecording? = null
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val random = SecureRandom()

    /** Start recording. If a session is already running (ours or one core still holds), it is kept. */
    suspend fun start(
        context: Context,
        selfNeedsRestart: Boolean,
    ): StartOutcome =
        withContext(Dispatchers.IO) {
            activeMutex.withLock {
                active?.let { return@withLock StartOutcome.Started(it.id) }
                startRecordingLocked(context.applicationContext, selfNeedsRestart)
            }
        }

    /**
     * Adopt a session core is still recording (the app was restarted mid-recording), so the
     * card shows it and Stop collects it. No-op while this process holds one.
     */
    suspend fun refresh(context: Context) =
        withContext(Dispatchers.IO) {
            activeMutex.withLock {
                if (active != null) return@withLock
                val session = (CoreRoot.logcatStatus() as? CoreLogcatReply.Ok)?.session ?: return@withLock
                if (session.recording) adoptLocked(context.applicationContext, session, selfNeedsRestart = false)
            }
        }

    private suspend fun startRecordingLocked(
        context: Context,
        selfNeedsRestart: Boolean,
    ): StartOutcome {
        val id = newSessionId()
        val loggingSession = beginDebugCaptureLogging()
        return when (val reply = CoreRoot.startLogcat(id)) {
            is CoreLogcatReply.Ok -> {
                val session = reply.session
                if (session == null || !session.recording) {
                    restoreDebugCaptureLogging(loggingSession)
                    fail(Failure.Stage.Start, "not_recording", "core ended the session at once")
                } else {
                    writeLogcatMarker("start", id)
                    track(context, session, selfNeedsRestart, loggingSession)
                    StartOutcome.Started(id)
                }
            }

            is CoreLogcatReply.Busy -> {
                // A session from before an app restart: show it so it can be stopped.
                restoreDebugCaptureLogging(loggingSession)
                val running = (CoreRoot.logcatStatus() as? CoreLogcatReply.Ok)?.session
                if (running != null && running.recording && running.id == reply.running) {
                    adoptLocked(context, running, selfNeedsRestart)
                    StartOutcome.Started(running.id)
                } else {
                    fail(Failure.Stage.Start, "busy", "another recording is running")
                }
            }

            is CoreLogcatReply.Failed -> {
                restoreDebugCaptureLogging(loggingSession)
                fail(Failure.Stage.Start, reply.status, reply.detail)
            }
        }
    }

    private fun fail(
        stage: Failure.Stage,
        status: String,
        detail: String,
    ): StartOutcome.Failed {
        VpnHideLog.w(TAG, "logcat ${stage.name.lowercase()} failed: $status $detail")
        _state.value = State.Stopped(null, failure = Failure(stage, status))
        return StartOutcome.Failed(status, detail)
    }

    private fun adoptLocked(
        context: Context,
        session: CoreLogcatSession,
        selfNeedsRestart: Boolean,
    ) {
        VpnHideLog.i(TAG, "adopting core logcat session ${session.id}")
        track(context, session, selfNeedsRestart, loggingSession = null)
    }

    private fun track(
        context: Context,
        session: CoreLogcatSession,
        selfNeedsRestart: Boolean,
        loggingSession: DebugCaptureLoggingSession?,
    ) {
        active =
            ActiveRecording(
                id = session.id,
                startMs = session.startedAtMs,
                selfNeedsRestart = selfNeedsRestart,
                loggingSession = loggingSession,
                watchJob = watch(context, session.id),
            )
        _state.value = session.toRecording()
    }

    private fun CoreLogcatSession.toRecording() = State.Recording(id, startedAtMs, bytes, maxBytes, maxDurationMs)

    /** Size and state only (no log lines cross); an auto-stop is collected at once. */
    private fun watch(
        context: Context,
        id: Long,
    ): Job =
        scope.launch {
            while (isActive) {
                delay(STATUS_POLL_MS)
                val session = (CoreRoot.logcatStatus() as? CoreLogcatReply.Ok)?.session ?: continue
                if (session.id != id) continue
                if (session.recording) {
                    if (_state.value is State.Recording) _state.value = session.toRecording()
                } else {
                    scope.launch { stop(context, expectedId = id) }
                    return@launch
                }
            }
        }

    /**
     * Stop recording and return the final diagnostic ZIP, or null if no recording was
     * active or core could not hand the file over. Safe to call multiple times.
     */
    suspend fun stop(context: Context): File? = stop(context, expectedId = null)

    private suspend fun stop(
        context: Context,
        expectedId: Long?,
    ): File? =
        withContext(Dispatchers.IO) {
            activeMutex.withLock {
                val recording = active ?: return@withLock null
                if (expectedId != null && recording.id != expectedId) return@withLock null
                active = null
                stopRecordingLocked(context.applicationContext, recording)
            }
        }

    private suspend fun stopRecordingLocked(
        context: Context,
        recording: ActiveRecording,
    ): File? {
        var restoreAttempted = false
        var rawLogFile: File? = null
        try {
            recording.watchJob.cancel()
            writeLogcatMarker("stop", recording.id)
            delay(LOGCAT_STOP_GRACE_MS)
            val session =
                when (val reply = CoreRoot.stopLogcat(recording.id)) {
                    is CoreLogcatReply.Ok -> reply.session
                    is CoreLogcatReply.Busy -> null
                    is CoreLogcatReply.Failed -> null.also { fail(Failure.Stage.Stop, reply.status, reply.detail) }
                }
            rawLogFile = session?.file
            if (session == null || rawLogFile == null || !rawLogFile.exists()) {
                if (_state.value !is State.Stopped) _state.value = State.Stopped(null, failure = Failure(Failure.Stage.Stop, "no_file"))
                return null
            }
            val stopMs = System.currentTimeMillis()
            val stateJson = captureLogcatState(context, recording, session, stopMs)
            restoreAttempted = true

            val timestamp = SimpleDateFormat("yyyyMMdd_HHmmss", Locale.US).format(Date(stopMs))
            val zipFile = File(context.cacheDir, "vpnhide_logcat_$timestamp.zip")
            writeDiagnosticZip(
                zipFile = zipFile,
                textEntries = mapOf("state.json" to stateJson),
                fileEntries = listOf(DiagnosticFileEntry("logcat_full.txt", rawLogFile)),
            )
            _state.value =
                State.Stopped(
                    lastFile = zipFile,
                    lastDurationMs = session.durationMs,
                    autoStop = session.autoStop,
                    maxBytes = session.maxBytes,
                    maxDurationMs = session.maxDurationMs,
                )
            return zipFile
        } finally {
            withContext(NonCancellable) {
                if (!restoreAttempted) {
                    recording.loggingSession?.let { session ->
                        runCatching { restoreDebugCaptureLogging(session) }
                            .onFailure { VpnHideLog.w(TAG, "failed to restore debug logging: ${it.message}") }
                    }
                }
                // Core's copy has been packed into the ZIP (or the run failed); a repeated
                // Stop would hand it over again, so it is not kept here.
                rawLogFile?.let { runCatching { it.delete() } }
            }
        }
    }

    private fun newSessionId(): Long {
        while (true) {
            val id = random.nextLong() and Long.MAX_VALUE
            if (id != 0L) return id
        }
    }

    /** Markers go to this app's own log, which core's logcat child records. */
    private fun writeLogcatMarker(
        phase: String,
        id: Long,
    ) {
        android.util.Log.i(TAG, "full-system-logcat $phase id=$id")
    }

    // Build the canonical state.json for this recording: the same authoritative
    // root snapshot the dashboard uses, plus forensic sections, minus a diagnostics
    // run (the payload is the raw log). Restores debug logging as part of the flow.
    private suspend fun captureLogcatState(
        context: Context,
        recording: ActiveRecording,
        session: CoreLogcatSession,
        stopMs: Long,
    ): String {
        val rawLogFile = requireNotNull(session.file)
        val dmesg = captureDmesg()
        val rootSnapshot =
            runCatching { RootSnapshotCache.refresh() }.getOrElse { RootSnapshot(emptyMap()) }
        val shellSnapshot = collectDebugShellSnapshot()
        val logging = recording.loggingSession?.let { it.withRestore(restoreDebugCaptureLogging(it)) }
        return buildVpnHideState(
            context = context,
            captureKind = "full_system_logcat",
            generatedAt = isoNow(),
            selfNeedsRestart = recording.selfNeedsRestart,
            rootSnapshot =
                RootSnapshot(
                    rootSnapshot.sections +
                        ("logcat_recording" to buildRecordingText(recording, session, stopMs)),
                ),
            shellSnapshot = shellSnapshot,
            gate = null,
            checkResults = null,
            dmesg = dmesg,
            logcat = filterVpnHideLogcat(rawLogFile),
            bootLsposedLogcat = captureBootLsposedLogcat(),
            lsposedConfigDb = buildLsposedConfigText(context),
            hookReport = buildHookDiagnosticsText(context, shellSnapshot),
            debugCapture = logging?.toDebugCaptureInfo(),
            errors = emptyList(),
            // A full-logcat bug report wants the forensic context; keep the
            // installed-app list private (redacted) as before.
            options = StateContentOptions(forensics = true, appList = false),
        ).toJson()
    }

    private fun buildRecordingText(
        recording: ActiveRecording,
        session: CoreLogcatSession,
        stopMs: Long,
    ): String =
        buildString {
            appendLine("recordingId=${recording.id}")
            appendLine("started=${formatWallTime(session.startedAtMs)}")
            appendLine("stopped=${formatWallTime(stopMs)}")
            appendLine("durationMs=${session.durationMs}")
            appendLine("selfNeedsRestart=${recording.selfNeedsRestart}")
            appendLine("adoptedAfterAppRestart=${recording.loggingSession == null}")
            appendLine("rawLogBytes=${session.bytes}")
            appendLine("truncated=${session.truncated}")
            appendLine("autoStopped=${session.autoStop?.wire ?: "no"}")
            session.logcatExit?.let { appendLine("logcatExit=$it") }
            appendLine("coreLimits=${session.maxBytes} bytes, ${session.maxDurationMs} ms")
            appendLine("logcatSource=core session (logcat -b all -v threadtime -T <start-1s>, one child, core-owned file)")
        }.trimEnd()

    private fun filterVpnHideLogcat(file: File): String =
        buildString {
            file.bufferedReader().useLines { lines ->
                lines.filter(::isVpnHideLogcatLine).forEach { appendLine(it) }
            }
        }.trimEnd()

    private fun isVpnHideLogcatLine(line: String): Boolean = (LogTags.APP_TAGS + LogTags.NATIVE_TAGS).any { line.contains(it) }

    private fun formatWallTime(ms: Long): String = SimpleDateFormat("yyyy-MM-dd HH:mm:ss Z", Locale.US).format(Date(ms))
}
