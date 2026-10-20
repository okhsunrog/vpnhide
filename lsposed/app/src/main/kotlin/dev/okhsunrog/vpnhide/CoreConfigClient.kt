package dev.okhsunrog.vpnhide

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.withTimeoutOrNull
import org.json.JSONObject
import org.json.JSONTokener
import uniffi.vpnhide_core_ffi.configOperation
import uniffi.vpnhide_core_ffi.configRepair
import uniffi.vpnhide_core_ffi.configReplace
import uniffi.vpnhide_core_ffi.configState
import java.security.SecureRandom

/** The canonical config as core reports it. */
internal sealed interface RootCanonicalRead {
    data class Available(
        val config: CanonicalConfig,
    ) : RootCanonicalRead

    data object Missing : RootCanonicalRead

    data object Invalid : RootCanonicalRead

    data object Unavailable : RootCanonicalRead
}

/** What the root core holds, as the app's coordinator consumes it. */
internal data class CoreStateView(
    val epoch: Long,
    val configRevision: Long,
    val canonical: RootCanonicalRead,
    val operationsInFlight: List<Long>,
    /** A core logcat recording holds debug logging on: core keeps `debug` true meanwhile. */
    val debugLease: Boolean = false,
)

/** A core operation receipt: phase outcomes as core executed them, and the settled result. */
internal data class CoreOperationRecord(
    val op: Long,
    val phases: Map<ConfigPhase, PhaseOutcome>,
    val settled: PhaseOutcome?,
)

/** Relay status: the edit's native target set does not fit; nothing was saved or applied. */
internal const val NATIVE_CAPACITY_EXCEEDED = "native_capacity_exceeded"

internal sealed interface CoreReply {
    /** `state` / `operation` answered; [record] is null for an id core never admitted. */
    data class Observed(
        val state: CoreStateView,
        val record: CoreOperationRecord? = null,
    ) : CoreReply

    /** Core admitted the edit and owns it; [record] is its receipt once settled (or still running). */
    data class Accepted(
        val op: Long,
        val record: CoreOperationRecord?,
        val state: CoreStateView,
    ) : CoreReply

    /**
     * Core refused the edit before any phase ran (conflict, reused id, invalid, unavailable,
     * or [NATIVE_CAPACITY_EXCEEDED] with the typed report as JSON [detail]).
     */
    data class Refused(
        val reason: String,
        val detail: String,
    ) : CoreReply

    /** The app and core speak different IPC protocol versions: update the lagging one. */
    data object ProtocolSkew : CoreReply

    /** Core socket not reachable (module missing, disabled or not started). */
    data object Unreachable : CoreReply

    /** The call may or may not have reached core: query the same op id, never resubmit. */
    data object Uncertain : CoreReply
}

/** The app's only path to config mutations: core's typed commands. */
internal interface CoreConfigClient {
    suspend fun state(): CoreReply

    suspend fun operation(op: Long): CoreReply

    /** `ConfigEdit::Replace` with [canonicalJson] (on-disk format) against [baseRevision]. */
    suspend fun replace(
        op: Long,
        baseRevision: Long,
        canonicalJson: String,
    ): CoreReply

    /** `ConfigEdit::RepairActivation`: core re-applies Native and Ports from its durable config. */
    suspend fun repair(
        op: Long,
        baseRevision: Long,
    ): CoreReply
}

/** Fresh, non-zero, i64-safe operation ids: unique across app restarts so core never sees a reused id. */
internal object CoreOperationIds {
    private val random = SecureRandom()

    fun next(): Long {
        while (true) {
            val id = random.nextLong() and Long.MAX_VALUE
            if (id in 1 until Long.MAX_VALUE) return id
        }
    }
}

/**
 * The in-process core client: the UniFFI `core-ffi` library over the app-owned socket in the
 * app's device-encrypted data dir. No root process, no shell; core authenticates the peer uid.
 * Each call is bounded; a call that runs out of time may still have reached core ([CoreReply.Uncertain]).
 */
internal class FfiCoreConfigClient(
    private val socketPath: String,
) : CoreConfigClient {
    override suspend fun state(): CoreReply = call(QUERY_TIMEOUT_MS) { configState(socketPath) }

    override suspend fun operation(op: Long): CoreReply = call(QUERY_TIMEOUT_MS) { configOperation(socketPath, requireOp(op).toULong()) }

    override suspend fun replace(
        op: Long,
        baseRevision: Long,
        canonicalJson: String,
    ): CoreReply {
        require(canonicalJson.toByteArray(Charsets.UTF_8).size <= MAX_CORE_CONFIG_BYTES)
        return call(MUTATION_TIMEOUT_MS) {
            configReplace(socketPath, requireOp(op).toULong(), requireRevision(baseRevision).toULong(), canonicalJson)
        }
    }

    override suspend fun repair(
        op: Long,
        baseRevision: Long,
    ): CoreReply = call(MUTATION_TIMEOUT_MS) { configRepair(socketPath, requireOp(op).toULong(), requireRevision(baseRevision).toULong()) }

    private suspend fun call(
        timeoutMillis: Long,
        block: suspend () -> String,
    ): CoreReply =
        try {
            withTimeoutOrNull(timeoutMillis) { block() }?.let(::parseCoreReply) ?: CoreReply.Uncertain
        } catch (e: CancellationException) {
            throw e
        } catch (_: UnsatisfiedLinkError) {
            // The client library failed to load: nothing was sent.
            CoreReply.Unreachable
        } catch (_: Exception) {
            CoreReply.Uncertain
        }

    private fun requireOp(op: Long): Long = op.also { require(it in 1 until Long.MAX_VALUE) }

    private fun requireRevision(revision: Long): Long = revision.also { require(it in 0 until Long.MAX_VALUE) }

    private companion object {
        const val QUERY_TIMEOUT_MS = 15_000L

        // Core applies Native and Ports inside the submit; the client waits up to 90 s for settlement.
        const val MUTATION_TIMEOUT_MS = 110_000L
    }
}

/**
 * Strict parse of one core-ffi reply. Malformed output is [CoreReply.Uncertain]: it never
 * becomes an accepted receipt, and never a known failure the app could act on blindly.
 */
internal fun parseCoreReply(raw: String): CoreReply =
    try {
        val tokener = JSONTokener(raw)
        val json = tokener.nextValue() as JSONObject
        require(tokener.nextClean() == '\u0000')
        when (val status = json.getString("status")) {
            "ok", "unknown_op" -> {
                CoreReply.Observed(
                    parseCoreState(json.getJSONObject("state")),
                    json.optJSONObject("record")?.let(::parseCoreRecord),
                )
            }

            "accepted" -> {
                CoreReply.Accepted(
                    json.getLong("op"),
                    json.optJSONObject("record")?.let(::parseCoreRecord),
                    parseCoreState(json.getJSONObject("state")),
                )
            }

            "conflict", "id_reused", "refused_unavailable", "rejected", "invalid_input", NATIVE_CAPACITY_EXCEEDED -> {
                CoreReply.Refused(status, json.optString("detail"))
            }

            "protocol_skew" -> {
                CoreReply.ProtocolSkew
            }

            "unreachable" -> {
                CoreReply.Unreachable
            }

            // "transport" (submitted, reply lost) and "unavailable" (client failed) are uncertain.
            else -> {
                CoreReply.Uncertain
            }
        }
    } catch (_: Exception) {
        CoreReply.Uncertain
    }

private fun parseCoreState(json: JSONObject): CoreStateView {
    val canonical =
        when (json.getString("config_status")) {
            "readable" -> parseCanonicalConfig(json.getString("config"))?.let(RootCanonicalRead::Available) ?: RootCanonicalRead.Invalid
            "missing" -> RootCanonicalRead.Missing
            else -> RootCanonicalRead.Unavailable
        }
    val inFlight = json.optJSONArray("operations_in_flight")
    return CoreStateView(
        epoch = json.getLong("epoch"),
        configRevision = json.getLong("config_revision").also { require(it >= 0) },
        canonical = canonical,
        operationsInFlight = List(inFlight?.length() ?: 0) { inFlight!!.getLong(it) },
        debugLease = json.optBoolean("debug_lease"),
    )
}

private fun parseCoreRecord(json: JSONObject): CoreOperationRecord {
    val phases = json.getJSONArray("phases")
    return CoreOperationRecord(
        op = json.getLong("op"),
        phases =
            buildMap {
                for (i in 0 until phases.length()) {
                    val entry = phases.getJSONObject(i)
                    put(corePhase(entry.getString("phase")), coreOutcome(entry.getString("outcome")))
                }
            },
        settled = if (json.isNull("settled")) null else coreOutcome(json.getString("settled")),
    )
}

private fun corePhase(raw: String): ConfigPhase = ConfigPhase.entries.first { it.name == raw }

private fun coreOutcome(raw: String): PhaseOutcome = PhaseOutcome.entries.first { it.name == raw }
