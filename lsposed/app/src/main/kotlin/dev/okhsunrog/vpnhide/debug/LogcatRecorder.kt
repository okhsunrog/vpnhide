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
import dev.okhsunrog.vpnhide.core.LogcatDebugRestore
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
 * Debug logging is core's too: Start makes core turn it on through its own journaled config
 * write and hold it on, and core restores it when the session ends however it ends (Stop, an
 * auto-stop, or the next core start after a crash), so a killed app can't leave it on. The
 * ZIP's `debugCapture` comes from core's lease record.
 *
 * Delivery is acknowledged: once the ZIP is written the app tells core ([CoreRoot.ackLogcat]),
 * and only then does core drop its copy. A finished recording the app never saved (killed
 * mid-collect, or auto-stopped while the app was gone) stays in core and is offered on
 * Diagnostics as [State.Undelivered]: save it ([saveUndelivered]) or discard it.
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
            /** The last Start, Stop, save or discard failed: core's reply status. */
            val failure: Failure? = null,
        ) : State

        data class Recording(
            val sessionId: Long,
            val startMs: Long,
            val sizeBytes: Long,
            val maxBytes: Long,
            val maxDurationMs: Long,
        ) : State

        /** Core holds a finished recording the app has not saved yet. */
        data class Undelivered(
            val sessionId: Long,
            val startMs: Long,
            val durationMs: Long,
            val sizeBytes: Long,
            val autoStop: LogcatAutoStop?,
            val maxBytes: Long,
            val maxDurationMs: Long,
            /** The last save or discard attempt failed: core's reply status. */
            val failure: Failure? = null,
        ) : State
    }

    data class Failure(
        val stage: Stage,
        val status: String,
    ) {
        enum class Stage { Start, Stop, Discard }
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
        /** Started by an earlier app process (adopted, or offered after the fact). */
        val adopted: Boolean,
        val watchJob: Job?,
    )

    private val _state = MutableStateFlow<State>(State.Stopped(null))
    val state: StateFlow<State> = _state

    private var active: ActiveRecording? = null
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val actions = CaptureActionOwner(scope)
    private val random = SecureRandom()

    /** Start recording. If a session is already running (ours or one core still holds), it is kept. */
    suspend fun start(
        context: Context,
        selfNeedsRestart: Boolean,
    ): StartOutcome =
        actions.perform {
            active?.let { return@perform StartOutcome.Started(it.id) }
            startRecordingLocked(context.applicationContext, selfNeedsRestart)
        }

    /**
     * Pick up what core holds: adopt a session it is still recording (the app was restarted
     * mid-recording) so the card shows it and Stop collects it, or offer a finished one the
     * app never saved. No-op while this process holds one.
     */
    suspend fun refresh(context: Context) =
        actions.perform {
            if (active != null) return@perform
            val reply = CoreRoot.logcatStatus()
            if (reply !is CoreLogcatReply.Ok) return@perform
            val session = reply.session
            when {
                session == null -> {
                    if (_state.value is State.Undelivered) _state.value = State.Stopped(null)
                }

                session.recording -> {
                    adoptLocked(context.applicationContext, session, selfNeedsRestart = false)
                }

                session.undelivered -> {
                    _state.value = session.toUndelivered()
                }

                _state.value is State.Undelivered -> {
                    _state.value = State.Stopped(null)
                }
            }
        }

    private suspend fun startRecordingLocked(
        context: Context,
        selfNeedsRestart: Boolean,
    ): StartOutcome {
        val id = newSessionId()
        return when (val reply = CoreRoot.startLogcat(id)) {
            is CoreLogcatReply.Ok -> {
                val session = reply.session
                if (session == null || !session.recording) {
                    fail(Failure.Stage.Start, "not_recording", "core ended the session at once")
                } else {
                    writeLogcatMarker("start", id)
                    track(context, session, selfNeedsRestart, adopted = false)
                    StartOutcome.Started(id)
                }
            }

            is CoreLogcatReply.Busy -> {
                // A session from before an app restart: show it so it can be stopped.
                val running = (CoreRoot.logcatStatus() as? CoreLogcatReply.Ok)?.session
                if (running != null && running.recording && running.id == reply.running) {
                    adoptLocked(context, running, selfNeedsRestart)
                    StartOutcome.Started(running.id)
                } else {
                    fail(Failure.Stage.Start, "busy", "another recording is running")
                }
            }

            is CoreLogcatReply.Failed -> {
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
        track(context, session, selfNeedsRestart, adopted = true)
    }

    private fun track(
        context: Context,
        session: CoreLogcatSession,
        selfNeedsRestart: Boolean,
        adopted: Boolean,
    ) {
        active =
            ActiveRecording(
                id = session.id,
                startMs = session.startedAtMs,
                selfNeedsRestart = selfNeedsRestart,
                adopted = adopted,
                watchJob = watch(context, session.id),
            )
        _state.value = session.toRecording()
    }

    private fun CoreLogcatSession.toRecording() = State.Recording(id, startedAtMs, bytes, maxBytes, maxDurationMs)

    private fun CoreLogcatSession.toUndelivered(failure: Failure? = null) =
        State.Undelivered(id, startedAtMs, durationMs, bytes, autoStop, maxBytes, maxDurationMs, failure)

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
        actions.perform {
            val recording = active ?: return@perform null
            if (expectedId != null && recording.id != expectedId) return@perform null
            writeLogcatMarker("stop", recording.id)
            delay(LOGCAT_STOP_GRACE_MS)
            collectLocked(context.applicationContext, recording)
        }

    /**
     * Save the finished recording core still holds ([State.Undelivered]) as a ZIP.
     * [selfNeedsRestart] is this app process's current view (the original one is gone).
     */
    suspend fun saveUndelivered(
        context: Context,
        selfNeedsRestart: Boolean,
    ): File? =
        actions.perform {
            if (active != null) return@perform null
            val offered = _state.value as? State.Undelivered ?: return@perform null
            val recording = ActiveRecording(offered.sessionId, offered.startMs, selfNeedsRestart, adopted = true, watchJob = null)
            collectLocked(context.applicationContext, recording)
        }

    /** Drop the finished recording core still holds; core deletes it. */
    suspend fun discardUndelivered() =
        actions.perform {
            if (active != null) return@perform
            val offered = _state.value as? State.Undelivered ?: return@perform
            when (val reply = CoreRoot.discardLogcat(offered.sessionId)) {
                is CoreLogcatReply.Ok -> _state.value = State.Stopped(null)
                is CoreLogcatReply.Busy -> _state.value = offered.copy(failure = Failure(Failure.Stage.Discard, "busy"))
                is CoreLogcatReply.Failed -> _state.value = offered.copy(failure = Failure(Failure.Stage.Discard, reply.status))
            }
        }

    /** Stop (or re-collect) [recording] in core, build the ZIP, then acknowledge it to core. */
    private suspend fun collectLocked(
        context: Context,
        recording: ActiveRecording,
    ): File? {
        var rawLogFile: File? = null
        try {
            val reply = CoreRoot.stopLogcat(recording.id)
            val session = (reply as? CoreLogcatReply.Ok)?.session
            rawLogFile = session?.file
            if (session != null && !session.recording && active?.id == recording.id) {
                active = null
                recording.watchJob?.cancel()
            }
            if (session == null || rawLogFile == null || !rawLogFile.exists()) {
                reportMissingRecording(recording, reply, session)
                return null
            }
            val stopMs = System.currentTimeMillis()
            val stateJson = captureLogcatState(context, recording, session, stopMs)

            val timestamp = SimpleDateFormat("yyyyMMdd_HHmmss", Locale.US).format(Date(stopMs))
            val zipFile = File(context.cacheDir, "vpnhide_logcat_$timestamp.zip")
            writeDiagnosticZip(
                zipFile = zipFile,
                textEntries = mapOf("state.json" to stateJson),
                fileEntries = listOf(DiagnosticFileEntry("logcat_full.txt", rawLogFile)),
            )
            // The ZIP is on disk: now core may drop its copy. A lost ack leaves the session
            // undelivered in core; the next refresh offers it again (and a repeated ack is a no-op).
            when (val ack = CoreRoot.ackLogcat(recording.id)) {
                is CoreLogcatReply.Ok -> {}

                is CoreLogcatReply.Busy -> {
                    VpnHideLog.w(TAG, "logcat ack: busy")
                }

                is CoreLogcatReply.Failed -> {
                    VpnHideLog.w(TAG, "logcat ack failed: ${ack.status} ${ack.detail}")
                }
            }
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
                // Core's export copy is packed into the ZIP (or the run failed); core still
                // holds its own until the ack, so a retry is handed a fresh copy.
                rawLogFile?.let { runCatching { it.delete() } }
            }
        }
    }

    private fun reportMissingRecording(
        recording: ActiveRecording,
        reply: CoreLogcatReply,
        session: CoreLogcatSession?,
    ) {
        val status =
            when (reply) {
                is CoreLogcatReply.Failed -> reply.status
                is CoreLogcatReply.Busy -> "busy"
                is CoreLogcatReply.Ok -> if (session?.delivered == true) "already_saved" else "no_file"
            }
        VpnHideLog.w(TAG, "logcat collect failed: $status")
        // Still in core and not saved: keep offering it.
        _state.value =
            if (active?.id == recording.id) {
                session?.takeIf { it.recording }?.toRecording() ?: _state.value
            } else {
                session?.takeIf { it.undelivered }?.toUndelivered(Failure(Failure.Stage.Stop, status))
                    ?: State.Stopped(null, failure = Failure(Failure.Stage.Stop, status))
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
    // run (the payload is the raw log). Debug logging is core's (see the lease).
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
            debugCapture = session.debugCaptureInfo(),
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
            appendLine("adoptedAfterAppRestart=${recording.adopted}")
            appendLine("rawLogBytes=${session.bytes}")
            appendLine("truncated=${session.truncated}")
            appendLine("autoStopped=${session.autoStop?.wire ?: "no"}")
            session.logcatExit?.let { appendLine("logcatExit=$it") }
            appendLine(
                "debugLease=${session.debug?.let {
                    "previousDebug=${it.previousDebug} previousDebugSwitch=${it.previousDebugSwitch} restore=${it.restore.wire}"
                } ?: "none"}",
            )
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

/**
 * The ZIP's `debugCapture` from core's lease: forced when core turned debug logging on;
 * apply/restore are 0 when core's write landed, -1 for a restore still pending, null when
 * nothing was written (no lease, or still held).
 */
internal fun CoreLogcatSession.debugCaptureInfo(): DebugCaptureInfo? {
    val lease = debug ?: return null
    val restoreExit =
        when (lease.restore) {
            LogcatDebugRestore.Restored, LogcatDebugRestore.UserChoice -> 0
            LogcatDebugRestore.Pending -> -1
            LogcatDebugRestore.Held -> null
        }
    return DebugCaptureInfo(
        forced = lease.forced,
        applyExit = 0,
        restoreExit = restoreExit,
        detail = "core lease, restore=${lease.restore.wire}",
    )
}
