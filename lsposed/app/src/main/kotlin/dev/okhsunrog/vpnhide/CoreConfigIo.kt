package dev.okhsunrog.vpnhide

import java.util.UUID

/**
 * The coordinator's effect adapter in 2.0: core is the only writer.
 *
 * - **Persist** submits `Replace(candidate)` against the revision of the coordinator's fresh
 *   read. Core persists, then applies Native and Ports itself inside the same operation and
 *   journals it; the app never writes the canonical config, `/proc/vpnhide_ctl`, Zygisk
 *   `targets.txt` or Ports rules.
 * - **Native / Ports** report core's outcome for that operation. When no core operation is
 *   current (a forced reconcile, or a coupled command ran since), they submit
 *   `RepairActivation` so core re-applies from its durable config.
 * - **Secret / Cleanup** are the only shell effects left (APatch superkey, pre-1.0 legacy
 *   files). They run on the receipt-tracked [RootCommandLane]; none touches a core-owned path.
 *
 * Recovery queries core for the same operation id — an uncertain submission is never
 * resubmitted as a new edit.
 */
internal class CoreConfigIo(
    private val prepareCore: () -> CoreConfigClient?,
    private val lane: RootCommandLane,
    private val newOperationId: () -> Long = CoreOperationIds::next,
) : ConfigCoordinatorIo {
    private var core: CoreConfigClient? = null
    private var revision = 0L

    /** The core operation the current Native/Ports phases report from, if still current. */
    private var current: CoreOperationRecord? = null

    /** The core op an uncertain Persist/Native/Ports dispatch may have started. */
    private var pending: Long? = null
    private var coupledOperation: Long? = null
    private var coupledPhase: ConfigPhase? = null

    override suspend fun initialize(): ConfigInitialization {
        val client = core ?: prepareCore()?.also { core = it } ?: return ConfigInitialization(ConfigCoordinatorMode.Unavailable)
        val state = (client.state() as? CoreReply.Observed)?.state ?: return ConfigInitialization(ConfigCoordinatorMode.Unavailable)
        revision = state.configRevision
        val config = (state.canonical as? RootCanonicalRead.Available)?.config
        // Core serializes its own operations; one still running (e.g. boot apply) pauses the lane.
        if (state.operationsInFlight.isNotEmpty()) return ConfigInitialization(ConfigCoordinatorMode.Paused, config)
        return when (state.canonical) {
            is RootCanonicalRead.Available -> ConfigInitialization(ConfigCoordinatorMode.Open, config)
            RootCanonicalRead.Missing -> ConfigInitialization(ConfigCoordinatorMode.Missing)
            RootCanonicalRead.Invalid -> ConfigInitialization(ConfigCoordinatorMode.Invalid)
            RootCanonicalRead.Unavailable -> ConfigInitialization(ConfigCoordinatorMode.Unavailable)
        }
    }

    override suspend fun read(): RootCanonicalRead {
        val state = (core?.state() as? CoreReply.Observed)?.state ?: return RootCanonicalRead.Unavailable
        // The base the next Replace is conflict-checked against: exactly what was just read.
        revision = state.configRevision
        current = null
        return state.canonical
    }

    override suspend fun execute(
        ticket: EffectTicket,
        phase: ConfigPhase,
        base: CanonicalConfig?,
        candidate: CanonicalConfig,
        command: String,
    ): ConfigPhaseEvidence {
        val client = core ?: return ConfigPhaseEvidence(PhaseOutcome.Unknown, RootCanonicalRead.Unavailable)
        return when (phase) {
            ConfigPhase.Persist -> {
                submit(phase) { op -> client.replace(op, revision, canonicalConfigJson(candidate)) }
            }

            ConfigPhase.Native, ConfigPhase.Ports -> {
                current?.let { record -> ConfigPhaseEvidence(applyOutcome(record, phase), RootCanonicalRead.Unavailable) }
                    ?: submit(phase) { op -> client.repair(op, revision) }
            }

            ConfigPhase.Secret, ConfigPhase.Cleanup -> {
                // A coupled effect (e.g. the superkey) may change what activation needs: the
                // following Native/Ports phases ask core to re-apply instead of reusing.
                current = null
                coupledOperation = ticket.owner
                coupledPhase = phase
                ConfigPhaseEvidence(lane.execute(command), RootCanonicalRead.Unavailable)
            }
        }
    }

    override suspend fun recover(
        operationId: Long,
        phase: ConfigPhase,
    ): ConfigPhaseEvidence {
        if (phase == ConfigPhase.Secret || phase == ConfigPhase.Cleanup) {
            return if (coupledOperation == operationId && coupledPhase == phase) {
                ConfigPhaseEvidence(lane.recover(), RootCanonicalRead.Unavailable)
            } else {
                ConfigPhaseEvidence(PhaseOutcome.Unknown, RootCanonicalRead.Unavailable)
            }
        }
        val op = pending ?: return ConfigPhaseEvidence(PhaseOutcome.Unknown, RootCanonicalRead.Unavailable)
        val reply =
            core?.operation(op) as? CoreReply.Observed ?: return ConfigPhaseEvidence(PhaseOutcome.Unknown, RootCanonicalRead.Unavailable)
        revision = reply.state.configRevision
        val record = reply.record ?: return ConfigPhaseEvidence(PhaseOutcome.Unknown, reply.state.canonical)
        if (record.settled == null) return ConfigPhaseEvidence(PhaseOutcome.Unknown, reply.state.canonical)
        pending = null
        current = record
        return ConfigPhaseEvidence(phaseOutcome(record, phase), reply.state.canonical)
    }

    private suspend fun submit(
        phase: ConfigPhase,
        call: suspend (Long) -> CoreReply,
    ): ConfigPhaseEvidence {
        val op = newOperationId()
        pending = op
        current = null
        return when (val reply = call(op)) {
            is CoreReply.Accepted -> {
                revision = reply.state.configRevision
                val record = reply.record?.takeIf { it.settled != null }
                if (record == null) {
                    // Admitted but still running past the client's wait: recover by op id.
                    ConfigPhaseEvidence(PhaseOutcome.Unknown, reply.state.canonical)
                } else {
                    pending = null
                    current = record
                    ConfigPhaseEvidence(phaseOutcome(record, phase), reply.state.canonical)
                }
            }

            // Refused before any phase ran: a known failure with nothing written.
            is CoreReply.Refused -> {
                pending = null
                ConfigPhaseEvidence(PhaseOutcome.FailedKnown, RootCanonicalRead.Unavailable)
            }

            CoreReply.ProtocolSkew, CoreReply.Unreachable -> {
                pending = null
                ConfigPhaseEvidence(PhaseOutcome.FailedKnown, RootCanonicalRead.Unavailable)
            }

            is CoreReply.Observed, CoreReply.Uncertain -> {
                ConfigPhaseEvidence(PhaseOutcome.Unknown, RootCanonicalRead.Unavailable)
            }
        }
    }

    private fun phaseOutcome(
        record: CoreOperationRecord,
        phase: ConfigPhase,
    ): PhaseOutcome =
        if (phase == ConfigPhase.Persist) {
            // A no-op Replace settles Confirmed without a Persist write.
            record.phases[ConfigPhase.Persist] ?: settledOutcome(record)
        } else {
            applyOutcome(record, phase)
        }
}

/**
 * The Native/Ports outcome core reports. Core plans the phases itself: a phase it did not plan
 * (nothing to apply) settles with the operation; one it never reached after an earlier failure
 * is a known failure, not success.
 */
internal fun applyOutcome(
    record: CoreOperationRecord,
    phase: ConfigPhase,
): PhaseOutcome =
    when (val outcome = record.phases[phase]) {
        null -> settledOutcome(record)
        PhaseOutcome.NotAttempted -> if (record.settled == PhaseOutcome.Unknown) PhaseOutcome.Unknown else PhaseOutcome.FailedKnown
        PhaseOutcome.Running -> PhaseOutcome.Unknown
        else -> outcome
    }

private fun settledOutcome(record: CoreOperationRecord): PhaseOutcome =
    when (record.settled) {
        PhaseOutcome.Confirmed -> PhaseOutcome.Confirmed
        PhaseOutcome.FailedKnown -> PhaseOutcome.FailedKnown
        else -> PhaseOutcome.Unknown
    }

/**
 * Receipt-tracked root lane for the coupled shell effects that are not core's (APatch superkey,
 * pre-1.0 legacy file cleanup). Opened lazily on first use; outcomes come from the receipt's
 * exit status only — the lane never carries or confirms a config write.
 */
internal class RootCommandLane(
    private val prepare: () -> RootMutationClient?,
) {
    private var transport: RootMutationClient? = null
    private var session: RootMutationSession? = null
    private var sequence = 0L

    fun execute(command: String): PhaseOutcome {
        if (command.isEmpty()) return PhaseOutcome.FailedKnown
        val client = transport ?: prepare()?.also { transport = it } ?: return PhaseOutcome.FailedKnown
        val opened = session ?: open(client) ?: return PhaseOutcome.FailedKnown
        sequence += 1
        return outcome(client.execute(opened, sequence, command), opened)
    }

    fun recover(): PhaseOutcome {
        val client = transport ?: return PhaseOutcome.Unknown
        val opened = session ?: return PhaseOutcome.Unknown
        return outcome(client.recover(opened, sequence), opened)
    }

    private fun open(client: RootMutationClient): RootMutationSession? {
        val id = UUID.randomUUID().toString()
        val adopted = client.adopt(id) as? RootMutationReply.Observed ?: return null
        if (!adopted.accepted) return null
        return openedRootMutationSession(adopted, adopted.snapshot.boot, id)?.also {
            session = it
            sequence = adopted.snapshot.receipt.sequence
        }
    }

    private fun outcome(
        reply: RootMutationReply,
        opened: RootMutationSession,
    ): PhaseOutcome = rootMutationPhaseOutcome(reply, opened, sequence, ConfigPhase.Secret, null, CanonicalConfig())
}
