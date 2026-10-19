package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.core.CoreEffectOutcome
import dev.okhsunrog.vpnhide.core.CoreRoot
import dev.okhsunrog.vpnhide.core.CoreRootEffect

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
 * - **Secret / Cleanup** are typed core root effects (`root_effect`: APatch superkey, pre-1.0
 *   legacy files, full reset). They are idempotent, so an uncertain one is recovered by
 *   applying the same effects again. The app runs no shell and no root process.
 *
 * Recovery queries core for the same operation id — an uncertain submission is never
 * resubmitted as a new edit.
 */
internal class CoreConfigIo(
    private val prepareCore: () -> CoreConfigClient?,
    private val rootEffects: CoreRootEffects = CoreRootEffects { op, effect -> CoreRoot.apply(op, effect) },
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
    private var coupledEffects: List<CoreRootEffect> = emptyList()

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
        effects: List<CoreRootEffect>,
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
                coupledEffects = effects
                ConfigPhaseEvidence(applyEffects(effects), RootCanonicalRead.Unavailable)
            }
        }
    }

    override suspend fun recover(
        operationId: Long,
        phase: ConfigPhase,
    ): ConfigPhaseEvidence {
        if (phase == ConfigPhase.Secret || phase == ConfigPhase.Cleanup) {
            return if (coupledOperation == operationId && coupledPhase == phase) {
                ConfigPhaseEvidence(applyEffects(coupledEffects), RootCanonicalRead.Unavailable)
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

    /** All effects in order; the first that does not apply decides the outcome. */
    private suspend fun applyEffects(effects: List<CoreRootEffect>): PhaseOutcome {
        if (effects.isEmpty()) return PhaseOutcome.FailedKnown
        for (effect in effects) {
            when (rootEffects.apply(newOperationId(), effect)) {
                CoreEffectOutcome.Ok -> {}

                CoreEffectOutcome.FailedKnown -> {
                    return PhaseOutcome.FailedKnown
                }

                CoreEffectOutcome.Unknown -> {
                    return PhaseOutcome.Unknown
                }
            }
        }
        return PhaseOutcome.Confirmed
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

/** Core's `root_effect`, as a seam for tests. */
internal fun interface CoreRootEffects {
    suspend fun apply(
        op: Long,
        effect: CoreRootEffect,
    ): CoreEffectOutcome
}
