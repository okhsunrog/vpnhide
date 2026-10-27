package dev.okhsunrog.vpnhide

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
    val revision: Long = 0,
    val nativeBoot: NativeBootStatus? = null,
    val activeCaptures: Int = 0,
    val applyEvidence: List<CoreApplyIdentity> = emptyList(),
    val heldResources: Set<ConfigPhase> = emptySet(),
    val backend: CoreBackendView? = null,
    val routing: CoreRoutingView? = null,
    val mutationHeld: Boolean = false,
)

/** A core operation receipt: phase outcomes as core executed them, and the settled result. */
internal data class CoreOperationRecord(
    val op: Long,
    val phases: Map<ConfigPhase, PhaseOutcome>,
    val settled: PhaseOutcome?,
)

internal sealed interface CoreReply {
    /** `state` / `operation` answered; [record] is null for an id core never admitted. */
    data class Observed(
        val state: CoreStateView?,
        val record: CoreOperationRecord? = null,
    ) : CoreReply

    /** Core admitted the edit and owns it; [record] is its receipt once settled (or still running). */
    data class Accepted(
        val op: Long,
        val record: CoreOperationRecord?,
        val state: CoreStateView?,
    ) : CoreReply

    /** Core refused the command before any phase ran. */
    data class Refused(
        val reason: uniffi.vpnhide_core_ffi.CoreSubmitRefusal,
    ) : CoreReply {
        val nativeCapacity: NativeCapacityRefusal?
            get() = (reason as? uniffi.vpnhide_core_ffi.CoreSubmitRefusal.NativeCapacityExceeded)?.report?.toCapacity()
    }

    /** The app and core speak different IPC protocol versions: update the lagging one. */
    data object ProtocolSkew : CoreReply

    /** Core socket not reachable (module missing, disabled or not started). */
    data object Unreachable : CoreReply

    /** The call may or may not have reached core: query the same op id, never resubmit. */
    data object Uncertain : CoreReply
}

/** Fresh, non-zero app-action and debug-lease identities; durable command IDs are allocated by core. */
internal object CoreOperationIds {
    private val random = SecureRandom()

    fun next(): Long {
        while (true) {
            val id = random.nextLong() and Long.MAX_VALUE
            if (id in 1 until Long.MAX_VALUE) return id
        }
    }
}

/** Root-observed UID membership; absent/unknown evidence is never VPN-off. */
internal data class CoreRoutingView(
    val current: Boolean,
    val generation: Long,
    val uid: Int?,
    val state: uniffi.vpnhide_core_ffi.CoreUidVpnRouteState?,
    val interfaces: List<String>,
    val session: String? = null,
)

/** Effective debug is published by core, including recording leases and FullReset. */
internal fun corePublishedDebug(update: CoreSessionUpdate): Boolean? =
    if (!update.current) {
        null
    } else {
        when (val canonical = update.state?.canonical) {
            is RootCanonicalRead.Available -> canonical.config.debug
            RootCanonicalRead.Missing -> false
            else -> null
        }
    }

/** Delivery evidence is distinct from the canonical revision and actual measurements. */
internal data class CoreApplyIdentity(
    val phase: ConfigPhase,
    val desiredRevision: Long,
    val operationId: Long,
    val outcome: PhaseOutcome,
)

internal data class CoreBackendView(
    val current: Boolean,
    val selected: NativeBackendId? = null,
    val applied: NativeBackendId? = null,
    val active: NativeBackendId? = null,
    val installed: Set<NativeBackendId> = emptySet(),
)
