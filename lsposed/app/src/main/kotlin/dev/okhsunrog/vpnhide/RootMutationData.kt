package dev.okhsunrog.vpnhide

internal enum class RootReceiptStatus { Idle, Running, Finished, NotStarted }

internal data class RootMutationSession(
    val boot: String,
    val id: String,
)

internal data class RootMutationReceipt(
    val revision: Long,
    val boot: String,
    val session: String?,
    val sequence: Long,
    val status: RootReceiptStatus,
    val exitCode: Int?,
    val descendantFailed: Boolean,
)

internal sealed interface RootCanonicalRead {
    data class Available(
        val config: CanonicalConfig,
    ) : RootCanonicalRead

    data object Missing : RootCanonicalRead

    data object Invalid : RootCanonicalRead

    data object Unavailable : RootCanonicalRead
}

internal data class RootMutationSnapshot(
    val boot: String,
    val receipt: RootMutationReceipt,
    val canonical: RootCanonicalRead,
) {
    val quiescent: Boolean get() = receipt.boot != boot || receipt.status != RootReceiptStatus.Running
}

internal sealed interface RootMutationReply {
    data class Observed(
        val accepted: Boolean,
        val snapshot: RootMutationSnapshot,
    ) : RootMutationReply

    data object Busy : RootMutationReply

    data object Unavailable : RootMutationReply
}

/** An exit code alone cannot decide whether an atomic replacement occurred. */
internal fun rootMutationPhaseOutcome(
    reply: RootMutationReply,
    session: RootMutationSession,
    sequence: Long,
    phase: ConfigPhase,
    base: CanonicalConfig,
    candidate: CanonicalConfig,
): PhaseOutcome {
    val observation = reply as? RootMutationReply.Observed ?: return PhaseOutcome.Unknown
    val snapshot = observation.snapshot
    val receipt = snapshot.receipt
    if (!observation.accepted || snapshot.boot != session.boot || receipt.boot != session.boot ||
        receipt.session != session.id || receipt.sequence != sequence || !snapshot.quiescent
    ) {
        return PhaseOutcome.Unknown
    }
    if (receipt.status == RootReceiptStatus.NotStarted) return PhaseOutcome.FailedKnown
    if (receipt.status != RootReceiptStatus.Finished) return PhaseOutcome.Unknown
    if (phase != ConfigPhase.Persist) {
        return if (receipt.exitCode == 0 && !receipt.descendantFailed) PhaseOutcome.Confirmed else PhaseOutcome.FailedKnown
    }
    val actual = (snapshot.canonical as? RootCanonicalRead.Available)?.config ?: return PhaseOutcome.Unknown
    return when (actual) {
        candidate -> PhaseOutcome.Confirmed
        base -> PhaseOutcome.FailedKnown
        else -> PhaseOutcome.Unknown
    }
}
