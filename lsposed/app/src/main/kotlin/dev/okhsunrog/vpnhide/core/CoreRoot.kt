package dev.okhsunrog.vpnhide.core

import android.content.Context
import dev.okhsunrog.vpnhide.nativeBootFromCoreState
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeoutOrNull
import org.json.JSONArray
import org.json.JSONObject
import org.json.JSONTokener
import java.io.File

/**
 * One root probe core runs for the app (`core-rpc` `DiagnosticProbe`). The set is fixed in
 * core; the app can only name a probe, never pass a command.
 */
internal sealed class CoreProbe(
    val kind: String,
) {
    open fun json(): JSONObject = JSONObject().put("kind", kind)

    data class RootSnapshot(
        val packages: Boolean,
    ) : CoreProbe("root_snapshot") {
        override fun json(): JSONObject = super.json().put("packages", packages)
    }

    data object DebugSnapshot : CoreProbe("debug_snapshot")

    data object HookCounters : CoreProbe("hook_counters")

    data object Dmesg : CoreProbe("dmesg")

    data object LogcatAppTags : CoreProbe("logcat_app_tags")

    data object LogcatLsposedBoot : CoreProbe("logcat_lsposed_boot")

    data object KernelPartitions : CoreProbe("kernel_partitions")

    data object KernelImages : CoreProbe("kernel_images")

    data object LsposedModulesDb : CoreProbe("lsposed_modules_db")

    data object GroundTruthChecks : CoreProbe("ground_truth_checks")

    data class AppVpnState(
        val uid: Int,
    ) : CoreProbe("app_vpn_state") {
        override fun json(): JSONObject = super.json().put("uid", uid)
    }
}

/** A fixed root effect core applies (`core-rpc` `RootEffect`). */
internal sealed class CoreRootEffect(
    val kind: String,
) {
    open fun json(): JSONObject = JSONObject().put("kind", kind)

    /** Core's input rule for this effect (core-daemon `effects::apply`), checked before dispatch. */
    open val valid: Boolean get() = true

    /** Not a data class: the key must never reach toString, logs or state. */
    class SetSuperkey(
        val superkey: String,
    ) : CoreRootEffect("set_superkey") {
        override fun json(): JSONObject = super.json().put("superkey", superkey)

        override val valid: Boolean
            get() = superkey.trim().let { it.isNotEmpty() && it.toByteArray().size <= MAX_SUPERKEY_BYTES && '\n' !in it && '\u0000' !in it }

        override fun toString(): String = "SetSuperkey(<redacted>)"

        override fun equals(other: Any?): Boolean = other is SetSuperkey && other.superkey == superkey

        override fun hashCode(): Int = superkey.hashCode()
    }

    data object ClearSuperkey : CoreRootEffect("clear_superkey")

    companion object {
        const val MAX_SUPERKEY_BYTES = 4096
    }

    data object DeleteLegacyConfig : CoreRootEffect("delete_legacy_config")

    data object FullReset : CoreRootEffect("full_reset")
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

/** The core-ffi calls, as a seam for tests. */
internal interface CoreRootTransport {
    suspend fun collectDiagnostics(requestJson: String): String

    suspend fun rootEffect(
        op: Long,
        effectJson: String,
    ): String

    /** core-ffi `config_state`: core's published state, including the native boot decision. */
    suspend fun coreState(): String = ""

    /** core-ffi `logcat_recording`: start/stop/status/ack/discard of core's logcat session. */
    suspend fun logcatRecording(requestJson: String): String = ""
}

private class FfiCoreRootTransport(
    private val socketPath: String,
) : CoreRootTransport {
    override suspend fun collectDiagnostics(requestJson: String): String =
        uniffi.vpnhide_core_ffi.collectDiagnostics(socketPath, requestJson)

    override suspend fun rootEffect(
        op: Long,
        effectJson: String,
    ): String = uniffi.vpnhide_core_ffi.rootEffect(socketPath, op.toULong(), effectJson)

    override suspend fun coreState(): String = uniffi.vpnhide_core_ffi.configState(socketPath)

    override suspend fun logcatRecording(requestJson: String): String = uniffi.vpnhide_core_ffi.logcatRecording(socketPath, requestJson)
}

/**
 * The app's only route to root: core runs the probes and effects. The APK runs no `su` and
 * no root process. Attach once with the app context; until then every call fails as
 * unreachable.
 */
internal object CoreRoot {
    @Volatile private var transport: CoreRootTransport? = null

    fun attach(context: Context) {
        if (transport == null) transport = FfiCoreRootTransport(CorePaths.socket(context.applicationContext))
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
        val request =
            JSONObject()
                .put("probes", JSONArray(probes.map { it.json() }))
                .put("clear_dmesg", clearDmesg)
                .toString()
        val raw =
            try {
                withTimeoutOrNull(DIAGNOSTICS_TIMEOUT_MS) { client.collectDiagnostics(request) }
                    ?: return CoreDiagnostics.Failed("timeout", "")
            } catch (e: CancellationException) {
                throw e
            } catch (e: Throwable) {
                return CoreDiagnostics.Failed("unreachable", e.message.orEmpty())
            }
        return parseCoreDiagnostics(raw, probes.size)
    }

    /**
     * Core's once-per-boot native decision (`backend.boot` of its published state) as JSON,
     * or null when core is unreachable, has not scanned yet, or replied with anything else.
     */
    suspend fun nativeBootJson(): String? {
        val client = transport ?: return null
        return try {
            val raw = withTimeoutOrNull(STATE_TIMEOUT_MS) { client.coreState() } ?: return null
            val reply = JSONTokener(raw).nextValue() as? JSONObject ?: return null
            if (reply.optString("status") != "ok") return null
            reply.optJSONObject("state")?.let(::nativeBootFromCoreState)
        } catch (e: CancellationException) {
            throw e
        } catch (_: Throwable) {
            null
        }
    }

    fun nativeBootJsonBlocking(): String? = runBlocking(Dispatchers.IO) { nativeBootJson() }

    suspend fun collectOne(probe: CoreProbe): CoreProbeResult = collect(listOf(probe)).results(1).first()

    /** Blocking form for callers already on a background thread. */
    fun collectBlocking(
        probes: List<CoreProbe>,
        clearDmesg: Boolean = false,
    ): CoreDiagnostics = runBlocking(Dispatchers.IO) { collect(probes, clearDmesg) }

    fun collectOneBlocking(probe: CoreProbe): CoreProbeResult = collectBlocking(listOf(probe)).results(1).first()

    suspend fun apply(
        op: Long,
        effect: CoreRootEffect,
    ): CoreEffectOutcome {
        val client = transport ?: return CoreEffectOutcome.FailedKnown
        val raw =
            try {
                withTimeoutOrNull(EFFECT_TIMEOUT_MS) { client.rootEffect(op, effect.json().toString()) }
                    ?: return CoreEffectOutcome.Unknown
            } catch (e: CancellationException) {
                throw e
            } catch (_: UnsatisfiedLinkError) {
                return CoreEffectOutcome.FailedKnown
            } catch (_: Throwable) {
                return CoreEffectOutcome.Unknown
            }
        return parseCoreEffectReply(raw)
    }

    /** Start core's logcat session [id] (idempotent per id). */
    suspend fun startLogcat(id: Long): CoreLogcatReply = logcat("start", id, LOGCAT_CALL_TIMEOUT_MS)

    /** Stop session [id]; [CoreLogcatSession.file] is the copy core put in the export dir. */
    suspend fun stopLogcat(id: Long): CoreLogcatReply = logcat("stop", id, LOGCAT_STOP_TIMEOUT_MS)

    /** The session core currently holds (running, or finished and not yet saved), if any. */
    suspend fun logcatStatus(): CoreLogcatReply = logcat("status", 0L, LOGCAT_CALL_TIMEOUT_MS)

    /** The app saved session [id]: core marks it delivered and deletes its copies (idempotent). */
    suspend fun ackLogcat(id: Long): CoreLogcatReply = logcat("ack", id, LOGCAT_CALL_TIMEOUT_MS)

    /** Drop session [id] and its files; stops it first if it still records (idempotent). */
    suspend fun discardLogcat(id: Long): CoreLogcatReply = logcat("discard", id, LOGCAT_STOP_TIMEOUT_MS)

    private suspend fun logcat(
        action: String,
        id: Long,
        timeoutMs: Long,
    ): CoreLogcatReply {
        val client = transport ?: return CoreLogcatReply.Failed("unreachable", "not attached")
        val request = JSONObject().put("action", action).put("session", id).toString()
        val raw =
            try {
                withTimeoutOrNull(timeoutMs) { client.logcatRecording(request) }
                    ?: return CoreLogcatReply.Failed("timeout", "")
            } catch (e: CancellationException) {
                throw e
            } catch (e: Throwable) {
                return CoreLogcatReply.Failed("unreachable", e.message.orEmpty())
            }
        return parseCoreLogcatReply(raw)
    }

    // Core bounds the whole batch at 210 s (`DIAGNOSTICS_DEADLINE`).
    private const val DIAGNOSTICS_TIMEOUT_MS = 220_000L
    private const val STATE_TIMEOUT_MS = 15_000L
    private const val EFFECT_TIMEOUT_MS = 30_000L
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

internal fun parseCoreLogcatReply(raw: String): CoreLogcatReply =
    try {
        val json = strictObject(raw)
        when (val status = json.getString("status")) {
            "ok" -> CoreLogcatReply.Ok(json.optJSONObject("session")?.let(::parseCoreLogcatSession))
            "busy" -> CoreLogcatReply.Busy(json.getLong("running"))
            else -> CoreLogcatReply.Failed(status, json.optString("detail"))
        }
    } catch (e: Exception) {
        CoreLogcatReply.Failed("malformed", e.message.orEmpty())
    }

internal fun parseCoreLogcatSession(json: JSONObject): CoreLogcatSession {
    val state = json.getJSONObject("state")
    val kind = state.getString("kind")
    require(kind in setOf("recording", "stopped", "auto_stopped")) { "unknown logcat state $kind" }
    val reason = state.optString("reason")
    return CoreLogcatSession(
        id = json.getLong("session"),
        recording = kind == "recording",
        autoStop = if (kind == "auto_stopped") LogcatAutoStop.entries.first { it.wire == reason } else null,
        startedAtMs = json.getLong("started_at_ms"),
        durationMs = json.getLong("duration_ms"),
        bytes = json.getLong("bytes"),
        truncated = json.optBoolean("truncated"),
        maxBytes = json.getLong("max_bytes"),
        maxDurationMs = json.getLong("max_duration_ms"),
        logcatExit = if (json.has("logcat_exit") && !json.isNull("logcat_exit")) json.getInt("logcat_exit") else null,
        file = json.optString("file").takeIf { json.has("file") && it.isNotEmpty() }?.let(::File),
        delivered = json.optBoolean("delivered"),
        debug = json.optJSONObject("debug")?.let(::parseCoreLogcatDebugLease),
    )
}

private fun parseCoreLogcatDebugLease(json: JSONObject): CoreLogcatDebugLease {
    val restore = json.getString("restore")
    return CoreLogcatDebugLease(
        previousDebug = json.getBoolean("previous_debug"),
        previousDebugSwitch = json.getBoolean("previous_debug_switch"),
        restore = LogcatDebugRestore.entries.first { it.wire == restore },
    )
}

internal fun parseCoreEffectReply(raw: String): CoreEffectOutcome =
    when (runCatching { strictObject(raw).getString("status") }.getOrNull()) {
        "ok" -> CoreEffectOutcome.Ok
        "busy", "refused", "failed", "invalid_input", "unreachable", "protocol_skew" -> CoreEffectOutcome.FailedKnown
        else -> CoreEffectOutcome.Unknown
    }

/** Strict parse of a core-ffi diagnostics reply; probe files core exported are read back here. */
internal fun parseCoreDiagnostics(
    raw: String,
    expected: Int,
): CoreDiagnostics =
    try {
        val json = strictObject(raw)
        when (val status = json.getString("status")) {
            "ok" -> {
                val report = json.getJSONObject("report")
                val outputs = report.getJSONArray("outputs")
                require(outputs.length() == expected)
                CoreDiagnostics.Collected(
                    List(outputs.length()) { parseProbeOutput(outputs.getJSONObject(it)) },
                    report.optBoolean("dmesg_cleared"),
                )
            }

            else -> {
                CoreDiagnostics.Failed(status, json.optString("detail"))
            }
        }
    } catch (e: Exception) {
        CoreDiagnostics.Failed("malformed", e.message.orEmpty())
    }

private fun parseProbeOutput(json: JSONObject): CoreProbeResult {
    val file = json.optString("file").takeIf { json.has("file") && !json.isNull("file") && it.isNotEmpty() }?.let(::File)
    val inline = json.optString("output")
    // Core hands the export file to the app; it is read once and removed.
    val text = file?.let { runCatching { it.readText() }.getOrDefault(inline).also { _ -> it.delete() } } ?: inline
    val artifacts = json.optJSONArray("artifacts")
    return CoreProbeResult(
        exit = json.getInt("exit"),
        text = text,
        artifacts = List(artifacts?.length() ?: 0) { File(artifacts!!.getString(it)) },
        truncated = json.optBoolean("truncated"),
    )
}

private fun strictObject(raw: String): JSONObject {
    val tokener = JSONTokener(raw)
    val json = tokener.nextValue() as JSONObject
    require(tokener.nextClean() == '\u0000')
    return json
}
