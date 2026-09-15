package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.picker.NativeTargetCapacityWarning

internal enum class ConfigCoordinatorMode { Initializing, Open, Missing, Invalid, Unavailable, Paused, RebootRequired }

internal data class ConfigCoordinatorView(
    val mode: ConfigCoordinatorMode = ConfigCoordinatorMode.Initializing,
    val confirmed: CanonicalConfig? = null,
    val pending: Map<CanonicalToggle, Boolean> = emptyMap(),
    val operations: Set<Long> = emptySet(),
    val lastResult: ConfigOperationResult? = null,
    val lastNativeCapacity: NativeTargetCapacityWarning? = null,
)

internal data class ConfigInitialization(
    val mode: ConfigCoordinatorMode,
    val config: CanonicalConfig? = null,
)

internal data class ConfigPhaseEvidence(
    val outcome: PhaseOutcome,
    val canonical: RootCanonicalRead,
    val nativeCapacity: NativeTargetCapacityWarning? = null,
) {
    val output: String get() = nativeCapacityOutput(nativeCapacity)
}

internal fun nativeCapacityOutput(warning: NativeTargetCapacityWarning?): String =
    warning?.let { "vpnhide-warning native_target_cap total=${it.total} cap=${it.capacity} dropped=${it.dropped}" }.orEmpty()

internal interface ConfigCoordinatorIo {
    suspend fun initialize(): ConfigInitialization

    suspend fun read(): RootCanonicalRead

    suspend fun execute(
        ticket: EffectTicket,
        phase: ConfigPhase,
        base: CanonicalConfig?,
        candidate: CanonicalConfig,
        command: String,
    ): ConfigPhaseEvidence

    suspend fun recover(
        operationId: Long,
        phase: ConfigPhase,
    ): ConfigPhaseEvidence
}

internal fun configMutationPlan(
    mutation: CanonicalMutation,
    changed: Boolean,
): List<ConfigPhase> =
    buildList {
        if (changed) add(ConfigPhase.Persist)
        if (mutation.coupledCommands.isNotEmpty()) add(ConfigPhase.Secret)
        if (changed || mutation.coupledCommands.isNotEmpty() || mutation.forceActivation) {
            if (mutation.activation.native) add(ConfigPhase.Native)
            if (mutation.activation.ports) add(ConfigPhase.Ports)
        }
    }

internal fun configPhaseCommand(
    phase: ConfigPhase,
    config: CanonicalConfig,
    mutation: CanonicalMutation,
): String =
    when (phase) {
        ConfigPhase.Persist -> buildCanonicalConfigWriteCommand(config)
        ConfigPhase.Secret -> mutation.coupledCommands.joinToString(" && ")
        ConfigPhase.Native -> ConfigChannels.nativeActivatorCommand()
        ConfigPhase.Ports -> ConfigChannels.portsActivatorCommand()
    }

internal fun pendingConfigToggles(mutations: Collection<CanonicalMutation>): Map<CanonicalToggle, Boolean> =
    buildMap {
        mutations.forEach { mutation ->
            mutation.edits.filterIsInstance<CanonicalEdit.Toggle>().forEach { put(it.field, it.enabled) }
        }
    }

internal fun prepareConfigMutation(
    ticket: EffectTicket,
    read: RootCanonicalRead,
    mutation: CanonicalMutation,
): ConfigOperationEvent {
    val base =
        (read as? RootCanonicalRead.Available)?.config
            ?: if (read == RootCanonicalRead.Missing &&
                mutation.bootstrap
            ) {
                CanonicalConfig()
            } else {
                return ConfigOperationEvent.PreparationFailed(ticket, TransitionFailure.ReadFailed)
            }
    return runCatching {
        val candidate = applyCanonicalMutation(base, mutation)
        // Readback must represent exactly the candidate, including normalized policies and disabled roles.
        require(parseCanonicalConfig(canonicalConfigJson(candidate)) == candidate)
        val plan = configMutationPlan(mutation, candidate != base || read == RootCanonicalRead.Missing)
        // Reject unframeable/oversized effects before even the persistence phase can be dispatched.
        plan.forEach { rootMutationScriptInput(configPhaseCommand(it, candidate, mutation)) }
        ConfigOperationEvent.Prepared(
            ticket,
            base,
            candidate,
            plan,
            canonicalEdits(base, candidate).flatMapTo(linkedSetOf(), ::canonicalEditFields),
            read is RootCanonicalRead.Available,
        )
    }.getOrElse { ConfigOperationEvent.PreparationFailed(ticket, TransitionFailure.ValidationFailed) }
}

internal fun configRecoveryEvidence(
    active: ActiveConfigOperation,
    evidence: ConfigPhaseEvidence,
    available: Boolean,
): ConfigRecoveryEvidence? {
    if (evidence.outcome !in setOf(PhaseOutcome.Confirmed, PhaseOutcome.FailedKnown)) return null
    val config =
        (evidence.canonical as? RootCanonicalRead.Available)?.config ?: if (
            !available && active.phase == ConfigPhase.Persist && evidence.outcome == PhaseOutcome.FailedKnown &&
            evidence.canonical == RootCanonicalRead.Missing
        ) {
            CanonicalConfig()
        } else {
            return null
        }
    return ConfigRecoveryEvidence(config, active.outcomes + (requireNotNull(active.phase) to evidence.outcome))
}
