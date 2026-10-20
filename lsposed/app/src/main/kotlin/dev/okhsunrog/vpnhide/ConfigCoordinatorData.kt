package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.core.CoreRootEffect

internal enum class ConfigCoordinatorMode { Initializing, Open, Missing, Invalid, Unavailable, Paused }

internal data class ConfigCoordinatorView(
    val mode: ConfigCoordinatorMode = ConfigCoordinatorMode.Initializing,
    val confirmed: CanonicalConfig? = null,
    val pending: Map<CanonicalToggle, Boolean> = emptyMap(),
    val operations: Set<Long> = emptySet(),
    val lastResult: ConfigOperationResult? = null,
    /** Why [lastResult] was refused at Save, when its native target set did not fit. */
    val lastNativeCapacity: NativeCapacityRefusal? = null,
    val activeCaptures: Int = 0,
    val recoveredResult: ConfigOperationResult? = null,
    val rechecking: Boolean = false,
)

internal data class ConfigInitialization(
    val mode: ConfigCoordinatorMode,
    val config: CanonicalConfig? = null,
)

internal data class ConfigPhaseEvidence(
    val outcome: PhaseOutcome,
    val canonical: RootCanonicalRead,
    /** Core refused the edit at Save because its native target set does not fit. */
    val nativeCapacity: NativeCapacityRefusal? = null,
)

internal interface ConfigCoordinatorIo {
    suspend fun initialize(): ConfigInitialization

    suspend fun read(): RootCanonicalRead

    suspend fun execute(
        ticket: EffectTicket,
        phase: ConfigPhase,
        base: CanonicalConfig?,
        candidate: CanonicalConfig,
        effects: List<CoreRootEffect>,
    ): ConfigPhaseEvidence

    suspend fun recover(
        operationId: Long,
        phase: ConfigPhase,
    ): ConfigPhaseEvidence

    /**
     * Whether the last state read says core holds debug logging on for a logcat recording.
     * Candidates then keep `debug` true (core enforces it anyway), so confirmed == candidate.
     */
    val debugLease: Boolean
        get() = false
}

internal fun configMutationPlan(
    mutation: CanonicalMutation,
    changed: Boolean,
    loggingChanged: Boolean = false,
): List<ConfigPhase> =
    buildList {
        if (changed && !mutation.removesCanonical) add(ConfigPhase.Persist)
        if (mutation.coupledEffects.isNotEmpty()) {
            add(
                if (mutation.cleanup ||
                    mutation.removesCanonical
                ) {
                    ConfigPhase.Cleanup
                } else {
                    ConfigPhase.Secret
                },
            )
        }
        if (changed || mutation.coupledEffects.isNotEmpty() || mutation.forceActivation) {
            if (mutation.activation.native || loggingChanged) add(ConfigPhase.Native)
            if (mutation.activation.ports) add(ConfigPhase.Ports)
        }
    }

/** The root effects a phase carries: only Secret/Cleanup do; the rest are typed core edits (see CoreConfigIo). */
internal fun configPhaseEffects(
    phase: ConfigPhase,
    mutation: CanonicalMutation,
): List<CoreRootEffect> =
    when (phase) {
        ConfigPhase.Persist, ConfigPhase.Native, ConfigPhase.Ports -> emptyList()
        ConfigPhase.Secret, ConfigPhase.Cleanup -> mutation.coupledEffects
    }

/** The canonical JSON size core-ffi `config_replace` accepts (`config_relay::MAX_CONFIG`). */
internal const val MAX_CORE_CONFIG_BYTES = 512 * 1024

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
    captureEnabled: Boolean? = null,
): ConfigOperationEvent {
    val base =
        (read as? RootCanonicalRead.Available)?.config
            ?: if (read == RootCanonicalRead.Missing &&
                (mutation.bootstrap || mutation.removesCanonical)
            ) {
                CanonicalConfig()
            } else {
                return ConfigOperationEvent.PreparationFailed(ticket, TransitionFailure.ReadFailed)
            }
    return runCatching {
        val edited = applyCanonicalMutation(base, mutation)
        val candidate =
            if (captureEnabled == null ||
                mutation.removesCanonical
            ) {
                edited
            } else {
                edited.copy(debug = edited.debugSwitch || captureEnabled)
            }
        // Readback must represent exactly the candidate, including normalized policies and disabled roles.
        require(parseCanonicalConfig(canonicalConfigJson(candidate)) == candidate)
        val plan =
            configMutationPlan(
                mutation,
                candidate != base || read == RootCanonicalRead.Missing,
                loggingChanged = captureEnabled != null && candidate.debug != base.debug,
            )
        // Reject unframeable/oversized effects before even the persistence phase can be dispatched.
        require(canonicalConfigJson(candidate).toByteArray(Charsets.UTF_8).size <= MAX_CORE_CONFIG_BYTES)
        require(mutation.coupledEffects.all { it.valid })
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
            evidence.canonical == RootCanonicalRead.Missing &&
            (
                (!available && active.phase == ConfigPhase.Persist && evidence.outcome == PhaseOutcome.FailedKnown) ||
                    (active.request.spec.removesCanonical && active.phase == ConfigPhase.Cleanup)
            )
        ) {
            CanonicalConfig()
        } else {
            return null
        }
    return ConfigRecoveryEvidence(config, active.outcomes + (requireNotNull(active.phase) to evidence.outcome))
}

internal fun configCompletionEvidence(
    active: ActiveConfigOperation,
    evidence: ConfigPhaseEvidence,
): ConfigPhaseEvidence {
    if (!active.request.spec.removesCanonical || active.phase != ConfigPhase.Cleanup ||
        evidence.outcome == PhaseOutcome.Unknown
    ) {
        return evidence
    }
    val outcome =
        when (evidence.canonical) {
            RootCanonicalRead.Missing -> evidence.outcome
            is RootCanonicalRead.Available -> PhaseOutcome.FailedKnown
            else -> PhaseOutcome.Unknown
        }
    return evidence.copy(outcome = outcome)
}
