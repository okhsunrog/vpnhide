package dev.okhsunrog.vpnhide

import java.util.UUID

/** Sequential effect adapter. The coordinator, not a screen/caller, owns its lifetime. */
internal class ConfigRootIo(
    private val prepare: () -> RootMutationClient?,
) : ConfigCoordinatorIo {
    private var transport: RootMutationClient? = null
    private var session: RootMutationSession? = null
    private var sequence = 0L
    private var operationId: Long? = null
    private var phase: ConfigPhase? = null
    private var base: CanonicalConfig? = null
    private var candidate: CanonicalConfig? = null

    override suspend fun initialize(): ConfigInitialization {
        val client = transport ?: prepare()?.also { transport = it } ?: return ConfigInitialization(ConfigCoordinatorMode.Unavailable)
        // One privileged round trip: the helper inspects and opens under its lock, and a
        // rejected adoption still returns the receipt this policy is decided from.
        val id = UUID.randomUUID().toString()
        val adopted = client.adopt(id) as? RootMutationReply.Observed ?: return ConfigInitialization(ConfigCoordinatorMode.Unavailable)
        val snapshot = adopted.snapshot
        if (!adopted.accepted) {
            val config = (snapshot.canonical as? RootCanonicalRead.Available)?.config
            return when {
                !snapshot.quiescent -> {
                    ConfigInitialization(ConfigCoordinatorMode.Paused, config)
                }

                // First adoption cannot prove that commands from the old, untracked transport stopped in this boot.
                snapshot.receipt.session == null && snapshot.receipt.boot == snapshot.boot -> {
                    ConfigInitialization(ConfigCoordinatorMode.RebootRequired, config)
                }

                else -> {
                    ConfigInitialization(ConfigCoordinatorMode.Unavailable)
                }
            }
        }
        session = openedRootMutationSession(adopted, snapshot.boot, id) ?: return ConfigInitialization(ConfigCoordinatorMode.Unavailable)
        sequence = snapshot.receipt.sequence
        return when (val read = snapshot.canonical) {
            is RootCanonicalRead.Available -> ConfigInitialization(ConfigCoordinatorMode.Open, read.config)
            RootCanonicalRead.Missing -> ConfigInitialization(ConfigCoordinatorMode.Missing)
            RootCanonicalRead.Invalid -> ConfigInitialization(ConfigCoordinatorMode.Invalid)
            RootCanonicalRead.Unavailable -> ConfigInitialization(ConfigCoordinatorMode.Unavailable)
        }
    }

    override suspend fun read(): RootCanonicalRead {
        val reply = transport?.inspect() as? RootMutationReply.Observed ?: return RootCanonicalRead.Unavailable
        return if (reply.accepted && reply.snapshot.quiescent && reply.snapshot.boot == session?.boot &&
            reply.snapshot.receipt.boot == session?.boot && reply.snapshot.receipt.session == session?.id
        ) {
            reply.snapshot.canonical
        } else {
            RootCanonicalRead.Unavailable
        }
    }

    override suspend fun execute(
        ticket: EffectTicket,
        phase: ConfigPhase,
        base: CanonicalConfig?,
        candidate: CanonicalConfig,
        command: String,
    ): ConfigPhaseEvidence {
        this.phase = phase
        this.operationId = ticket.owner
        this.base = base
        this.candidate = candidate
        sequence += 1
        return evidence(requireNotNull(transport).execute(requireNotNull(session), sequence, command))
    }

    override suspend fun recover(
        operationId: Long,
        phase: ConfigPhase,
    ): ConfigPhaseEvidence =
        if (this.operationId == operationId && this.phase == phase) {
            evidence(requireNotNull(transport).recover(requireNotNull(session), sequence))
        } else {
            ConfigPhaseEvidence(PhaseOutcome.Unknown, RootCanonicalRead.Unavailable)
        }

    private fun evidence(reply: RootMutationReply): ConfigPhaseEvidence {
        val snapshot = (reply as? RootMutationReply.Observed)?.snapshot
        val outcome =
            rootMutationPhaseOutcome(reply, requireNotNull(session), sequence, requireNotNull(phase), base, requireNotNull(candidate))
        val capacity = snapshot?.receipt?.nativeCapacity.takeIf { phase == ConfigPhase.Native && outcome != PhaseOutcome.Unknown }
        return ConfigPhaseEvidence(outcome, snapshot?.canonical ?: RootCanonicalRead.Unavailable, capacity)
    }
}
