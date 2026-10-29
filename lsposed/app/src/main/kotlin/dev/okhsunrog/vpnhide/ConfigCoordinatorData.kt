package dev.okhsunrog.vpnhide

import uniffi.vpnhide_core_ffi.CoreRecoveryAccess

internal enum class ConfigCoordinatorMode { Initializing, Open, Missing, Invalid, Unavailable, Paused }

internal data class ConfigCoordinatorView(
    val mode: ConfigCoordinatorMode = ConfigCoordinatorMode.Initializing,
    val confirmed: CanonicalConfig? = null,
    val pending: Map<CanonicalToggle, Boolean> = emptyMap(),
    val operations: Set<Long> = emptySet(),
    val lastResult: ConfigOperationResult? = null,
    val lastNativeCapacity: NativeCapacityRefusal? = null,
    val activeCaptures: Int = 0,
    val desiredRevision: Long? = null,
    val nativeObservationCurrent: Boolean = false,
    val coreOperations: Set<Long> = emptySet(),
    val applyEvidence: List<CoreApplyIdentity> = emptyList(),
    val recovery: CoreRecoveryAccess? = null,
    val startupError: String? = null,
    val storageFault: String? = null,
    val heldResources: Set<ConfigPhase> = emptySet(),
    val canRepair: Boolean = false,
    val coreCurrent: Boolean = false,
    val coreEpoch: Long? = null,
    val coreSequence: Long = 0,
    val recoveredResult: ConfigOperationResult? = null,
    val rechecking: Boolean = false,
)

internal const val MAX_CORE_CONFIG_BYTES = 512 * 1024

internal data class PreparedConfigIntent(
    val candidate: CanonicalConfig,
    val edits: List<CanonicalEdit>,
    val writes: Set<ConfigField>,
)

internal sealed interface ConfigIntentPreparation {
    data class Prepared(
        val intent: PreparedConfigIntent,
    ) : ConfigIntentPreparation

    data class Failed(
        val reason: TransitionFailure,
    ) : ConfigIntentPreparation
}

/** Resolve app-owned transforms into field intents; core alone plans execution. */
internal fun prepareConfigIntent(
    read: RootCanonicalRead,
    mutation: CanonicalMutation,
): ConfigIntentPreparation {
    val base =
        (read as? RootCanonicalRead.Available)?.config
            ?: if (read == RootCanonicalRead.Missing &&
                (mutation.bootstrap || mutation.removesCanonical || mutation.forceActivation || mutation.cleanup)
            ) {
                CanonicalConfig()
            } else {
                return ConfigIntentPreparation.Failed(TransitionFailure.ReadFailed)
            }
    return runCatching {
        val candidate = applyCanonicalMutation(base, mutation)
        require(parseCanonicalConfig(canonicalConfigJson(candidate)) == candidate)
        require(canonicalConfigJson(candidate).toByteArray(Charsets.UTF_8).size <= MAX_CORE_CONFIG_BYTES)
        require(mutation.coupledEffects.all { it.valid })
        val edits = configIntentEdits(base, candidate, mutation, read == RootCanonicalRead.Missing)
        ConfigIntentPreparation.Prepared(PreparedConfigIntent(candidate, edits, mutation.writes + edits.flatMap(::canonicalEditFields)))
    }.getOrElse { ConfigIntentPreparation.Failed(TransitionFailure.ValidationFailed) }
}

private fun configIntentEdits(
    base: CanonicalConfig,
    candidate: CanonicalConfig,
    mutation: CanonicalMutation,
    missing: Boolean,
): List<CanonicalEdit> {
    if (mutation.removesCanonical) return emptyList()
    if (missing || mutation.edits.any { it is CanonicalEdit.Replace }) return listOf(CanonicalEdit.Replace(candidate))
    val changed = canonicalEdits(base, candidate)
    // Same-value setters still declare intent and must participate in core's conflict check.
    val changedFields = changed.flatMapTo(linkedSetOf(), ::canonicalEditFields)
    return changed +
        mutation.edits
            .flatMap { finalIntentEdits(it, candidate) }
            .filter { canonicalEditFields(it).none { field -> field in changedFields } }
}

/** Policies may override a requested value (notably the app's own protected roles). */
private fun finalIntentEdits(
    edit: CanonicalEdit,
    candidate: CanonicalConfig,
): List<CanonicalEdit> =
    when (edit) {
        is CanonicalEdit.Toggle -> {
            listOf(edit.copy(enabled = canonicalToggle(candidate, edit.field)))
        }

        is CanonicalEdit.SettingSet -> {
            listOf(edit.copy(values = canonicalSet(candidate, edit.field)))
        }

        is CanonicalEdit.App -> {
            listOf(edit.copy(value = candidate.apps[edit.pkg] ?: CanonicalApp()))
        }

        is CanonicalEdit.RemoveApp -> {
            candidate.apps[edit.pkg]?.let { app ->
                CanonicalAppField.entries.map { field -> CanonicalEdit.App(edit.pkg, field, app) }
            } ?: listOf(edit)
        }

        is CanonicalEdit.Replace -> {
            listOf(CanonicalEdit.Replace(candidate))
        }
    }

internal fun pendingConfigToggles(mutations: Collection<CanonicalMutation>): Map<CanonicalToggle, Boolean> =
    buildMap {
        mutations.forEach { mutation -> mutation.edits.filterIsInstance<CanonicalEdit.Toggle>().forEach { put(it.field, it.enabled) } }
    }

internal fun coreRecordFailure(record: CoreOperationRecord): TransitionFailure? =
    when (record.settled) {
        PhaseOutcome.Confirmed -> null
        PhaseOutcome.FailedKnown -> TransitionFailure.ExecutionFailed
        else -> TransitionFailure.ApplicationUnknown
    }
