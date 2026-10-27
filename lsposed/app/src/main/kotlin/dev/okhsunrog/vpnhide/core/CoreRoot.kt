package dev.okhsunrog.vpnhide.core

import android.content.Context
import dev.okhsunrog.vpnhide.CoreSessions
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeoutOrNull
import uniffi.vpnhide_core_ffi.CoreDebugLeaseOwner
import uniffi.vpnhide_core_ffi.CoreDiagnosticsReport
import uniffi.vpnhide_core_ffi.CoreDiagnosticsRequest
import uniffi.vpnhide_core_ffi.CoreException
import uniffi.vpnhide_core_ffi.CoreLogcatAutoStopReason
import uniffi.vpnhide_core_ffi.CoreLogcatDebugRestore
import uniffi.vpnhide_core_ffi.CoreLogcatError
import uniffi.vpnhide_core_ffi.CoreLogcatRequest
import uniffi.vpnhide_core_ffi.CoreLogcatSessionState
import uniffi.vpnhide_core_ffi.CoreLogcatSessionStatus
import uniffi.vpnhide_core_ffi.CoreProbeOutput
import uniffi.vpnhide_core_ffi.LogcatReply
import java.io.File
import uniffi.vpnhide_core_ffi.CoreDiagnosticProbe as CoreProbe

/** A fixed root effect core applies (`core-rpc` `RootEffect`). */
internal sealed class CoreRootEffect {
    /** Core's input rule for this effect (core-daemon `effects::apply`), checked before dispatch. */
    open val valid: Boolean get() = true

    /** Not a data class: the key must never reach toString, logs or state. */
    class SetSuperkey(
        val superkey: String,
    ) : CoreRootEffect() {
        override val valid: Boolean
            get() = superkey.trim().let { it.isNotEmpty() && it.toByteArray().size <= MAX_SUPERKEY_BYTES && '\n' !in it && '\u0000' !in it }

        override fun toString(): String = "SetSuperkey(<redacted>)"

        override fun equals(other: Any?): Boolean = other is SetSuperkey && other.superkey == superkey

        override fun hashCode(): Int = superkey.hashCode()
    }

    data object ClearSuperkey : CoreRootEffect()

    companion object {
        const val MAX_SUPERKEY_BYTES = 4096
    }

    data object DeleteLegacyConfig : CoreRootEffect()

    data object FullReset : CoreRootEffect()
}

/**
 * One probe's result. [exit] is -1 when core could not run it (or was unreachable). [text] is
 * the output, read back from the export file core wrote when it was too large to inline.
 */
internal data class CoreProbeResult(
    val exit: Int,
    val text: String,
    val artifacts: List<File> = emptyList(),
    val truncated: Boolean = false,
) {
    val ok: Boolean get() = exit == 0

    companion object {
        val Unavailable = CoreProbeResult(-1, "")
    }
}

internal sealed interface CoreDiagnostics {
    data class Collected(
        val results: List<CoreProbeResult>,
        val dmesgCleared: Boolean,
    ) : CoreDiagnostics

    /** Core is not reachable, speaks another protocol, or the request was refused. */
    data class Failed(
        val status: String,
        val detail: String,
    ) : CoreDiagnostics

    /** Results in request order; every probe [CoreProbeResult.Unavailable] on failure. */
    fun results(count: Int): List<CoreProbeResult> =
        when (this) {
            is Collected -> results
            is Failed -> List(count) { CoreProbeResult.Unavailable }
        }
}

internal enum class CoreEffectOutcome {
    /** Applied. */
    Ok,

    /** Core refused it before running (invalid input, effects disabled, busy, unreachable). */
    FailedKnown,

    /** Delivered but the reply was lost: it may or may not have run. */
    Unknown,
}

/** Typed transport seam; serialized domain control values cannot be sent. */
internal interface CoreRootTransport {
    suspend fun collectDiagnostics(request: CoreDiagnosticsRequest): CoreDiagnosticsReport

    suspend fun logcatRecording(request: CoreLogcatRequest): LogcatReply

    suspend fun acquireDebugLease(id: Long)

    suspend fun releaseDebugLease(id: Long)
}

private class FfiCoreRootTransport(
    private val session: dev.okhsunrog.vpnhide.FfiCoreSessionClient,
) : CoreRootTransport {
    override suspend fun collectDiagnostics(request: CoreDiagnosticsRequest): CoreDiagnosticsReport =
        session.rootRequest { session.native.collectDiagnostics(request) }

    override suspend fun logcatRecording(request: CoreLogcatRequest): LogcatReply =
        session.rootRequest {
            session.native.logcatRecording(request)
        }

    override suspend fun acquireDebugLease(id: Long) =
        session.rootRequest {
            session.native.acquireDebugLease(id.toULong(), CoreDebugLeaseOwner.FORENSIC)
        }

    override suspend fun releaseDebugLease(id: Long) = session.rootRequest { session.native.releaseDebugLease(id.toULong()) }
}

/**
 * The app's only route to root: core runs the probes and effects. The APK runs no `su` and
 * no root process. Attach once with the app context; until then every call fails as
 * unreachable.
 */
internal object CoreRoot {
    @Volatile private var transport: CoreRootTransport? = null

    fun attach(context: Context) {
        if (transport == null) CoreSessions.attach(context.applicationContext)?.let { transport = FfiCoreRootTransport(it) }
    }

    /** Tests: replace the transport (null detaches). */
    internal fun setTransport(value: CoreRootTransport?) {
        transport = value
    }

    suspend fun collect(
        probes: List<CoreProbe>,
        clearDmesg: Boolean = false,
    ): CoreDiagnostics {
        val client = transport ?: return CoreDiagnostics.Failed("unreachable", "not attached")
        val request = CoreDiagnosticsRequest(probes, clearDmesg)
        val raw =
            try {
                withTimeoutOrNull(DIAGNOSTICS_TIMEOUT_MS) { client.collectDiagnostics(request) }
                    ?: return CoreDiagnostics.Failed("timeout", "")
            } catch (e: CancellationException) {
                throw e
            } catch (e: Throwable) {
                return CoreDiagnostics.Failed("unreachable", e.message.orEmpty())
            }
        return coreDiagnostics(raw, probes.size)
    }

    suspend fun collectOne(probe: CoreProbe): CoreProbeResult = collect(listOf(probe)).results(1).first()

    /** Blocking form for callers already on a background thread. */
    fun collectBlocking(
        probes: List<CoreProbe>,
        clearDmesg: Boolean = false,
    ): CoreDiagnostics = runBlocking(Dispatchers.IO) { collect(probes, clearDmesg) }

    fun collectOneBlocking(probe: CoreProbe): CoreProbeResult = collectBlocking(listOf(probe)).results(1).first()

    suspend fun debugLease(
        id: Long,
        acquire: Boolean,
    ): CoreEffectOutcome {
        val client = transport ?: return CoreEffectOutcome.FailedKnown
        return try {
            if (acquire) client.acquireDebugLease(id) else client.releaseDebugLease(id)
            runCatching { CoreSessions.client?.read() }
            CoreEffectOutcome.Ok
        } catch (e: CancellationException) {
            throw e
        } catch (_: CoreException.Refused) {
            CoreEffectOutcome.FailedKnown
        } catch (_: Throwable) {
            CoreEffectOutcome.Unknown
        }
    }

    /** Start core's logcat session [id] (idempotent per id). */
    suspend fun startLogcat(id: Long): CoreLogcatReply = logcat(CoreLogcatRequest.Start(id.toULong()), LOGCAT_CALL_TIMEOUT_MS)

    /** Stop session [id]; [CoreLogcatSession.file] is the copy core put in the export dir. */
    suspend fun stopLogcat(id: Long): CoreLogcatReply = logcat(CoreLogcatRequest.Stop(id.toULong()), LOGCAT_STOP_TIMEOUT_MS)

    /** The session core currently holds (running, or finished and not yet saved), if any. */
    suspend fun logcatStatus(): CoreLogcatReply = logcat(CoreLogcatRequest.Status, LOGCAT_CALL_TIMEOUT_MS)

    /** The app saved session [id]: core marks it delivered and deletes its copies (idempotent). */
    suspend fun ackLogcat(id: Long): CoreLogcatReply = logcat(CoreLogcatRequest.Ack(id.toULong()), LOGCAT_CALL_TIMEOUT_MS)

    /** Drop session [id] and its files; stops it first if it still records (idempotent). */
    suspend fun discardLogcat(id: Long): CoreLogcatReply = logcat(CoreLogcatRequest.Discard(id.toULong()), LOGCAT_STOP_TIMEOUT_MS)

    private suspend fun logcat(
        request: CoreLogcatRequest,
        timeoutMs: Long,
    ): CoreLogcatReply {
        val client = transport ?: return CoreLogcatReply.Failed("unreachable", "not attached")
        val raw =
            try {
                withTimeoutOrNull(timeoutMs) { client.logcatRecording(request) }
                    ?: return CoreLogcatReply.Failed("timeout", "")
            } catch (e: CancellationException) {
                throw e
            } catch (e: Throwable) {
                return CoreLogcatReply.Failed("unreachable", e.message.orEmpty())
            }
        return coreLogcatReply(raw)
    }

    // Core bounds the whole batch at 210 s (`DIAGNOSTICS_DEADLINE`).
    private const val DIAGNOSTICS_TIMEOUT_MS = 220_000L
    private const val LOGCAT_CALL_TIMEOUT_MS = 20_000L

    // Core's client waits 60 s for Stop (`LOGCAT_STOP_DEADLINE`).
    private const val LOGCAT_STOP_TIMEOUT_MS = 70_000L
}

/** Why core ended a logcat session nobody stopped (`core-rpc` `LogcatAutoStopReason`). */
internal enum class LogcatAutoStop(
    val wire: String,
) {
    SizeCap("size_cap"),
    DurationCap("duration_cap"),
    LogcatExited("logcat_exited"),

    /** Core stopped (crash, update, reboot) while the session recorded. */
    CoreRestarted("core_restarted"),
}

/** How core put debug logging back after a session (`core-rpc` `LogcatDebugRestore`). */
internal enum class LogcatDebugRestore(
    val wire: String,
) {
    Held("held"),
    Pending("pending"),
    Restored("restored"),
    UserChoice("user_choice"),
}

/** Core's debug-logging lease for a session (`core-rpc` `LogcatDebugLease`). */
internal data class CoreLogcatDebugLease(
    val previousDebug: Boolean,
    val previousDebugSwitch: Boolean,
    val restore: LogcatDebugRestore,
) {
    /** Core turned debug logging on for the session (it was off). */
    val forced: Boolean get() = !previousDebug
}

/** Core's view of one logcat recording (`core-rpc` `LogcatSessionStatus`). */
internal data class CoreLogcatSession(
    val id: Long,
    val recording: Boolean,
    /** Set when core ended the session on its own limits. */
    val autoStop: LogcatAutoStop?,
    val startedAtMs: Long,
    val durationMs: Long,
    val bytes: Long,
    val truncated: Boolean,
    val maxBytes: Long,
    val maxDurationMs: Long,
    val logcatExit: Int?,
    /** Stop only: the recording core copied into the app's export dir. */
    val file: File?,
    /** The app acknowledged it saved the recording; core holds no copy any more. */
    val delivered: Boolean = false,
    /** Null when core recorded without forcing debug logging (no config to write). */
    val debug: CoreLogcatDebugLease? = null,
) {
    /** Finished and not saved yet: the app should offer to save or discard it. */
    val undelivered: Boolean get() = !recording && !delivered
}

internal sealed interface CoreLogcatReply {
    /** [session] is null only for a status query when core holds no session. */
    data class Ok(
        val session: CoreLogcatSession?,
    ) : CoreLogcatReply

    /** Another session ([running]) is recording. */
    data class Busy(
        val running: Long,
    ) : CoreLogcatReply

    /** `unknown_session`, `disabled`, `refused`, `failed`, or a delivery status. */
    data class Failed(
        val status: String,
        val detail: String,
    ) : CoreLogcatReply
}

internal fun coreLogcatReply(reply: LogcatReply): CoreLogcatReply =
    when (reply) {
        is LogcatReply.Ok -> {
            CoreLogcatReply.Ok(reply.session?.let(::coreLogcatSession))
        }

        is LogcatReply.Refused -> {
            when (val reason = reply.reason) {
                is CoreLogcatError.Busy -> CoreLogcatReply.Busy(reason.running.toLong())
                CoreLogcatError.UnknownSession -> CoreLogcatReply.Failed("unknown_session", "")
                CoreLogcatError.Disabled -> CoreLogcatReply.Failed("disabled", "")
                is CoreLogcatError.Invalid -> CoreLogcatReply.Failed("invalid", reason.detail)
                is CoreLogcatError.Failed -> CoreLogcatReply.Failed("failed", reason.detail)
            }
        }
    }

internal fun coreLogcatSession(value: CoreLogcatSessionStatus): CoreLogcatSession =
    CoreLogcatSession(
        id = value.session.toLong(),
        recording = value.state == CoreLogcatSessionState.Recording,
        autoStop =
            (value.state as? CoreLogcatSessionState.AutoStopped)?.reason?.let { reason ->
                when (reason) {
                    CoreLogcatAutoStopReason.SIZE_CAP -> LogcatAutoStop.SizeCap
                    CoreLogcatAutoStopReason.DURATION_CAP -> LogcatAutoStop.DurationCap
                    CoreLogcatAutoStopReason.LOGCAT_EXITED -> LogcatAutoStop.LogcatExited
                    CoreLogcatAutoStopReason.CORE_RESTARTED -> LogcatAutoStop.CoreRestarted
                }
            },
        startedAtMs = value.startedAtMs.toLong(),
        durationMs = value.durationMs.toLong(),
        bytes = value.bytes.toLong(),
        truncated = value.truncated,
        maxBytes = value.maxBytes.toLong(),
        maxDurationMs = value.maxDurationMs.toLong(),
        logcatExit = value.logcatExit,
        file = value.file?.let(::File),
        delivered = value.delivered,
        debug =
            value.debug?.let { debug ->
                CoreLogcatDebugLease(
                    debug.previousDebug,
                    debug.previousDebugSwitch,
                    when (debug.restore) {
                        CoreLogcatDebugRestore.HELD -> LogcatDebugRestore.Held
                        CoreLogcatDebugRestore.PENDING -> LogcatDebugRestore.Pending
                        CoreLogcatDebugRestore.RESTORED -> LogcatDebugRestore.Restored
                        CoreLogcatDebugRestore.USER_CHOICE -> LogcatDebugRestore.UserChoice
                    },
                )
            },
    )

/** File payloads are forensic content; all control metadata arrived as generated records. */
internal fun coreDiagnostics(
    report: CoreDiagnosticsReport,
    expected: Int,
): CoreDiagnostics =
    if (report.outputs.size != expected) {
        CoreDiagnostics.Failed("malformed", "unexpected probe count")
    } else {
        CoreDiagnostics.Collected(report.outputs.map(::probeOutput), report.dmesgCleared)
    }

private fun probeOutput(value: CoreProbeOutput): CoreProbeResult {
    val file = value.file?.let(::File)
    val content = file?.let { runCatching { it.readText() }.getOrDefault(value.output).also { _ -> it.delete() } } ?: value.output
    return CoreProbeResult(value.exit, content, value.artifacts.map(::File), value.truncated)
}
